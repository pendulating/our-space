//! Bake a single real service day of ACE bus trips from the MTA GTFS *schedule*
//! (not just geometry): each trip becomes a time→arc-length keyframe path along
//! its shape, so the app can replay the real timetable minute-by-minute.
//!
//! Inputs: a GTFS static dir (calendar, calendar_dates, routes, trips, stop_times,
//! shapes, stops), the ki2b-sg5y ACE route list, and a `YYYYMMDD` service date.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use sim_core::assets::{BusDayLayer, BusTrip, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

#[derive(serde::Deserialize)]
struct AceRow {
    route: String,
    program: String,
}

/// Day-of-week for a `YYYYMMDD` date: 0 = Sunday … 6 = Saturday (Sakamoto).
fn weekday(ymd: u32) -> usize {
    let (y, m, d) = ((ymd / 10000) as i64, ((ymd / 100) % 100) as i64, (ymd % 100) as i64);
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    (((y + y / 4 - y / 100 + y / 400 + t[(m - 1) as usize] + d) % 7) as usize) % 7
}

/// Active GTFS `service_id`s on `date` (calendar weekday window ± calendar_dates
/// exceptions). exception_type 1 = added, 2 = removed.
fn active_services(gtfs_dir: &str, date: u32) -> Result<HashSet<String>> {
    // Monday-indexed column order in calendar.txt is mon..sun; map our Sun..Sat.
    let dow = weekday(date); // 0=Sun
    let cal_col = [6usize, 0, 1, 2, 3, 4, 5][dow]; // Sun->idx6(sunday), Mon->idx0, ...
    let mut active: HashSet<String> = HashSet::new();

    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(format!("{gtfs_dir}/calendar.txt"))
        .with_context(|| format!("opening {gtfs_dir}/calendar.txt"))?;
    let h = rdr.headers()?.clone();
    let col = |n: &str| h.iter().position(|x| x == n);
    let (c_id, c_start, c_end) = (
        col("service_id").context("service_id")?,
        col("start_date").context("start_date")?,
        col("end_date").context("end_date")?,
    );
    // Weekday flag columns in spec order.
    let days = [
        col("monday").context("monday")?,
        col("tuesday").context("tuesday")?,
        col("wednesday").context("wednesday")?,
        col("thursday").context("thursday")?,
        col("friday").context("friday")?,
        col("saturday").context("saturday")?,
        col("sunday").context("sunday")?,
    ];
    for rec in rdr.records() {
        let rec = rec?;
        let start: u32 = rec.get(c_start).unwrap_or("0").parse().unwrap_or(0);
        let end: u32 = rec.get(c_end).unwrap_or("0").parse().unwrap_or(0);
        if date < start || date > end {
            continue;
        }
        if rec.get(days[cal_col]).unwrap_or("0") == "1" {
            active.insert(rec.get(c_id).unwrap_or("").to_string());
        }
    }

    // Exceptions.
    let cd_path = format!("{gtfs_dir}/calendar_dates.txt");
    if std::path::Path::new(&cd_path).exists() {
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(&cd_path)?;
        let h = rdr.headers()?.clone();
        let (c_id, c_date, c_ex) = (
            h.iter().position(|x| x == "service_id").context("cd service_id")?,
            h.iter().position(|x| x == "date").context("cd date")?,
            h.iter().position(|x| x == "exception_type").context("cd exception_type")?,
        );
        for rec in rdr.records() {
            let rec = rec?;
            if rec.get(c_date).unwrap_or("0").parse::<u32>().unwrap_or(0) != date {
                continue;
            }
            let sid = rec.get(c_id).unwrap_or("").to_string();
            match rec.get(c_ex).unwrap_or("") {
                "1" => {
                    active.insert(sid);
                }
                "2" => {
                    active.remove(&sid);
                }
                _ => {}
            }
        }
    }
    Ok(active)
}

/// "HH:MM:SS" → minutes since service midnight (HH may be ≥ 24).
fn parse_min(t: &str) -> Option<f32> {
    let mut it = t.split(':');
    let h: f32 = it.next()?.trim().parse().ok()?;
    let m: f32 = it.next()?.trim().parse().ok()?;
    let s: f32 = it.next().unwrap_or("0").trim().parse().unwrap_or(0.0);
    Some(h * 60.0 + m + s / 60.0)
}

