//! `bake-subway` — MTA subway GTFS static → all-pairs station router asset (`.ossub`).
//!
//! Parses the unzipped feed in `data/snapshots/gtfs/subway/` (see `tools/fetch_gtfs.py`),
//! normalizes platforms to parent stations, filters to one service day + AM-peak window,
//! and hands the result to `sim_core::subway::build_subway_matrix` (the math lives there;
//! this module is file parsing). The NYCT feed includes the Staten Island Railway, but no
//! rail link crosses the harbor — so the Staten Island Ferry (St George <-> Whitehall,
//! free, DOT-operated) is grafted on as a real line from **its own GTFS feed**
//! (`data/snapshots/gtfs/siferry/`, NYC DOT; fetched by the same tool): real weekday
//! departures set its pooled frequency and real stop_times its ~25-min crossing. When
//! that feed is missing the bake falls back — loudly — to a parameterized 15-min/25-min
//! pseudo-route, because losing SI transit connectivity silently would be *more* wrong:
//! SI commuters demonstrably ride SIR + ferry. The boat and both terminals are
//! camera-equipped, so a ferry boarding counts like a train.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use sim_core::assets::Provenance;
use sim_core::subway::{
    build_subway_matrix, FeedTrip, SubwayBuildParams, SubwayFeed, SubwayStation,
};
use sim_core::GeoOrigin;

/// Fallback-only parameterization (real service: every 15 min peak, ~25-min crossing).
const FERRY_HEADWAY_S: f64 = 900.0;
const FERRY_RUN_S: f64 = 1500.0;
/// Whitehall Terminal sits between the 1's South Ferry (142) and the R/W's Whitehall
/// St (R27); the two stations are also a physical in-system transfer the feed omits.
const FERRY_SI: &str = "S31"; // St George (SIR terminal)
const FERRY_MANHATTAN: [&str; 2] = ["142", "R27"];