/// Arc length (m) of the nearest point on `shape` (with prefix lengths `cum`) to `p`.
fn project_arc(p: [f64; 2], shape: &[[f64; 2]], cum: &[f64]) -> f64 {
    let mut best = (f64::MAX, 0.0);
    for i in 0..shape.len().saturating_sub(1) {
        let (a, b) = (shape[i], shape[i + 1]);
        let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
        let len2 = abx * abx + aby * aby;
        let t = if len2 > 0.0 {
            (((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (px, py) = (a[0] + t * abx, a[1] + t * aby);
        let d2 = (p[0] - px).powi(2) + (p[1] - py).powi(2);
        if d2 < best.0 {
            best = (d2, cum[i] + t * len2.sqrt());
        }
    }
    best.1
}

pub fn bake(
    gtfs_dir: &str,
    ace_json: &str,
    date: u32,
    out_path: &str,
    extent: crate::extent::Extent,
    boundary_geojson: Option<&str>,
) -> Result<usize> {
    // Optional borough clip: some ACE shapes (e.g. M60-SBS) run out to LGA, so a
    // bus would drive off the island. We clip each shape to its longest in-borough
    // run and keep only the in-boundary stops, so the trip terminates at the
    // boundary instead. Shape geometry + keyframe arc-lengths stay consistent.
    // Citywide bakes pass no boundary (the whole-NYC geometry is wanted).
    let boundary = boundary_geojson
        .map(crate::boundary::ManhattanBoundary::load)
        .transpose()?;

    // 1. ACE route bases → matched GTFS route_ids + short names. Manhattan extent
    // keeps only M… routes; NYC extent keeps all five boroughs.
    let ace_rows: Vec<AceRow> = serde_json::from_slice(
        &std::fs::read(ace_json).with_context(|| format!("reading {ace_json}"))?,
    )
    .context("parsing ACE JSON")?;
    let ace_bases: HashSet<String> = ace_rows
        .iter()
        .filter(|r| r.program.eq_ignore_ascii_case("ACE"))
        .filter_map(|r| extent.route_base(&r.route))
        .collect();
    anyhow::ensure!(!ace_bases.is_empty(), "no ACE routes for extent {}", extent.label());

    let mut route_id_to_short: HashMap<String, String> = HashMap::new();
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(format!("{gtfs_dir}/routes.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_id, c_short) = (
        h.iter().position(|x| x == "route_id").context("route_id")?,
        h.iter().position(|x| x == "route_short_name").context("route_short_name")?,
    );
    for rec in rdr.records() {
        let rec = rec?;
        let short = rec.get(c_short).unwrap_or("");
        if extent.route_base(short).is_some_and(|b| ace_bases.contains(&b)) {
            route_id_to_short.insert(rec.get(c_id).unwrap_or("").to_string(), short.to_string());
        }
    }

    // 2. Active services for the date.
    let services = active_services(gtfs_dir, date)?;

    // 3. trips.txt: ACE trips whose service is active → trip_id → (short, shape_id).
    let mut trip_short: HashMap<String, String> = HashMap::new();
    let mut trip_shape: HashMap<String, String> = HashMap::new();
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(format!("{gtfs_dir}/trips.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_rid, c_sid, c_tid, c_shp) = (
        h.iter().position(|x| x == "route_id").context("t route_id")?,
        h.iter().position(|x| x == "service_id").context("t service_id")?,
        h.iter().position(|x| x == "trip_id").context("t trip_id")?,
        h.iter().position(|x| x == "shape_id").context("t shape_id")?,
    );
    for rec in rdr.records() {
        let rec = rec?;
        let rid = rec.get(c_rid).unwrap_or("");
        let sid = rec.get(c_sid).unwrap_or("");
        if let Some(short) = route_id_to_short.get(rid) {
            if services.contains(sid) {
                let tid = rec.get(c_tid).unwrap_or("").to_string();
                trip_short.insert(tid.clone(), short.clone());
                trip_shape.insert(tid, rec.get(c_shp).unwrap_or("").to_string());
            }
        }
    }
    anyhow::ensure!(!trip_short.is_empty(), "no ACE trips on {date}");

    // 4. stops.txt → stop_id → (lat, lon).
    let mut stop_ll: HashMap<String, (f64, f64)> = HashMap::new();
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(format!("{gtfs_dir}/stops.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_sid, c_la, c_lo) = (
        h.iter().position(|x| x == "stop_id").context("stop_id")?,
        h.iter().position(|x| x == "stop_lat").context("stop_lat")?,
        h.iter().position(|x| x == "stop_lon").context("stop_lon")?,
    );
    for rec in rdr.records() {
        let rec = rec?;
        // GTFS feeds pad numeric fields with leading spaces (e.g. "  40.78"); f64
        // parse rejects leading whitespace, so trim first or every coord silents to 0.
        let la: f64 = rec.get(c_la).unwrap_or("0").trim().parse().unwrap_or(0.0);
        let lo: f64 = rec.get(c_lo).unwrap_or("0").trim().parse().unwrap_or(0.0);
        stop_ll.insert(rec.get(c_sid).unwrap_or("").to_string(), (la, lo));
    }

    // 5. stop_times.txt (stream once): per kept trip → [(seq, dep_min, stop_id)].
    let mut trip_stops: HashMap<String, Vec<(u32, f32, String)>> = HashMap::new();
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(format!("{gtfs_dir}/stop_times.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_tid, c_dep, c_stop, c_seq) = (
        h.iter().position(|x| x == "trip_id").context("st trip_id")?,
        h.iter().position(|x| x == "departure_time").context("st departure_time")?,
        h.iter().position(|x| x == "stop_id").context("st stop_id")?,
        h.iter().position(|x| x == "stop_sequence").context("st stop_sequence")?,
    );
    for rec in rdr.records() {
        let rec = rec?;
        let tid = rec.get(c_tid).unwrap_or("");
        if !trip_short.contains_key(tid) {
            continue;
        }
        let Some(dep) = parse_min(rec.get(c_dep).unwrap_or("")) else { continue };
        let seq: u32 = rec.get(c_seq).unwrap_or("0").trim().parse().unwrap_or(0);
        trip_stops
            .entry(tid.to_string())
            .or_default()
            .push((seq, dep, rec.get(c_stop).unwrap_or("").to_string()));
    }

    // 6. shapes.txt (only kept shapes) → ordered ENU + de-dup index.
    let kept_shapes: HashSet<String> =
        trip_shape.values().filter(|s| !s.is_empty()).cloned().collect();
    let proj = EnuProjection::default();
    let mut raw_shape: HashMap<String, Vec<(u32, f64, f64)>> = HashMap::new();
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(format!("{gtfs_dir}/shapes.txt"))?;
    let h = rdr.headers()?.clone();
    let (c_sid, c_la, c_lo, c_seq) = (
        h.iter().position(|x| x == "shape_id").context("shape_id")?,
        h.iter().position(|x| x == "shape_pt_lat").context("shape_pt_lat")?,
        h.iter().position(|x| x == "shape_pt_lon").context("shape_pt_lon")?,
        h.iter().position(|x| x == "shape_pt_sequence").context("shape_pt_sequence")?,
    );
    for rec in rdr.records() {
        let rec = rec?;
        let sid = rec.get(c_sid).unwrap_or("");
        if !kept_shapes.contains(sid) {
            continue;
        }
        // GTFS feeds pad numeric fields with leading spaces (e.g. "  40.78"); f64
        // parse rejects leading whitespace, so trim first or every coord silents to 0.
        let la: f64 = rec.get(c_la).unwrap_or("0").trim().parse().unwrap_or(0.0);
        let lo: f64 = rec.get(c_lo).unwrap_or("0").trim().parse().unwrap_or(0.0);
        let seq: u32 = rec.get(c_seq).unwrap_or("0").trim().parse().unwrap_or(0);
        raw_shape.entry(sid.to_string()).or_default().push((seq, la, lo));
    }

    // Build de-duplicated ENU shapes + cumulative arc lengths.
    let mut shape_idx: HashMap<String, u32> = HashMap::new();
    let mut shapes_enu: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut shapes_cum: Vec<Vec<f64>> = Vec::new();
    let mut shapes_out: Vec<Vec<[f32; 2]>> = Vec::new();
    for (sid, pts) in raw_shape.iter_mut() {
        pts.sort_by_key(|(s, _, _)| *s);
        let full: Vec<[f64; 2]> = pts.iter().map(|(_, la, lo)| {
            let p = proj.to_enu(*la, *lo);
            [p.x, p.y]
        }).collect();
        // Clip to the longest in-boundary run (no clip → the whole shape).
        let enu = match &boundary {
            Some(b) => b.longest_run(&full),
            None => full,
        };
        if enu.len() < 2 {
            continue;
        }
        let mut cum = vec![0.0f64; enu.len()];
        for i in 1..enu.len() {
            cum[i] = cum[i - 1] + ((enu[i][0] - enu[i - 1][0]).powi(2) + (enu[i][1] - enu[i - 1][1]).powi(2)).sqrt();
        }
        shape_idx.insert(sid.clone(), shapes_enu.len() as u32);
        shapes_out.push(enu.iter().map(|p| [p[0] as f32, p[1] as f32]).collect());
        shapes_enu.push(enu);
        shapes_cum.push(cum);
    }

    // 7. Per trip: project stops onto the shape → time/arc keyframes.
    let mut routes: Vec<String> = route_id_to_short.values().cloned().collect();
    routes.sort();
    routes.dedup();
    let route_idx: HashMap<&String, u16> =
        routes.iter().enumerate().map(|(i, r)| (r, i as u16)).collect();

    let mut trips: Vec<BusTrip> = Vec::new();
    for (tid, short) in &trip_short {
        let Some(shp_id) = trip_shape.get(tid) else { continue };
        let Some(&sidx) = shape_idx.get(shp_id) else { continue };
        let Some(stops) = trip_stops.get_mut(tid) else { continue };
        stops.sort_by_key(|(s, _, _)| *s);
        let shape = &shapes_enu[sidx as usize];
        let cum = &shapes_cum[sidx as usize];
        let mut keyframes: Vec<[f32; 2]> = Vec::with_capacity(stops.len());
        let mut last_arc = 0.0f64;
        for (_, dep, stop_id) in stops.iter() {
            let Some(&(la, lo)) = stop_ll.get(stop_id) else { continue };
            let p = proj.to_enu(la, lo);
            // Drop stops outside the boundary so the trip ends at the border; the
            // remaining stops project cleanly onto the clipped (in-borough) shape.
            if let Some(b) = &boundary {
                if !b.contains([p.x, p.y]) {
                    continue;
                }
            }
            let arc = project_arc([p.x, p.y], shape, cum).max(last_arc); // keep monotonic
            last_arc = arc;
            keyframes.push([*dep, arc as f32]);
        }
        if keyframes.len() < 2 {
            continue;
        }
        trips.push(BusTrip {
            route_idx: *route_idx.get(short).unwrap_or(&0),
            shape_idx: sidx,
            start_min: keyframes.first().unwrap()[0],
            end_min: keyframes.last().unwrap()[0],
            keyframes,
        });
    }
    trips.sort_by(|a, b| a.start_min.total_cmp(&b.start_min));
    anyhow::ensure!(!trips.is_empty(), "no timed ACE trips built");

    let layer = BusDayLayer {
        origin: GeoOrigin::MANHATTAN,
        service_date: date,
        routes: routes.clone(),
        shapes: shapes_out,
        trips,
        provenance: Provenance {
            source: "MTA GTFS static schedule (stop_times) + data.ny.gov ki2b-sg5y (ACE routes)".into(),
            url: "https://data.ny.gov/d/ki2b-sg5y".into(),
            license: "MTA / OPEN-NY Terms of Use".into(),
            as_of: format!("GTFS service {date}"),
            notes: format!("Real ACE bus timetable replay ({}).", routes.join(", ")),
        },
    };
    let (nt, ns) = (layer.trips.len(), layer.shapes.len());
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("bus day {date}: {nt} ACE trips, {ns} shapes, {} routes -> {out_path}", routes.len());
    Ok(nt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekday_known_dates() {
        assert_eq!(weekday(20260421), 2); // Tue
        assert_eq!(weekday(20260419), 0); // Sun
        assert_eq!(weekday(20260101), 4); // Thu
    }

    #[test]
    fn parse_min_after_midnight() {
        assert_eq!(parse_min("05:00:27"), Some(300.0 + 27.0 / 60.0));
        assert_eq!(parse_min("25:30:00"), Some(25.0 * 60.0 + 30.0));
    }
}