/// Parse the NYC DOT Staten Island Ferry GTFS into trips over the subway's station
/// indices (terminals map onto the adjacent stations: St George -> S31, Whitehall ->
/// 142). Weekday service ids are detected from calendar day flags, not by name.
fn parse_ferry(dir: &str, si: u32, manhattan: u32) -> Result<Vec<FeedTrip>> {
    let path = |t: &str| format!("{dir}/{t}");

    let mut weekday_services: Vec<String> = Vec::new();
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(path("calendar.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_sid, c_mon, c_fri) =
        (col(&h, "service_id")?, col(&h, "monday")?, col(&h, "friday")?);
    for rec in rdr.records() {
        let rec = rec?;
        if rec.get(c_mon).unwrap_or("") == "1" && rec.get(c_fri).unwrap_or("") == "1" {
            weekday_services.push(rec.get(c_sid).unwrap_or("").trim().to_string());
        }
    }
    if weekday_services.is_empty() {
        bail!("no weekday-flagged service in {dir}/calendar.txt");
    }

    // Terminal stop ids by name (ids are `stgeorge`/`whitehall` today; match names too).
    let mut terminal: HashMap<String, u32> = HashMap::new();
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(path("stops.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_id, c_name) = (col(&h, "stop_id")?, col(&h, "stop_name")?);
    for rec in rdr.records() {
        let rec = rec?;
        let id = rec.get(c_id).unwrap_or("").trim().to_string();
        let name = rec.get(c_name).unwrap_or("").to_ascii_lowercase();
        if id == "stgeorge" || name.contains("george") {
            terminal.insert(id, si);
        } else if id == "whitehall" || name.contains("whitehall") {
            terminal.insert(id, manhattan);
        }
    }
    if terminal.len() < 2 {
        bail!("could not identify both ferry terminals in {dir}/stops.txt");
    }

    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(path("trips.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_trip, c_service) = (col(&h, "trip_id")?, col(&h, "service_id")?);
    let mut keep: std::collections::HashSet<String> = std::collections::HashSet::new();
    for rec in rdr.records() {
        let rec = rec?;
        if weekday_services.iter().any(|s| s == rec.get(c_service).unwrap_or("").trim()) {
            keep.insert(rec.get(c_trip).unwrap_or("").trim().to_string());
        }
    }

    let mut rdr =
        csv::ReaderBuilder::new().flexible(true).from_path(path("stop_times.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_trip, c_stop, c_dep, c_seq) = (
        col(&h, "trip_id")?,
        col(&h, "stop_id")?,
        col(&h, "departure_time")?,
        col(&h, "stop_sequence")?,
    );
    let mut per_trip: HashMap<String, Vec<(u32, u32, f64)>> = HashMap::new();
    for rec in rdr.records() {
        let rec = rec?;
        let tid = rec.get(c_trip).unwrap_or("");
        if !keep.contains(tid) {
            continue;
        }
        let (Some(&station), Some(dep)) = (
            terminal.get(rec.get(c_stop).unwrap_or("").trim()),
            gtfs_time_s(rec.get(c_dep).unwrap_or("")),
        ) else {
            continue;
        };
        let seq: u32 = rec.get(c_seq).unwrap_or("").trim().parse().unwrap_or(u32::MAX);
        per_trip.entry(tid.to_string()).or_default().push((seq, station, dep));
    }

    let mut out = Vec::new();
    let mut tids: Vec<String> = per_trip.keys().cloned().collect();
    tids.sort(); // deterministic
    for tid in tids {
        let mut stops = per_trip.remove(&tid).unwrap();
        stops.sort_by_key(|&(seq, _, _)| seq);
        let stops: Vec<(u32, f64)> = stops.into_iter().map(|(_, s, d)| (s, d)).collect();
        if stops.len() >= 2 && stops[0].0 != stops[1].0 {
            let direction = u8::from(stops[0].0 != si);
            out.push(FeedTrip { route: "SIF".into(), direction, stops });
        }
    }
    Ok(out)
}

fn col(h: &csv::StringRecord, name: &str) -> Result<usize> {
    h.iter()
        .position(|c| c.trim_start_matches('\u{feff}') == name)
        .with_context(|| format!("GTFS table missing column {name}"))
}

/// "HH:MM:SS" (hours may exceed 24) → seconds since service-day midnight.
fn gtfs_time_s(s: &str) -> Option<f64> {
    let mut it = s.trim().split(':');
    let h: f64 = it.next()?.parse().ok()?;
    let m: f64 = it.next()?.parse().ok()?;
    let sec: f64 = it.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

pub fn bake(
    gtfs_dir: &str,
    out_path: &str,
    service_id: &str,
    window_h: (f64, f64),
    ferry_dir: &str,
) -> Result<()> {
    let path = |t: &str| format!("{gtfs_dir}/{t}");
    let params = SubwayBuildParams {
        window_start_s: window_h.0 * 3600.0,
        window_end_s: window_h.1 * 3600.0,
        ..SubwayBuildParams::default()
    };

    // --- stops.txt: parent stations become matrix rows; platforms map to parents. ----
    let mut stations: Vec<SubwayStation> = Vec::new();
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(path("stops.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_id, c_name, c_lat, c_lon, c_type, c_parent) = (
        col(&h, "stop_id")?,
        col(&h, "stop_name")?,
        col(&h, "stop_lat")?,
        col(&h, "stop_lon")?,
        col(&h, "location_type")?,
        col(&h, "parent_station")?,
    );
    let mut child_parent: Vec<(String, String)> = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        let id = rec.get(c_id).unwrap_or("").trim();
        if rec.get(c_type).unwrap_or("") == "1" {
            stations.push(SubwayStation {
                id: id.to_string(),
                name: rec.get(c_name).unwrap_or("").trim().to_string(),
                lat: rec.get(c_lat).unwrap_or("").trim().parse()?,
                lon: rec.get(c_lon).unwrap_or("").trim().parse()?,
            });
        } else if let Some(p) = rec.get(c_parent).filter(|p| !p.trim().is_empty()) {
            child_parent.push((id.to_string(), p.trim().to_string()));
        }
    }
    stations.sort_by(|a, b| a.id.cmp(&b.id)); // deterministic station indices
    let sta_ix: HashMap<&str, u32> =
        stations.iter().enumerate().map(|(i, s)| (s.id.as_str(), i as u32)).collect();
    let mut stop_station: HashMap<String, u32> = HashMap::new();
    for s in &stations {
        stop_station.insert(s.id.clone(), sta_ix[s.id.as_str()]);
    }
    for (child, parent) in child_parent {
        if let Some(&ix) = sta_ix.get(parent.as_str()) {
            stop_station.insert(child, ix);
        }
    }

    // --- trips.txt: keep the chosen service day. ---------------------------------
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(path("trips.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_route, c_trip, c_service, c_dir) = (
        col(&h, "route_id")?,
        col(&h, "trip_id")?,
        col(&h, "service_id")?,
        col(&h, "direction_id")?,
    );
    let mut trip_meta: HashMap<String, (String, u8)> = HashMap::new();
    let mut trip_order: Vec<String> = Vec::new(); // file order → deterministic build
    for rec in rdr.records() {
        let rec = rec?;
        if rec.get(c_service).unwrap_or("").trim() != service_id {
            continue;
        }
        let id = rec.get(c_trip).unwrap_or("").trim().to_string();
        let dir: u8 = rec.get(c_dir).unwrap_or("0").trim().parse().unwrap_or(0);
        trip_meta.insert(id.clone(), (rec.get(c_route).unwrap_or("").trim().to_string(), dir));
        trip_order.push(id);
    }
    if trip_meta.is_empty() {
        bail!("no trips with service_id={service_id} in {gtfs_dir}/trips.txt");
    }

    // --- stop_times.txt: group the service day's stop rows per trip. -------------
    let mut rdr =
        csv::ReaderBuilder::new().flexible(true).from_path(path("stop_times.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_trip, c_stop, c_dep, c_seq) = (
        col(&h, "trip_id")?,
        col(&h, "stop_id")?,
        col(&h, "departure_time")?,
        col(&h, "stop_sequence")?,
    );
    let mut trip_stops: HashMap<String, Vec<(u32, u32, f64)>> = HashMap::new(); // (seq, station, dep)
    for rec in rdr.records() {
        let rec = rec?;
        let tid = rec.get(c_trip).unwrap_or("");
        if !trip_meta.contains_key(tid) {
            continue;
        }
        let (Some(&station), Some(dep)) = (
            stop_station.get(rec.get(c_stop).unwrap_or("").trim()),
            gtfs_time_s(rec.get(c_dep).unwrap_or("")),
        ) else {
            continue;
        };
        let seq: u32 = rec.get(c_seq).unwrap_or("").trim().parse().unwrap_or(u32::MAX);
        trip_stops.entry(tid.to_string()).or_default().push((seq, station, dep));
    }

    let mut trips: Vec<FeedTrip> = Vec::new();
    for tid in &trip_order {
        let Some(mut stops) = trip_stops.remove(tid) else { continue };
        stops.sort_by_key(|&(seq, _, _)| seq);
        let mut path_stops: Vec<(u32, f64)> = Vec::with_capacity(stops.len());
        for (_, station, dep) in stops {
            if path_stops.last().map(|&(s, _)| s) != Some(station) {
                path_stops.push((station, dep));
            }
        }
        // Keep trips that touch the window at all; build_subway_matrix only counts
        // in-window departures for headways, but their leg run-times still inform medians.
        let touches = path_stops
            .iter()
            .any(|&(_, dep)| dep >= params.window_start_s && dep < params.window_end_s);
        if path_stops.len() >= 2 && touches {
            let (route, direction) = trip_meta[tid].clone();
            trips.push(FeedTrip { route, direction, stops: path_stops });
        }
    }

    // --- transfers.txt: same-station rows → boarding overhead; cross rows → walks. --
    let mut transfers: Vec<(u32, u32, f64)> = Vec::new();
    let mut rdr =
        csv::ReaderBuilder::new().flexible(true).from_path(path("transfers.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_from, c_to, c_time) = (
        col(&h, "from_stop_id")?,
        col(&h, "to_stop_id")?,
        col(&h, "min_transfer_time")?,
    );
    for rec in rdr.records() {
        let rec = rec?;
        let (Some(&a), Some(&b)) = (
            stop_station.get(rec.get(c_from).unwrap_or("").trim()),
            stop_station.get(rec.get(c_to).unwrap_or("").trim()),
        ) else {
            continue;
        };
        if let Ok(t) = rec.get(c_time).unwrap_or("").trim().parse::<f64>() {
            transfers.push((a, b, t));
        }
    }

    // --- Staten Island Ferry: real NYC DOT GTFS, with a loud parameterized fallback ---
    // (losing SI transit connectivity silently would be worse than the parameterization).
    let mut ferry_note = "no ferry (endpoint stations not found)".to_string();
    if let (Some(&si), Some(&sf), Some(&wh)) = (
        sta_ix.get(FERRY_SI),
        sta_ix.get(FERRY_MANHATTAN[0]),
        sta_ix.get(FERRY_MANHATTAN[1]),
    ) {
        let parsed = parse_ferry(ferry_dir, si, sf).and_then(|v| {
            anyhow::ensure!(!v.is_empty(), "feed parsed to 0 weekday trips");
            Ok(v)
        });
        match parsed {
            Ok(fts) => {
                ferry_note = format!(
                    "SI Ferry from NYC DOT GTFS ({} weekday trips; terminals -> S31/142); \
                     142<->R27 linked",
                    fts.len()
                );
                trips.extend(fts);
            }
            Err(e) => {
                eprintln!(
                    "  WARNING: SI Ferry GTFS unusable at {ferry_dir} ({e}) — synthesizing \
                     a {:.0}-min/{:.0}-min pseudo-route; run tools/fetch_gtfs.py",
                    FERRY_HEADWAY_S / 60.0,
                    FERRY_RUN_S / 60.0
                );
                let n_sail =
                    ((params.window_end_s - params.window_start_s) / FERRY_HEADWAY_S) as usize;
                for k in 0..n_sail {
                    let t0 = params.window_start_s + k as f64 * FERRY_HEADWAY_S;
                    for (dir, (from, to)) in [(0u8, (si, sf)), (1u8, (sf, si))] {
                        trips.push(FeedTrip {
                            route: "SIF".into(),
                            direction: dir,
                            stops: vec![(from, t0), (to, t0 + FERRY_RUN_S)],
                        });
                    }
                }
                ferry_note = format!(
                    "SI Ferry SYNTHESIZED (feed unusable): every {:.0} min, {:.0} min \
                     crossing; 142<->R27 linked",
                    FERRY_HEADWAY_S / 60.0,
                    FERRY_RUN_S / 60.0
                );
            }
        }
        if !transfers.iter().any(|&(a, b, _)| (a, b) == (sf, wh) || (a, b) == (wh, sf)) {
            transfers.push((sf, wh, 180.0));
        }
    }

    // --- feed vintage for provenance. ---------------------------------------------
    let as_of = std::fs::read_to_string(path("feed_info.txt"))
        .ok()
        .and_then(|s| s.lines().nth(1).map(|l| l.replace(',', " ").trim().to_string()))
        .unwrap_or_else(|| "unknown".into());

    let n_trips = trips.len();
    let feed = SubwayFeed { stations, trips, transfers };
    let provenance = Provenance {
        source: "MTA NYCT subway GTFS static (supplemented schedule; includes SIR)".into(),
        url: "https://rrgtfsfeeds.s3.amazonaws.com/gtfs_subway.zip".into(),
        license: "MTA developer data terms".into(),
        as_of,
        notes: format!(
            "service={service_id}, window {:.0}:00-{:.0}:00; headway-based (wait=headway/2, \
             cap {:.0}s), board overhead from same-station transfer rows, alight {:.0}s; {}",
            window_h.0, window_h.1, params.max_wait_s, params.alight_s, ferry_note
        ),
    };
    let mat = build_subway_matrix(&feed, &params, GeoOrigin::MANHATTAN, provenance);

    // Summary + named probes: cheap ground-truth anchors for the bake log.
    let n = mat.len();
    let reachable = (0..n)
        .flat_map(|i| (0..n).map(move |j| (i, j)))
        .filter(|&(i, j)| i != j && mat.itinerary(i, j).is_some())
        .count();
    println!(
        "subway matrix: {n} stations, {n_trips} trips ({service_id} {:.0}-{:.0}h), \
         {reachable}/{} pairs reachable",
        window_h.0,
        window_h.1,
        n * (n - 1)
    );
    let ix_of = |id: &str| mat.stations.iter().position(|s| s.id == id);
    for (a, b, label) in [
        ("127", "631", "Times Sq-42 St -> Grand Central-42 St"),
        ("H11", "127", "Far Rockaway-Mott Av -> Times Sq-42 St"),
        ("S31", "142", "St George -> South Ferry (ferry)"),
        ("S17", "127", "Great Kills -> Times Sq-42 St (SIR+ferry+1)"),
    ] {
        if let (Some(i), Some(j)) = (ix_of(a), ix_of(b)) {
            match mat.itinerary(i, j) {
                Some(it) => println!(
                    "  probe {label}: {:.1} min, {} boarding(s), {:.1} km line-haul",
                    it.time_s / 60.0,
                    it.boardings,
                    it.line_haul_m / 1000.0
                ),
                None => println!("  probe {label}: UNREACHABLE"),
            }
        }
    }
    std::fs::write(out_path, mat.to_bytes()?)?;
    let bytes = std::fs::metadata(out_path)?.len();
    println!("wrote {out_path} ({:.1} MB)", bytes as f64 / 1e6);
    Ok(())
}
