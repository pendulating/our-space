//! `batch` — native headless host for citywide exposure computation.
//!
//! `batch heatmap <out.postcard> [hour]` computes, for every street-graph edge,
//! the expected number of devices that would capture you per minute of presence
//! (fixed cameras covering it + ACE/dashcam/glasses field rates), via R-tree
//! spatial culling, and bakes a HeatmapLayer aligned to the graph's edge order.

use anyhow::{Context, Result};
use rstar::primitives::GeomWithData;
use rstar::RTree;

use sim_core::assets::{
    AceCorridorLayer, AlprReaderLayer, CctvCameraLayer, DashcamFieldLayer, EdgeData,
    FixedSensorLayer, GraphAsset, HeatmapLayer, Provenance,
};
use sim_core::{
    exposure_rates_per_minute, sensors_from_layer, AceConfig, FixedCameraDefaults, MobileScenario,
    SensorInstance, Vec2 as Enu,
};

// Canonical asset location is the app crate's `assets/` (Bevy's asset root);
// run batch from the workspace root.
const GRAPH_PATH: &str = "crates/app-interactive/assets/processed/graph_manhattan.osgraph";
const CAMERAS_PATH: &str = "crates/app-interactive/assets/processed/cameras_fixed.oscctv";
const ACE_PATH: &str = "crates/app-interactive/assets/processed/ace_corridors.osace";
const DASHCAM_PATH: &str = "crates/app-interactive/assets/processed/dashcam_field.osfield";
const ALPR_PATH: &str = "crates/app-interactive/assets/processed/alpr.osalpr";
const DOT_PATH: &str = "crates/app-interactive/assets/processed/dot_cameras.osdot";

// Citywide observed fixed-camera set for the block-group exposure batch (`bg-exposure`).
const CAMERAS_NYC: &str = "crates/app-interactive/assets/processed/cameras_fixed_nyc.oscctv";
const ALPR_NYC: &str = "crates/app-interactive/assets/processed/alpr.osalpr";
const DOT_NYC: &str = "crates/app-interactive/assets/processed/dot_cameras_nyc.osdot";
const ENFORCE_NYC: &str = "crates/app-interactive/assets/processed/enforcement.oscam";
/// Cross-source physical-camera grouping radius — matches the interactive app's
/// `FIXED_GROUP_RADIUS_M`, so the batch counts a multiply-attested camera once.
const FIXED_GROUP_RADIUS_M: f64 = 15.0;

/// Baked building footprints, one layer per borough — the occlusion fabric.
const FOOTPRINTS: [&str; 5] = [
    "crates/app-interactive/assets/processed/footprints.osbldg", // Manhattan
    "crates/app-interactive/assets/processed/footprints_bronx.osbldg",
    "crates/app-interactive/assets/processed/footprints_brooklyn.osbldg",
    "crates/app-interactive/assets/processed/footprints_queens.osbldg",
    "crates/app-interactive/assets/processed/footprints_statenisland.osbldg",
];

/// Build the citywide line-of-sight occlusion index from the baked footprints.
///
/// Returns an **empty** index (⇒ every sightline clear ⇒ exactly the pre-occlusion behaviour) when
/// `OURSPACE_OCCLUSION=0`, which is how the regression guard and the free-space/occluded comparison
/// are run from one binary.
fn load_occluders() -> Result<sim_core::OccluderIndex> {
    if std::env::var("OURSPACE_OCCLUSION").as_deref() == Ok("0") {
        eprintln!("  occlusion: DISABLED (OURSPACE_OCCLUSION=0) — free-space FOV");
        return Ok(sim_core::OccluderIndex::empty());
    }
    let mut layers = Vec::new();
    for p in FOOTPRINTS {
        match std::fs::read(p) {
            Ok(b) => match sim_core::assets::BuildingFootprints::from_bytes(&b) {
                Ok(l) => layers.push(l),
                Err(e) => eprintln!("  WARNING: {p} failed to decode ({e}) — occlusion incomplete"),
            },
            Err(_) => eprintln!("  WARNING: {p} MISSING — occlusion incomplete for that borough"),
        }
    }
    anyhow::ensure!(!layers.is_empty(), "no building footprints found; cannot occlude");
    let t = std::time::Instant::now();
    let idx = sim_core::OccluderIndex::from_footprints(&layers, sim_core::DEFAULT_CELL_M);
    eprintln!(
        "  occlusion: {} footprints → {} walls, indexed in {:.1}s",
        idx.n_polygons(),
        idx.n_walls(),
        t.elapsed().as_secs_f64()
    );
    Ok(idx)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("heatmap") => {
            let out = args
                .get(2)
                .context("usage: batch heatmap <out.postcard> [hour]")?;
            let hour: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(17.0);
            heatmap(out, hour)
        }
        Some("bg-exposure") => {
            const USAGE: &str =
                "usage: batch bg-exposure <graph.osgraph> <points.csv:id,lat,lon> <out.csv> [walk_min]";
            let graph = args.get(2).context(USAGE)?;
            let points = args.get(3).context(USAGE)?;
            let out = args.get(4).context(USAGE)?;
            let walk_min: f64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(10.0);
            bg_exposure(graph, points, out, walk_min)
        }
        Some("od-exposure") => {
            const USAGE: &str = "usage: batch od-exposure <drive.osgraph> <walk.osgraph> \
                <centroids.csv:id,lat,lon,pop> <od.csv:home_bg,work_bg,jobs,low_wage> \
                <out.csv> [walk_min] [top_k]";
            let graph = args.get(2).context(USAGE)?;
            let walk = args.get(3).context(USAGE)?;
            let cents = args.get(4).context(USAGE)?;
            let od = args.get(5).context(USAGE)?;
            let out = args.get(6).context(USAGE)?;
            let walk_min: f64 = args.get(7).and_then(|s| s.parse().ok()).unwrap_or(10.0);
            let top_k: usize = args.get(8).and_then(|s| s.parse().ok()).unwrap_or(25);
            od_exposure(graph, walk, cents, od, out, walk_min, top_k)
        }
        Some("od-exposure-modal") => {
            const USAGE: &str = "usage: batch od-exposure-modal <drive.osgraph> <walk.osgraph> \
                <centroids.csv> <od.csv> <acs.csv> <stations.csv> <out.csv> [walk_min] [top_k] \
                [subway.ossub]";
            let graph = args.get(2).context(USAGE)?;
            let walk = args.get(3).context(USAGE)?;
            let cents = args.get(4).context(USAGE)?;
            let od = args.get(5).context(USAGE)?;
            let acs = args.get(6).context(USAGE)?;
            let stations = args.get(7).context(USAGE)?;
            let out = args.get(8).context(USAGE)?;
            let walk_min: f64 = args.get(9).and_then(|s| s.parse().ok()).unwrap_or(10.0);
            let top_k: usize = args.get(10).and_then(|s| s.parse().ok()).unwrap_or(100);
            let subway = args.get(11).map(String::as_str);
            od_exposure_modal(graph, walk, cents, od, acs, stations, out, walk_min, top_k, subway)
        }
        Some("od-exposure-mnl") => {
            const USAGE: &str = "usage: batch od-exposure-mnl <drive.osgraph> <walk.osgraph> \
                <centroids.csv> <od.csv> <acs.csv> <stations.csv> <out.csv> [walk_min] [top_k] \
                [subway.ossub]";
            let graph = args.get(2).context(USAGE)?;
            let walk = args.get(3).context(USAGE)?;
            let cents = args.get(4).context(USAGE)?;
            let od = args.get(5).context(USAGE)?;
            let acs = args.get(6).context(USAGE)?;
            let stations = args.get(7).context(USAGE)?;
            let out = args.get(8).context(USAGE)?;
            let walk_min: f64 = args.get(9).and_then(|s| s.parse().ok()).unwrap_or(10.0);
            let top_k: usize = args.get(10).and_then(|s| s.parse().ok()).unwrap_or(100);
            let subway = args.get(11).map(String::as_str);
            od_exposure_mnl(graph, walk, cents, od, acs, stations, out, walk_min, top_k, subway)
        }
        Some("counterfactual") => {
            const USAGE: &str = "usage: batch counterfactual <walk.osgraph> \
                <centroids.csv> <od.csv> <out.csv> [walk_min]";
            let graph = args.get(2).context(USAGE)?;
            let cents = args.get(3).context(USAGE)?;
            let od = args.get(4).context(USAGE)?;
            let out = args.get(5).context(USAGE)?;
            let walk_min: f64 = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(10.0);
            counterfactual(graph, cents, od, out, walk_min)
        }
        Some("covariates") => {
            const USAGE: &str = "usage: batch covariates <walk.osgraph> <centroids.csv> \
                <od.csv> <stations.csv> <crime.csv> <out.csv> [walk_min] [311.csv]";
            let graph = args.get(2).context(USAGE)?;
            let cents = args.get(3).context(USAGE)?;
            let od = args.get(4).context(USAGE)?;
            let stations = args.get(5).context(USAGE)?;
            let crime = args.get(6).context(USAGE)?;
            let out = args.get(7).context(USAGE)?;
            let walk_min: f64 = args.get(8).and_then(|s| s.parse().ok()).unwrap_or(10.0);
            let req311 = args.get(9).map(String::as_str);
            covariates(graph, cents, od, stations, crime, req311, out, walk_min)
        }
        Some("exposure-table") => {
            const USAGE: &str = "usage: batch exposure-table <centroids.csv> <R_i.csv> \
                <A_i_drive.csv> <acs.csv> <out.csv> [A_i_modal.csv] [A_i_mnl.csv]";
            let cent = args.get(2).context(USAGE)?;
            let ri = args.get(3).context(USAGE)?;
            let ai = args.get(4).context(USAGE)?;
            let acs = args.get(5).context(USAGE)?;
            let out = args.get(6).context(USAGE)?;
            let modal = args.get(7).map(String::as_str);
            let mnl = args.get(8).map(String::as_str);
            exposure_table(cent, ri, ai, acs, out, modal, mnl)
        }
        Some("occlusion-probe") => occlusion_probe(
            args.get(2)
                .map(String::as_str)
                .unwrap_or("crates/app-interactive/assets/processed/graph_nyc.osgraph"),
        ),
        Some("graph-stats") => {
            const USAGE: &str = "usage: batch graph-stats <drive.osgraph> <walk.osgraph> [out.json]";
            let drive = args.get(2).context(USAGE)?;
            let walk = args.get(3).context(USAGE)?;
            let out = args.get(4).map(String::as_str);
            graph_stats(drive, walk, out)
        }
        Some("occlusion-audit") => {
            const USAGE: &str =
                "usage: batch occlusion-audit <graph.osgraph> <points.csv:id,lat,lon> <out.csv> [walk_min]";
            let graph = args.get(2).context(USAGE)?;
            let points = args.get(3).context(USAGE)?;
            let out = args.get(4).context(USAGE)?;
            let walk_min: f64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(10.0);
            occlusion_audit(graph, points, out, walk_min)
        }
        _ => {
            eprintln!(
                "usage: batch <heatmap|bg-exposure|od-exposure|exposure-table|occlusion-probe\
                 |occlusion-audit> …"
            );
            std::process::exit(2);
        }
    }
}

/// Phase-0 of the occlusion plan (docs/OCCLUSION_PLAN.md): **measure before changing anything.**
///
/// Emits the numbers that gate the design — above all the count of cameras whose apex sits *inside*
/// a footprint. Those are the trap: without host-building exclusion every ray from such a camera
/// crosses its own walls, so it is blind in all directions and silently contributes zero. `R_i`
/// would drop and it would *look like occlusion working*.
fn occlusion_probe(graph_path: &str) -> Result<()> {
    use sim_core::graph::StreetGraph;
    use sim_core::Vec2;

    let graph =
        StreetGraph::from_asset(GraphAsset::from_bytes(&read(graph_path)?).context("graph")?);
    let occ = load_occluders()?;
    let (sensors, _, _occ) = load_fixed_sensors()?;

    // ---- Trap 1: cameras inside a footprint -------------------------------------------------
    use rayon::prelude::*;
    let hosts: Vec<Option<u32>> = sensors
        .par_iter()
        .map(|s| occ.containing_polygon(s.wedge.apex))
        .collect();
    let inside = hosts.iter().filter(|h| h.is_some()).count();
    println!("\n=== TRAP 1: cameras geocoded INSIDE a building footprint ===");
    println!(
        "  {inside} / {} sensors ({:.2}%) — these would be BLIND without host exclusion",
        sensors.len(),
        100.0 * inside as f64 / sensors.len() as f64
    );
    let mut by_kind: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for (s, h) in sensors.iter().zip(&hosts) {
        let e = by_kind.entry(format!("{:?}", s.kind)).or_default();
        e.1 += 1;
        if h.is_some() {
            e.0 += 1;
        }
    }
    for (k, (i, n)) in &by_kind {
        println!("    {k:<20} {i:>6} / {n:<6} ({:.2}%)", 100.0 * *i as f64 / *n as f64);
    }

    // ---- Trap 3: walkshed sample points inside a footprint -----------------------------------
    let edges = &graph.asset().edges;
    let step = (edges.len() / 200_000).max(1);
    let (mut pts, mut pts_in) = (0usize, 0usize);
    for e in edges.iter().step_by(step) {
        for k in [0, e.polyline.len() / 2, e.polyline.len().saturating_sub(1)] {
            let p = e.polyline[k];
            pts += 1;
            if occ.containing_polygon(Vec2::new(p[0], p[1])).is_some() {
                pts_in += 1;
            }
        }
    }
    println!("\n=== TRAP 3: street-graph sample points inside a footprint ===");
    println!(
        "  {pts_in} / {pts} ({:.2}%) — arcades, tunnels, streets under buildings. Expected small.",
        100.0 * pts_in as f64 / pts.max(1) as f64
    );

    // ---- Sanity: how much of a camera's disc is interior to buildings? -----------------------
    // The smell test. Manhattan (near-100% lot coverage) must be MUCH higher than Staten Island
    // (detached houses). If that ordering fails, the geometry is wrong — stop and debug.
    println!("\n=== SANITY: fraction of a 15 m disc that is occluded, by borough ===");
    let boro_of = |p: Vec2| -> &'static str {
        // Cheap ENU-box borough proxy, good enough for a sanity check.
        let (x, y) = (p.x, p.y);
        if y > 8000.0 { "Bronx" } else if x < -6000.0 && y < -8000.0 { "StatenIsland" }
        else if y < -6000.0 { "Brooklyn" } else if x > 6000.0 { "Queens" } else { "Manhattan" }
    };
    let mut tally: std::collections::BTreeMap<&str, (f64, usize)> = Default::default();
    for (i, s) in sensors.iter().enumerate().step_by(7) {
        let apex = s.wedge.apex;
        let host = hosts[i];
        let (mut blocked, mut total) = (0usize, 0usize);
        for k in 0..36 {
            let th = k as f64 * std::f64::consts::TAU / 36.0;
            let t = Vec2::new(apex.x + 15.0 * th.cos(), apex.y + 15.0 * th.sin());
            total += 1;
            if occ.blocked(apex, t, host) {
                blocked += 1;
            }
        }
        let e = tally.entry(boro_of(apex)).or_default();
        e.0 += blocked as f64 / total as f64;
        e.1 += 1;
    }
    for (b, (sum, n)) in &tally {
        println!("  {b:<14} {:.1}% of rays blocked   (n={n} cameras sampled)", 100.0 * sum / *n as f64);
    }

    // ---- Why is the disc barely occluded? How far is the nearest wall? -----------------------
    // Hypothesis: the CCTV census geocodes to INTERSECTION CENTROIDS (Amnesty) and GSV panorama
    // points (Dahir) — i.e. the middle of the roadbed. With a 15 m range, such a camera may not
    // physically REACH the building line: a Manhattan avenue/street corner sits at
    // sqrt(15² + 9²) ≈ 17.5 m from the intersection centre. If so, the free-space model was never
    // over-counting much, and PLAN.md's "Manhattan LOS is wall-limited, ignoring it grossly
    // over-counts" was wrong about THIS model (though right about real cameras on real facades).
    let mut dists: Vec<f64> = Vec::new();
    for (i, s) in sensors.iter().enumerate().step_by(11) {
        let apex = s.wedge.apex;
        // Nearest distance at which a ray from the apex first hits a wall, over 72 bearings.
        let mut best = f64::MAX;
        for k in 0..72 {
            let th = k as f64 * std::f64::consts::TAU / 72.0;
            let dir = Vec2::new(th.cos(), th.sin());
            // Binary-search the first blocked radius along this bearing, out to 60 m.
            if !occ.blocked(apex, Vec2::new(apex.x + 60.0 * dir.x, apex.y + 60.0 * dir.y), hosts[i])
            {
                continue;
            }
            let (mut lo, mut hi) = (0.0f64, 60.0f64);
            for _ in 0..12 {
                let mid = 0.5 * (lo + hi);
                let t = Vec2::new(apex.x + mid * dir.x, apex.y + mid * dir.y);
                if occ.blocked(apex, t, hosts[i]) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            best = best.min(hi);
        }
        if best.is_finite() {
            dists.push(best);
        }
    }
    dists.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("\n=== DIAGNOSIS: distance from a camera apex to the NEAREST building wall ===");
    if !dists.is_empty() {
        let q = |f: f64| dists[((dists.len() - 1) as f64 * f) as usize];
        println!(
            "  p10 {:.1} m | p25 {:.1} m | MEDIAN {:.1} m | p75 {:.1} m | p90 {:.1} m   (n={})",
            q(0.10), q(0.25), q(0.50), q(0.75), q(0.90), dists.len()
        );
        let within = dists.iter().filter(|d| **d <= 15.0).count();
        println!(
            "  cameras with a wall INSIDE their 15 m range: {within}/{} ({:.0}%)",
            dists.len(),
            100.0 * within as f64 / dists.len() as f64
        );
        println!("  -> if that share is low, the modelled camera simply never reaches a building,");
        println!("     and occlusion cannot bite no matter how correct the geometry is.");
    }

    // ---- Query cost --------------------------------------------------------------------------
    let t = std::time::Instant::now();
    let mut hits = 0usize;
    let n_q = 200_000usize;
    for i in 0..n_q {
        let s = &sensors[i % sensors.len()];
        let a = s.wedge.apex;
        let th = (i as f64) * 0.7;
        let b = Vec2::new(a.x + 15.0 * th.cos(), a.y + 15.0 * th.sin());
        if occ.blocked(a, b, hosts[i % sensors.len()]) {
            hits += 1;
        }
    }
    let el = t.elapsed().as_secs_f64();
    let cands: usize = (0..500)
        .map(|i| {
            let a = sensors[i * 7 % sensors.len()].wedge.apex;
            occ.candidates_for(a, Vec2::new(a.x + 15.0, a.y + 15.0))
        })
        .sum::<usize>()
        / 500;
    println!("\n=== COST ===");
    println!(
        "  {n_q} sightline queries in {el:.2}s → {:.2} µs/query ({hits} blocked)",
        el / n_q as f64 * 1e6
    );
    println!("  mean walls tested per query: ~{cands}");
    Ok(())
}

/// Guaranteed MTA subway-system camera captures for a transit trip, as a function of the subway
/// line-haul distance. The MTA has cameras in every station and every subway car
/// (mta.info/document/178926), so a rider's underground line-haul is NOT invisible — it is
/// comprehensively surveilled. Distinct cameras on the rider's *path* =
///   `cams_station` · (2 endpoint stations + n transfer stations) + `cams_train` · (1 + n trains),
/// where n = estimated transfers = min(line_haul_km / km_per_transfer, max_transfers). We estimate
/// transfers from distance rather than routing the network (we only need entry/train/exit counts,
/// not the line itself). Central defaults (station 3, train 2, 12 km/transfer, cap 3) give a
/// pop-weighted mean ≈ 10–11 cameras/trip: a no-transfer trip = 3·2 + 2·1 = 8, a one-transfer
/// trip = 3·3 + 2·2 = 13. These MTA cameras are distinct from the street census, so they add to —
/// never double-count — the access/egress street-walk exposure. All knobs are env-overridable;
/// `OURSPACE_SUBWAY_SCALE` multiplies the whole result for sensitivity sweeps.
#[derive(Clone, Copy)]
struct SubwayParams {
    cams_station: f64,
    cams_train: f64,
    km_per_transfer: f64,
    max_transfers: f64,
    scale: f64,
}
impl SubwayParams {
    fn from_env() -> Self {
        let envf = |k: &str, d: f64| {
            std::env::var(k)
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0)
                .unwrap_or(d)
        };
        SubwayParams {
            cams_station: envf("OURSPACE_SUBWAY_CAMS_STATION", 3.0),
            cams_train: envf("OURSPACE_SUBWAY_CAMS_TRAIN", 2.0),
            km_per_transfer: envf("OURSPACE_SUBWAY_KM_PER_TRANSFER", 12.0).max(0.1),
            max_transfers: envf("OURSPACE_SUBWAY_MAX_TRANSFERS", 3.0),
            scale: envf("OURSPACE_SUBWAY_SCALE", 1.0),
        }
    }
    /// Guaranteed subway cameras for a trip whose origin→dest station straight-line is `line_haul_m`.
    fn cameras(&self, line_haul_m: f64) -> f64 {
        let n = (line_haul_m.max(0.0) / 1000.0 / self.km_per_transfer).min(self.max_transfers);
        self.scale * (self.cams_station * (2.0 + n) + self.cams_train * (1.0 + n))
    }
    /// Same station/train complement, but with the *routed* transfer count
    /// n = boardings − 1 (still capped) instead of the distance estimate. Used when a
    /// subway-graph matrix is loaded; a ferry boarding counts like a train (the boat and
    /// both terminals are camera-equipped).
    fn cameras_for_boardings(&self, boardings: u32) -> f64 {
        if boardings == 0 {
            return 0.0;
        }
        let n = ((boardings - 1) as f64).min(self.max_transfers);
        self.scale * (self.cams_station * (2.0 + n) + self.cams_train * (1.0 + n))
    }
    fn describe(&self) -> String {
        format!(
            "station×{:.1} + train×{:.1}, {:.0} km/transfer (cap {:.0}), scale {:.2}; \
             no-transfer={:.1}, +1-transfer={:.1}",
            self.cams_station, self.cams_train, self.km_per_transfer, self.max_transfers, self.scale,
            self.cameras(0.0), self.cameras(self.km_per_transfer * 1000.0),
        )
    }
}

/// The bus sub-mode of the transit alternative. ACS B08301 splits transit into bus vs
/// subway/rail per BG, and the two have qualitatively different exposure geometry: a bus
/// rides STREET-LEVEL (road-facing cameras along the corridor see the vehicle, like the
/// drive mode) and carries its own onboard cameras (every MTA bus is camera-equipped;
/// ACE corridors more so). v1 approximates the bus corridor with the drive route (same
/// origin/destination street network) and parameterizes service: in-vehicle time =
/// drive time × slowdown, plus a flat wait + stop-access allowance. All env-overridable
/// (`OURSPACE_BUS_*`); onboard complement swept linearly via the `commute_subway`
/// (transit-system cameras) column like the subway term.
#[derive(Clone, Copy)]
struct BusParams {
    cams_bus: f64,
    km_per_transfer: f64,
    max_transfers: f64,
    scale: f64,
    slowdown: f64,
    wait_s: f64,
    access_s: f64,
}
impl BusParams {
    fn from_env() -> Self {
        let envf = |k: &str, d: f64| {
            std::env::var(k)
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0)
                .unwrap_or(d)
        };
        BusParams {
            cams_bus: envf("OURSPACE_BUS_CAMS", 6.0),
            km_per_transfer: envf("OURSPACE_BUS_KM_PER_TRANSFER", 8.0).max(0.1),
            max_transfers: envf("OURSPACE_BUS_MAX_TRANSFERS", 2.0),
            scale: envf("OURSPACE_BUS_SCALE", 1.0),
            slowdown: envf("OURSPACE_BUS_SLOWDOWN", 2.0).max(1.0),
            wait_s: envf("OURSPACE_BUS_WAIT_S", 300.0),
            access_s: envf("OURSPACE_BUS_ACCESS_S", 240.0),
        }
    }
    /// Onboard bus cameras for a trip of `dist_m` (boardings = 1 + distance-scaled transfers).
    fn cameras(&self, dist_m: f64) -> f64 {
        let n = (dist_m.max(0.0) / 1000.0 / self.km_per_transfer).min(self.max_transfers);
        self.scale * self.cams_bus * (1.0 + n)
    }
    fn describe(&self) -> String {
        format!(
            "bus×{:.1} onboard, {:.0} km/transfer (cap {:.0}), slowdown ×{:.1}, wait {:.0}s",
            self.cams_bus, self.km_per_transfer, self.max_transfers, self.slowdown, self.wait_s
        )
    }
}

/// Crow-flies station-to-station distance understates the real subway line-haul (routes bend
/// through the trunk lines), which understates both the transfer-scaled camera count and the
/// in-vehicle time — and does so most for the cross-borough trips of outer-borough commuters.
/// Multiply the straight-line distance by this network-circuity factor (~1.3 is typical for
/// the NYC subway); a proper subway-graph router remains future work. Env-tunable, ≥ 1.
fn subway_circuity() -> f64 {
    std::env::var("OURSPACE_SUBWAY_CIRCUITY")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 1.0)
        .unwrap_or(1.3)
}

/// Fallback transit-time parameters (crow-flies model only; the subway-graph matrix
/// carries its own waits, run times, and transfer overheads).
const SUBWAY_MPS: f64 = 7.6; // NYC subway avg incl. dwell (~17 mph)
const TRANSIT_WAIT_S: f64 = 360.0; // initial wait + transfers

/// One candidate access station for a BG: station index (into the matrix/CSV order),
/// the access walk's exposure and length on the WALK graph, and the station ENU
/// (crow-flies fallback line-haul).
#[derive(Clone, Copy)]
struct StaCand {
    idx: usize,
    e: f64,
    d_m: f64,
    enu: Enu,
}

/// Walk-route a BG centroid to its `k` nearest stations. Unroutable candidates are
/// dropped (an empty result ⇒ transit infeasible for that BG) — claiming a transit trip
/// whose access walk cannot be walked would book station cameras from an impossible leg.
#[allow(clippy::too_many_arguments)]
fn station_candidates(
    enu: Enu,
    wnode: u32,
    k: usize,
    station_tree: &RTree<GeomWithData<[f64; 2], usize>>,
    station_enu: &[Enu],
    station_wnode: &[Option<u32>],
    walk: &sim_core::StreetGraph,
    sensors: &[SensorInstance],
    cam_tree: &RTree<GeomWithData<[f64; 2], usize>>,
    occ: &sim_core::OccluderIndex,
    recall: f64,
    fov: &sim_core::FovModel,
    // The BG's own walkshed groups: cameras already counted at this end of the trip.
    own_groups: &std::collections::HashSet<u32>,
) -> Vec<StaCand> {
    let empty = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(k);
    for n in station_tree.nearest_neighbor_iter(&[enu.x, enu.y]).take(k) {
        let idx = n.data;
        let senu = station_enu[idx];
        let Some(snode) = station_wnode[idx] else { continue };
        if snode == wnode {
            out.push(StaCand { idx, e: 0.0, d_m: 50.0, enu: senu }); // station at the centroid
        } else if let Ok((r, _, _)) = walk.route_timed_pen(wnode, snode, 1.0) {
            out.push(StaCand {
                idx,
                e: route_leg_exposure(&r.points, sensors, cam_tree, occ, recall, true, fov, (own_groups, &empty)).1,
                d_m: r.total_m,
                enu: senu,
            });
        }
    }
    out
}

/// A transit itinerary's mode-choice inputs: door-to-door minutes, street access+egress
/// walk exposure, and the guaranteed MTA station/train camera complement.
struct TransitLeg {
    t_min: f64,
    e_acc: f64,
    e_subway: f64,
}

/// Best transit leg between two candidate-station sets.
///
/// With a subway matrix: minimize (access walk + routed itinerary + egress walk) over the
/// candidate pairs; the itinerary already contains first wait, boarding overheads, rides,
/// and transfer walks, and its boarding count sets the camera complement. Zero-boarding
/// "trips" (walking a transfer corridor between adjacent complexes) are not transit.
/// Without a matrix: the legacy crow-flies × circuity parameterization over the single
/// nearest station pair.
fn best_transit_leg(
    submat: Option<&sim_core::SubwayMatrix>,
    subway: &SubwayParams,
    circuity: f64,
    walk_mps: f64,
    home: &[StaCand],
    work: &[StaCand],
) -> Option<TransitLeg> {
    match submat {
        Some(m) => {
            let mut best: Option<(f64, f64, f64)> = None; // (t_s, e_acc, e_subway)
            for a in home {
                for b in work {
                    if a.idx == b.idx {
                        continue;
                    }
                    let Some(it) = m.itinerary(a.idx, b.idx) else { continue };
                    if it.boardings == 0 {
                        continue;
                    }
                    let t = (a.d_m + b.d_m) / walk_mps + it.time_s;
                    if best.map_or(true, |(bt, _, _)| t < bt) {
                        best =
                            Some((t, a.e + b.e, subway.cameras_for_boardings(it.boardings)));
                    }
                }
            }
            let (t_s, e_acc, e_subway) = best?;
            Some(TransitLeg { t_min: t_s / 60.0, e_acc, e_subway })
        }
        None => {
            let (a, b) = (home.first()?, work.first()?);
            // Transit is a real alternative only if both station walks are routable and
            // the two nearest stations differ — a same-station "trip" rides nowhere yet
            // would book the full station+train camera complement.
            if a.d_m >= 1.0e5 || b.d_m >= 1.0e5 || a.enu.distance(b.enu) < 1.0 {
                return None;
            }
            let line_haul = a.enu.distance(b.enu) * circuity;
            Some(TransitLeg {
                t_min: (a.d_m / walk_mps + line_haul / SUBWAY_MPS + TRANSIT_WAIT_S
                    + b.d_m / walk_mps)
                    / 60.0,
                e_acc: a.e + b.e,
                e_subway: subway.cameras(line_haul),
            })
        }
    }
}

/// Load the subway matrix if a path was given, else announce the fallback. The matrix's
/// stations replace the stations CSV as the access/egress target set.
fn load_subway(path: Option<&str>) -> Result<Option<sim_core::SubwayMatrix>> {
    match path {
        Some(p) => {
            let m = sim_core::SubwayMatrix::from_bytes(&read(p)?)
                .map_err(|e| anyhow::anyhow!("decoding {p}: {e}"))?;
            eprintln!(
                "  transit: subway-graph router — {} stations; {}",
                m.len(),
                m.provenance.notes
            );
            Ok(Some(m))
        }
        None => {
            eprintln!(
                "  transit: crow-flies × circuity fallback (pass a .ossub asset for the \
                 subway-graph router)"
            );
            Ok(None)
        }
    }
}

fn read(path: &str) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("reading {path} (bake assets first)"))
}

/// The observed mobile classes (ACE buses + rideshare dashcams), loaded for the
/// block-group/OD exposure commands — the M1 plumbing that lets [[M_i]] terms ride
/// alongside the fixed-camera census.
///
/// **Graceful absence:** if either baked asset is missing, that class is silently
/// disabled and its output columns are 0 — but a stderr line says so loudly, because a
/// zero column in `data/derived/` must mean "layer off", never "layer forgotten". The
/// heatmap already loads these same two assets; this is the same universe, finally
/// shared by every command.
struct MobileLayers {
    ace_tree: Option<RTree<[f64; 2]>>,
    ace_cap_r2: f64,
    ace_routes: usize,
    dashcam: Option<sim_core::DashcamFieldLayer>,
    mobile: sim_core::MobileScenario,
    dashcam_on: bool,
}

impl MobileLayers {
    fn load() -> Result<Self> {
        let mut mobile = sim_core::MobileScenario::fields_only();
        // Only the OBSERVED fleets are enabled for the paper's tables; glasses stay
        // scenario-only (they are Tier D and excluded from empirical claims).
        mobile.glasses = None;

        let mut ace_tree: Option<RTree<[f64; 2]>> = None;
        let (mut ace_cap_r2, mut ace_routes) = (0.0_f64, 0_usize);
        match std::fs::read(ACE_PATH) {
            Ok(bytes) => match sim_core::AceCorridorLayer::from_bytes(&bytes) {
                Ok(ace) => {
                    ace_routes = ace.routes.len();
                    let cfg = sim_core::AceConfig::new(
                        ace.segments
                            .iter()
                            .map(|s| {
                                [sim_core::Vec2::new(s[0][0], s[0][1]), sim_core::Vec2::new(s[1][0], s[1][1])]
                            })
                            .collect(),
                    );
                    ace_cap_r2 = cfg.capture_range_m.powi(2);
                    // Densify each segment to ~10 m points so proximity queries don't
                    // miss the middle of long segments (same as the heatmap).
                    let mut pts = Vec::new();
                    for s in &ace.segments {
                        let a = sim_core::Vec2::new(s[0][0], s[0][1]);
                        let b = sim_core::Vec2::new(s[1][0], s[1][1]);
                        let n = (a.distance(b) / 10.0).ceil().max(1.0) as usize;
                        for k in 0..=n {
                            let p = a.lerp(b, k as f64 / n as f64);
                            pts.push([p.x, p.y]);
                        }
                    }
                    ace_tree = Some(RTree::bulk_load(pts));
                    mobile.ace = Some(cfg);
                }
                Err(e) => eprintln!("  WARNING: ACE asset failed to decode ({e}); ACE class OFF"),
            },
            Err(_) => eprintln!(
                "  WARNING: {ACE_PATH} missing — ACE class OFF (bake it with data-pipeline bake-ace)"
            ),
        }

        let dashcam = std::fs::read(DASHCAM_PATH)
            .ok()
            .and_then(|b| sim_core::DashcamFieldLayer::from_bytes(&b).ok());
        let dashcam_on = dashcam.is_some();
        if !dashcam_on {
            eprintln!(
                "  WARNING: {DASHCAM_PATH} missing — dashcam class OFF (bake it with bake-dashcam-field)"
            );
        }
        mobile.dashcam = dashcam.as_ref().map(|_| {
            // The field scales the config's baseline rate spatially; defaults carry the
            // penetration/capture assumptions, which sweep via OURSPACE_DASH_PEN /
            // OURSPACE_DASH_CAP (see `sweep_dashcam_params`).
            sim_core::DashcamConfig {
                penetration: env_f64("OURSPACE_DASH_PEN", 0.40),
                capture_prob: env_f64("OURSPACE_DASH_CAP", 0.40),
                ..sim_core::DashcamConfig::default()
            }
        });

        Ok(Self { ace_tree, ace_cap_r2, ace_routes, dashcam, mobile, dashcam_on })
    }
}

/// Hour-averaging weights over a representative day for walkshed-resident time:
/// 8 waking hours weighted by each hour's activity level, normalized to sum to 1.
/// The residential term integrates over *presence*, not a single departure hour,
/// so a single-hour snapshot would misstate it; this is the cheap defensible prior
/// (the composite E_i already uses a time-budget argument of the same kind).
fn day_weights() -> [(f64, f64); 8] {
    [
        (8.5, sim_core::traffic_multiplier(8.5)),
        (10.5, sim_core::traffic_multiplier(10.5)),
        (13.0, sim_core::traffic_multiplier(13.0)),
        (15.0, sim_core::traffic_multiplier(15.0)),
        (17.5, sim_core::traffic_multiplier(17.5)),
        (19.0, sim_core::traffic_multiplier(19.0)),
        (21.0, sim_core::traffic_multiplier(21.0)),
        (23.0, sim_core::traffic_multiplier(23.0)),
    ]
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key).ok().and_then(|s| s.trim().parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(default)
}

/// ACS B08301 bus share within the transit mode (the transit sub-mode weight used by
/// `od-exposure-modal`'s `bus_share` closure, factored out for M3 pair emission).
fn bus_share_of(acs: &Table, geoid: &str) -> f64 {
    let b = fnum(acs.get(geoid, "commute_bus")).unwrap_or(0.0);
    let tr = fnum(acs.get(geoid, "commute_transit")).unwrap_or(0.0);
    if tr > 0.0 { (b / tr).clamp(0.0, 1.0) } else { 0.35 }
}


fn edge_midpoint(e: &EdgeData) -> Enu {
    let p = &e.polyline[e.polyline.len() / 2];
    Enu::new(p[0], p[1])
}

/// The street-view CCTV census recall used by every exposure command.
///
/// **Default `r = 1.0`: the instrument reports OBSERVED counts, uncorrected.** That is a
/// deliberate design choice, not an oversight — and the reason is worth stating, because the
/// obvious alternative (bake in [`sim_core::CENSUS_RECALL`] ≈ 0.501) is wrong twice over:
///
/// 1. **Recall is an estimate with a confidence interval** ([0.458, 0.544]) and is a
///    *conservative lower bound* (both censuses derive from Street View, so their positive
///    dependence biases N̂ down). Baking a point estimate into `R_i` would silently propagate it
///    into every downstream number and **discard the uncertainty** — precisely the uncertainty
///    the paper needs to report.
/// 2. **The correction cannot change the finding anyway.** Detection is non-differential (no
///    demographic gradient: %Hisp p=0.89, %Black p=0.89, income p=0.42), so the correction is a
///    near-uniform ~1.94× rescaling. It moves the *relative* disparity by ~+6% — a robustness
///    check, not an amplification. Reporting it as if it "doubled the disparity" would be a
///    units artifact (a coefficient in cameras/SD doubles when the outcome doubles).
///
/// So the instrument observes, and the *analysis* corrects — where the bootstrap can propagate
/// the CI. We emit `cameras_unconfirmed` (the CCTV-census-only sub-population, the only part the
/// correction inflates) so that any `r`, including every bootstrap draw, is exactly
/// reconstructable from one bake with no re-run:
///
/// ```text
/// cameras(r) = cameras_unconfirmed / r + (cameras_raw - cameras_unconfirmed)
/// ```
///
/// Set `OURSPACE_CENSUS_RECALL=0.501` to have the batch emit the corrected estimate directly.
/// Deliberately NOT read from the asset's `recall` field — recall is an analysis parameter, not
/// a property of the data.
fn census_recall() -> f64 {
    std::env::var("OURSPACE_CENSUS_RECALL")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|r| *r > 0.0 && *r <= 1.0)
        .unwrap_or(1.0)
}

/// Assemble the observed fixed-camera set (CCTV census + ALPR + DOT + enforcement),
/// grouped across sources into physical-camera nodes (so a multiply-attested camera
/// counts once), plus the building-occlusion index every sightline is tested against.
///
/// Returns `(sensors, recall_factor, occluders)` where `recall_factor = 1/r` is the
/// inflation applied to CCTV-census-only groups by `walkshed_exposure`.
///
/// The occlusion index is built *here*, not by the caller, because each sensor's
/// `host_poly` — the footprint its apex sits inside — can only be resolved against a
/// specific index, and a sensor carrying a `host_poly` from a *different* index would
/// exclude an arbitrary unrelated building from its sightlines. Binding the two together
/// makes that mismatch unrepresentable.
fn load_fixed_sensors() -> Result<(Vec<SensorInstance>, f64, sim_core::OccluderIndex)> {
    let cctv = CctvCameraLayer::from_bytes(&read(CAMERAS_NYC)?).context("decoding CCTV census")?;
    let r = census_recall();
    let recall = 1.0 / r;
    let mut sensors = sensors_from_layer(&cctv.to_fixed_layer(), FixedCameraDefaults::default());
    if let Ok(b) = std::fs::read(ALPR_NYC) {
        if let Ok(al) = AlprReaderLayer::from_bytes(&b) {
            sensors.extend(sensors_from_layer(&al.to_fixed_layer(), FixedCameraDefaults::default()));
        }
    }
    if let Ok(b) = std::fs::read(DOT_NYC) {
        if let Ok(dot) = FixedSensorLayer::from_bytes(&b) {
            sensors.extend(sensors_from_layer(&dot, FixedCameraDefaults::dot_monitoring()));
        }
    }
    if let Ok(b) = std::fs::read(ENFORCE_NYC) {
        if let Ok(enf) = FixedSensorLayer::from_bytes(&b) {
            sensors.extend(sensors_from_layer(&enf, FixedCameraDefaults::default()));
        }
    }
    for (i, s) in sensors.iter_mut().enumerate() {
        s.id = i as u64;
    }
    let groups = sim_core::group_sensors(&mut sensors, FIXED_GROUP_RADIUS_M);
    let unconfirmed = sensors.iter().filter(|s| !s.confirmed).count();
    eprintln!(
        "  fixed cameras: {} sensors → {} physical-camera groups \
         (census recall r={r:.3} → CCTV-only groups ×{recall:.3}; {unconfirmed} unconfirmed sensors)",
        sensors.len(),
        groups,
    );

    // Occlusion's bite depends entirely on how far we assume a camera can see: at the modeled
    // 15 m (CCTV) / 30 m (DOT) a street-mounted camera's reach barely leaves the street canyon,
    // so buildings have nothing to block. `OURSPACE_RANGE_SCALE` multiplies every range so that
    // claim can be *tested* rather than asserted (docs/OCCLUSION_PLAN.md §6 sensitivity grid).
    if let Some(k) = std::env::var("OURSPACE_RANGE_SCALE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|k| *k > 0.0 && (*k - 1.0).abs() > 1e-9)
    {
        for s in sensors.iter_mut() {
            s.wedge.range_m *= k;
        }
        eprintln!("  camera range ×{k:.2} (OURSPACE_RANGE_SCALE) — sensitivity run, NOT the headline");
    }

    // A facade-mounted camera looks *out* from its host building — that building must not
    // occlude it, or it is silently blind in every direction. Resolved once, at load.
    let occ = load_occluders()?;
    let mut hosted = 0usize;
    for s in sensors.iter_mut() {
        s.host_poly = occ.containing_polygon(s.wedge.apex);
        if s.host_poly.is_some() {
            hosted += 1;
        }
    }
    if !occ.is_empty() {
        eprintln!(
            "  occlusion: {} footprints → {} walls; {hosted} sensors sit inside a footprint \
             (host building excluded from their sightlines)",
            occ.n_polygons(),
            occ.n_walls(),
        );
    }
    Ok((sensors, recall, occ))
}

/// Network descriptives for a baked graph, broken out by CSCL road class.
///
/// Exists to make the drive-vs-walk distinction *auditable* rather than asserted: the drive graph
/// must contain highways and no park paths, the walk graph must contain park paths and no
/// highways. Run it on both and the two keep-lists are visible side by side — which is also the
/// paper's network-descriptives table.
fn graph_stats(drive: &str, walk: &str, out: Option<&str>) -> Result<()> {
    let d = graph_class_km(drive)?;
    let w = graph_class_km(walk)?;
    for (label, g) in [("DRIVE", &d), ("WALK", &w)] {
        println!("\n{label}: {} nodes, {} edges", g.nodes, g.edges);
        for (name, km, n) in &g.classes {
            println!("  {name:<12} {n:>8} edges {km:>9.1} km");
        }
    }
    if let Some(path) = out {
        // The paper's network table is generated from this, so the .tex can never drift from
        // the assets the numbers were actually computed on (tools/make_tables.py).
        let j = |g: &GraphClassKm| {
            let km: Vec<String> = g
                .classes
                .iter()
                .map(|(n, km, _)| format!("\"{n}\": {km:.1}"))
                .collect();
            format!(
                "{{\"nodes\": {}, \"edges\": {}, \"km\": {{{}}}}}",
                g.nodes,
                g.edges,
                km.join(", ")
            )
        };
        std::fs::write(path, format!("{{\n  \"drive\": {},\n  \"walk\": {}\n}}\n", j(&d), j(&w)))
            .with_context(|| format!("writing {path}"))?;
        eprintln!("graph-stats -> {path}");
    }
    Ok(())
}

struct GraphClassKm {
    nodes: usize,
    edges: usize,
    classes: Vec<(&'static str, f64, usize)>,
}

fn graph_class_km(path: &str) -> Result<GraphClassKm> {
    use std::collections::BTreeMap;
    let g = GraphAsset::from_bytes(&read(path)?).context("decoding graph")?;
    // `segment_id` packs CSCL class + posted speed: rw_type * 100 + mph.
    const RW: [(i64, &str); 11] = [
        (1, "Street"),
        (2, "Highway"),
        (3, "Bridge"),
        (4, "Tunnel"),
        (5, "Boardwalk"),
        (6, "Path/Trail"),
        (7, "Step"),
        (9, "Ramp"),
        (10, "Alley"),
        (100, "connector*"), // synthetic stitch edge (CONNECTOR_SEGMENT_ID)
        (0, "unknown"),
    ];
    let mut km: BTreeMap<i64, f64> = BTreeMap::new();
    let mut n: BTreeMap<i64, usize> = BTreeMap::new();
    for e in &g.edges {
        let cls = match e.segment_id {
            Some(100) => 100, // the stitch connector, not rw_type 1 @ 0 mph
            Some(s) => s / 100,
            None => 0,
        };
        *km.entry(cls).or_default() += e.length_m / 1000.0;
        *n.entry(cls).or_default() += 1;
    }
    let classes: Vec<(&'static str, f64, usize)> = RW
        .iter()
        .filter_map(|&(c, name)| n.get(&c).map(|&e| (name, km[&c], e)))
        .collect();
    Ok(GraphClassKm {
        nodes: g.nodes.len(),
        edges: g.edges.len(),
        classes,
    })
}

/// Why does building occlusion barely move `R_i`? Separate the two levels at which it can act.
///
/// `R_i` counts **distinct cameras seen anywhere on a 10-minute walkshed**. Occlusion deletes
/// individual *(camera, street-point)* sightlines — but a camera only stops counting if *every*
/// one of its sightlines into the walkshed dies. This walks the identical walksheds as
/// `bg-exposure` and reports both levels:
///
/// - **point-level** — `pairs_occl / pairs_free`, over (camera, street-point) pairs. This is the
///   quantity a *place*-based exposure measure (e.g. a heatmap cell) would feel.
/// - **path-level** — `groups_occl / groups_free`, i.e. `R_i` itself. What a *person* feels.
/// - `groups_saved` — cameras blocked at ≥1 walkshed point yet still counted, because they see
///   the walker somewhere else. This is exactly the gap between the two levels.
///
/// If point-level attenuation is large while path-level is ~0, the null result is real and has a
/// mechanism. If *both* are ~0 the occlusion test is not firing and something is miswired — which
/// is the failure this subcommand exists to rule out.
fn occlusion_audit(graph_path: &str, points_path: &str, out_path: &str, walk_min: f64) -> Result<()> {
    use rayon::prelude::*;
    use sim_core::graph::{StreetGraph, DEFAULT_WALK_SPEED_MPS};
    use sim_core::math::Vec2;
    use sim_core::projection::EnuProjection;
    use std::collections::{HashMap, HashSet};

    let graph =
        StreetGraph::from_asset(GraphAsset::from_bytes(&read(graph_path)?).context("decoding graph")?);
    let (sensors, _recall, occ) = load_fixed_sensors()?;
    if occ.is_empty() {
        anyhow::bail!("occlusion-audit needs the occlusion index (unset OURSPACE_OCCLUSION=0)");
    }
    let proj = EnuProjection::default();
    let max_seconds = walk_min * 60.0;
    let cull_r2 = (max_seconds * DEFAULT_WALK_SPEED_MPS + 300.0).powi(2);
    let cam_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        sensors
            .iter()
            .enumerate()
            .map(|(i, s)| GeomWithData::new([s.wedge.apex.x, s.wedge.apex.y], i))
            .collect(),
    );

    let text = std::fs::read_to_string(points_path).with_context(|| format!("reading {points_path}"))?;
    let rows: Vec<(String, f64, f64)> = text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.trim().split(',').collect();
            match (
                f.first().map(|s| s.trim().to_string()),
                f.get(1)?.trim().parse::<f64>(),
                f.get(2)?.trim().parse::<f64>(),
            ) {
                (Some(id), Ok(lat), Ok(lon)) => Some((id, lat, lon)),
                _ => None,
            }
        })
        .collect();

    let edges = &graph.asset().edges;
    let out_rows: Vec<String> = rows
        .par_iter()
        .filter_map(|(id, lat, lon)| {
            let enu = proj.to_enu(*lat, *lon);
            let node = graph.snap_nearest(enu)?;
            let ws = graph.walkshed(node, max_seconds, DEFAULT_WALK_SPEED_MPS);
            let nearby: Vec<SensorInstance> = cam_tree
                .locate_within_distance([enu.x, enu.y], cull_r2)
                .map(|g| sensors[g.data])
                .collect();

            // (camera-group, street-point) sightline pairs, free-space vs occluded.
            let (mut pairs_free, mut pairs_occl) = (0u64, 0u64);
            // Per group: did it ever see the walker free-space / occluded / get blocked anywhere?
            let mut g_free: HashSet<u32> = HashSet::new();
            let mut g_occl: HashSet<u32> = HashSet::new();
            let mut g_blocked: HashMap<u32, bool> = HashMap::new();

            // Identical sample geometry to `walkshed_exposure` (shared stride, full +
            // boundary-partial edges), so the audit decomposes exactly the sightlines the
            // instrument evaluates — not an approximation of them.
            let mut points: Vec<Vec2> = Vec::new();
            for &ei in &ws.edges {
                points.extend(sim_core::sample_polyline(
                    &edges[ei as usize].polyline,
                    sim_core::EXPOSURE_SAMPLE_STRIDE_M,
                    true,
                    1.0,
                ));
            }
            for &(ei, entry, frac) in &ws.partial {
                let e = &edges[ei as usize];
                points.extend(sim_core::sample_polyline(
                    &e.polyline,
                    sim_core::EXPOSURE_SAMPLE_STRIDE_M,
                    entry == e.from,
                    frac,
                ));
            }
            for pt in points {
                for s in &nearby {
                    if !s.wedge.covers_unoccluded(pt) {
                        continue; // out of range/FOV — occlusion is not what excludes it
                    }
                    pairs_free += 1;
                    g_free.insert(s.group);
                    if occ.blocked(s.wedge.apex, pt, s.host_poly) {
                        g_blocked.insert(s.group, true);
                    } else {
                        pairs_occl += 1;
                        g_occl.insert(s.group);
                    }
                }
            }
            // Cameras blocked somewhere but still counted — they see the walker elsewhere.
            let saved = g_blocked.keys().filter(|g| g_occl.contains(g)).count();
            // Cameras occlusion actually removes from R_i (blocked at *every* sightline).
            let killed = g_free.len() - g_occl.len();
            Some(format!(
                "{id},{},{},{},{},{},{}\n",
                pairs_free,
                pairs_occl,
                g_free.len(),
                g_occl.len(),
                saved,
                killed
            ))
        })
        .collect();

    let mut out = String::from("id,pairs_free,pairs_occl,groups_free,groups_occl,groups_saved,groups_killed\n");
    out.extend(out_rows.iter().cloned());
    std::fs::write(out_path, out).with_context(|| format!("writing {out_path}"))?;
    eprintln!("occlusion-audit: {} points -> {out_path}", out_rows.len());
    Ok(())
}

/// Residential surveillance-exposure per origin point (`R_i` in the disparity plan,
/// docs/surveillance-exposure-disparity-plan.md): for each `id,lat,lon`, snap to the
/// pedestrian graph, flood a `walk_min` walkshed, and count the recall-corrected distinct
/// fixed cameras whose FOV covers any reachable street point — total + per source type.
/// Emits a tidy CSV keyed by `id` (block-group GEOID in the real run; any point set works).
fn bg_exposure(graph_path: &str, points_path: &str, out_path: &str, walk_min: f64) -> Result<()> {
    use sim_core::graph::{StreetGraph, DEFAULT_WALK_SPEED_MPS};
    use sim_core::projection::EnuProjection;
    use sim_core::SourceKind;

    let graph =
        StreetGraph::from_asset(GraphAsset::from_bytes(&read(graph_path)?).context("decoding graph")?);
    let (sensors, recall, occ) = load_fixed_sensors()?;
    let fov = sim_core::FovModel::from_env();
    eprintln!("  census FOV: {}", fov.describe());
    // M1: the observed mobile classes ride alongside the fixed census. Their per-minute
    // encounter rates are averaged over a representative day (see `day_weights`) because
    // a resident's walkshed presence spans hours, not one departure minute.
    let mob = MobileLayers::load()?;
    if mob.mobile.ace.is_some() {
        eprintln!("  mobile: {} ACE routes loaded", mob.ace_routes);
    }
    if mob.dashcam_on {
        eprintln!(
            "  mobile: dashcam field loaded (penetration {:.2}, capture {:.2}; env OURSPACE_DASH_PEN/OURSPACE_DASH_CAP)",
            env_f64("OURSPACE_DASH_PEN", 0.40),
            env_f64("OURSPACE_DASH_CAP", 0.40)
        );
    }
    let proj = EnuProjection::default();
    let max_seconds = walk_min * 60.0;

    // Per-point spatial cull: any camera that can capture a walkshed point is within
    // (walk radius + max camera range) of the origin. 300 m range bound is generous for
    // any street-level fixed camera, so the cull is exact — it never drops a capturer.
    let cull_r2 = (max_seconds * DEFAULT_WALK_SPEED_MPS + 300.0).powi(2);
    let cam_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        sensors
            .iter()
            .enumerate()
            .map(|(i, s)| GeomWithData::new([s.wedge.apex.x, s.wedge.apex.y], i))
            .collect(),
    );

    let text = std::fs::read_to_string(points_path)
        .with_context(|| format!("reading {points_path}"))?;
    let mut out = String::from(
        // `cameras_unconfirmed` is the CCTV-census-only sub-population — the ONLY part the
        // recall correction inflates. Because the correction is linear, emitting it makes any
        // recall r reconstructable from this one bake, with no re-run:
        //     cameras(r) = cameras_unconfirmed / r + (cameras_raw - cameras_unconfirmed)
        // That is what lets the undercount bootstrap draw 500 recall values cheaply.
        "id,lat,lon,snap_m,reachable_edges,cameras_raw,cameras_unconfirmed,cameras_corrected,cctv,alpr,dot,enforcement,m_ace_res,m_dash_res\n",
    );
    let (mut n, mut skipped) = (0usize, 0usize);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        // A header row (non-numeric lat/lon) is skipped by the parse guard below.
        let (Some(id), Ok(lat), Ok(lon)) = (
            f.first().map(|s| s.trim()),
            f.get(1).unwrap_or(&"").trim().parse::<f64>(),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) else {
            skipped += 1;
            continue;
        };
        let enu = proj.to_enu(lat, lon);
        let Some(node) = graph.snap_nearest(enu) else {
            skipped += 1;
            continue;
        };
        let snap_m = enu.distance(graph.node_pos(node));
        let ws = graph.walkshed(node, max_seconds, DEFAULT_WALK_SPEED_MPS);
        let nearby: Vec<SensorInstance> = cam_tree
            .locate_within_distance([enu.x, enu.y], cull_r2)
            .map(|g| sensors[g.data])
            .collect();
        let s = sim_core::walkshed_exposure_with(&graph, &ws, &nearby, &occ, recall, &fov, None);
        // M_i^res: expected devices/min from each observed mobile class, averaged over a
        // representative day of waking hours (weights ∝ each hour's activity level). Sample
        // points retrace the same walkshed edges the fixed census used, thinned to a 50 m
        // stride (the mobile fields are smooth zone-level intensities; 10 m buys nothing);
        // the reported figure is the mean day-averaged per-point rate over the walkshed —
        // the "typical sidewalk moment in my walkshed". A full space-time integral needs
        // trajectory-level presence data (future work).
        let (mut ace_res, mut dash_res) = (0.0_f64, 0.0_f64);
        if mob.mobile.ace.is_some() || mob.dashcam_on {
            const MOBILE_STRIDE_M: f64 = 50.0;
            let mut n_samples = 0_usize;
            let mut sw = [0.0_f64; 2];
            let edges = &graph.asset().edges;
            let mut scratch: Vec<sim_core::Vec2> = Vec::new();
            let weights = day_weights();
            let wsum: f64 = weights.iter().map(|(_, w)| w).sum();
            for &ei in &ws.edges {
                sim_core::sample_polyline_into(
                    &edges[ei as usize].polyline,
                    MOBILE_STRIDE_M,
                    true,
                    1.0,
                    &mut scratch,
                );
                for p in &scratch {
                    // ACE proximity test once per sample (cheap R-tree lookup); the rate
                    // itself is hour-folded below.
                    let near_ace = mob
                        .ace_tree
                        .as_ref()
                        .is_some_and(|t| t.locate_within_distance([p.x, p.y], mob.ace_cap_r2).next().is_some());
                    for &(h, w) in &weights {
                        let r = sim_core::exposure_rates_per_minute(
                            *p,
                            h,
                            &[],
                            &occ,
                            near_ace,
                            &mob.mobile,
                            recall,
                            mob.dashcam.as_ref(),
                            None,
                            None,
                            None,
                        );
                        sw[0] += r.ace * w;
                        sw[1] += r.dashcam * w;
                    }
                    n_samples += 1;
                }
                scratch.clear();
            }
            if n_samples > 0 {
                let inv = 1.0 / (n_samples as f64 * wsum);
                ace_res = sw[0] * inv;
                dash_res = sw[1] * inv;
            }
        }
        // Per-type distinct-camera tally (each group counted once, under its representative
        // kind) for the ALPR-vs-traffic-vs-CCTV type contrast (plan §5d).
        let (mut cctv, mut alpr, mut dot, mut enf) = (0u32, 0u32, 0u32, 0u32);
        for k in &s.camera_kinds {
            match k {
                SourceKind::Alpr => alpr += 1,
                SourceKind::DotLiveView => dot += 1,
                SourceKind::EnforcementCamera => enf += 1,
                _ => cctv += 1,
            }
        }
        out.push_str(&format!(
            "{id},{lat:.6},{lon:.6},{snap_m:.1},{},{},{:.4},{:.4},{cctv},{alpr},{dot},{enf},{ace_res:.4},{dash_res:.4}\n",
            s.reachable_edges,
            s.cameras_raw,
            s.cameras_unconfirmed, // weighted under the census-FOV expectation → fractional
            s.cameras_corrected.max(0.0), // normalize -0.0 → 0.0 for zero-camera walksheds
        ));
        n += 1;
    }
    std::fs::write(out_path, out).with_context(|| format!("writing {out_path}"))?;
    eprintln!("bg-exposure: {n} points ({skipped} skipped), {walk_min:.0}-min walkshed -> {out_path}");
    Ok(())
}

/// Need-neutral counterfactual (plan §5a). Holds the total observed street-camera burden fixed but
/// redistributes it so camera intensity ∝ a legitimate "need" proxy — ambient population, or
/// population + inbound jobs as a daytime foot-traffic proxy. Under a placement whose intensity ∝
/// need density, the expected number of cameras in a BG's 10-min walkshed is proportional to the
/// need integrated over that walkshed (`reach_need` = Σ of the need weights of every BG centroid
/// graph-reachable within the walkshed). We scale `reach_need` to conserve the *population-weighted
/// mean* of the actual R_i, so the citywide surveillance total is unchanged and only its spatial
/// *distribution* moves. Each BG's **excess** = actual R_i − need-neutral R_i; Σ pop·excess = 0 by
/// construction, so a group's mean excess > 0 means it carries more exposure than a need-neutral
/// placement predicts — the interpretable, regression-spec-free framing the plan wants.
fn counterfactual(
    graph_path: &str,
    centroids_path: &str,
    od_path: &str,
    out_path: &str,
    walk_min: f64,
) -> Result<()> {
    use rayon::prelude::*;
    use sim_core::{EnuProjection, StreetGraph, DEFAULT_WALK_SPEED_MPS};
    use std::collections::HashMap;

    // Walkshed-only: every quantity here is "what is reachable on foot from i", so this takes the
    // **pedestrian** graph and nothing else. Callers must pass `graph_nyc_walk.osgraph`.
    let graph = StreetGraph::from_asset(
        GraphAsset::from_bytes(&read(graph_path)?).context("decoding walk graph")?,
    );
    let (sensors, recall, occ) = load_fixed_sensors()?;
    let fov = sim_core::FovModel::from_env();
    eprintln!("  census FOV: {}", fov.describe());
    let proj = EnuProjection::default();
    let max_seconds = walk_min * 60.0;
    let cull_r2 = (max_seconds * DEFAULT_WALK_SPEED_MPS + 300.0).powi(2);
    let cam_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        sensors
            .iter()
            .enumerate()
            .map(|(i, s)| GeomWithData::new([s.wedge.apex.x, s.wedge.apex.y], i))
            .collect(),
    );

    // Inbound jobs per work BG (Σ over home origins of the LODES OD flow): a daytime foot-traffic
    // proxy that captures commercial/employment cores where ambient population ≠ residents.
    let mut jobs: HashMap<String, f64> = HashMap::new();
    for line in std::fs::read_to_string(od_path)
        .with_context(|| format!("reading {od_path}"))?
        .lines()
    {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(w), Ok(j)) = (
            f.get(1).map(|s| s.trim()),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) else {
            continue; // header / malformed
        };
        *jobs.entry(w.to_string()).or_default() += j;
    }

    // BG centroids (population-weighted) with population + inbound jobs, snapped to a graph node.
    struct Bg {
        id: String,
        lat: f64,
        lon: f64,
        enu: Enu,
        node: u32,
        pop: f64,
        jobs: f64,
    }
    let mut bgs: Vec<Bg> = Vec::new();
    for line in std::fs::read_to_string(centroids_path)
        .with_context(|| format!("reading {centroids_path}"))?
        .lines()
    {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(id), Ok(lat), Ok(lon)) = (
            f.first().map(|s| s.trim()),
            f.get(1).unwrap_or(&"").trim().parse::<f64>(),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) else {
            continue; // header row
        };
        let pop: f64 = f.get(3).unwrap_or(&"0").trim().parse().unwrap_or(0.0);
        let enu = proj.to_enu(lat, lon);
        if let Some(node) = graph.snap_nearest(enu) {
            let j = jobs.get(id).copied().unwrap_or(0.0);
            bgs.push(Bg { id: id.to_string(), lat, lon, enu, node, pop, jobs: j });
        }
    }

    // RTree over BG centroids: a walkshed's reachable BGs are found by spatial cull (max walk
    // radius) then filtered on true graph reachability (centroid node ∈ walkshed.node_time).
    let bg_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        bgs.iter()
            .enumerate()
            .map(|(i, b)| GeomWithData::new([b.enu.x, b.enu.y], i))
            .collect(),
    );
    let reach_r2 = (max_seconds * DEFAULT_WALK_SPEED_MPS).powi(2);

    // One walkshed per BG yields both R_i^actual (cameras in it) and reach_need (need reachable
    // within it), so the actual and need-neutral quantities are defined over the identical area.
    struct Row {
        id: String,
        lat: f64,
        lon: f64,
        pop: f64,
        r_actual: f64,
        reach_pop: f64,
        reach_amb: f64,
    }
    let rows: Vec<Row> = bgs
        .par_iter()
        .map(|b| {
            let ws = graph.walkshed(b.node, max_seconds, DEFAULT_WALK_SPEED_MPS);
            let nearby: Vec<SensorInstance> = cam_tree
                .locate_within_distance([b.enu.x, b.enu.y], cull_r2)
                .map(|g| sensors[g.data])
                .collect();
            let r_actual = sim_core::walkshed_exposure_with(&graph, &ws, &nearby, &occ, recall, &fov, None)
                .cameras_corrected
                .max(0.0);
            let (mut reach_pop, mut reach_amb) = (0.0f64, 0.0f64);
            for cand in bg_tree.locate_within_distance([b.enu.x, b.enu.y], reach_r2) {
                let j = &bgs[cand.data];
                if ws.node_time.contains_key(&j.node) {
                    reach_pop += j.pop;
                    reach_amb += j.pop + j.jobs;
                }
            }
            Row {
                id: b.id.clone(),
                lat: b.lat,
                lon: b.lon,
                pop: b.pop,
                r_actual,
                reach_pop,
                reach_amb,
            }
        })
        .collect();

    // Conserve population-weighted mean exposure: scale_w = Σ pop·R_actual / Σ pop·reach_w.
    let sum_pop_r: f64 = rows.iter().map(|r| r.pop * r.r_actual).sum();
    let sum_pop_reach_pop: f64 = rows.iter().map(|r| r.pop * r.reach_pop).sum();
    let sum_pop_reach_amb: f64 = rows.iter().map(|r| r.pop * r.reach_amb).sum();
    let scale_pop = if sum_pop_reach_pop > 0.0 { sum_pop_r / sum_pop_reach_pop } else { 0.0 };
    let scale_amb = if sum_pop_reach_amb > 0.0 { sum_pop_r / sum_pop_reach_amb } else { 0.0 };

    let mut sorted = rows;
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut out = String::from(
        "id,lat,lon,population,R_actual,reach_pop,reach_ambient,\
         R_neutral_pop,R_neutral_ambient,excess_pop,excess_ambient\n",
    );
    for r in &sorted {
        let rn_pop = scale_pop * r.reach_pop;
        let rn_amb = scale_amb * r.reach_amb;
        out.push_str(&format!(
            "{},{:.6},{:.6},{:.0},{:.3},{:.1},{:.1},{:.3},{:.3},{:.3},{:.3}\n",
            r.id, r.lat, r.lon, r.pop, r.r_actual, r.reach_pop, r.reach_amb,
            rn_pop, rn_amb, r.r_actual - rn_pop, r.r_actual - rn_amb,
        ));
    }
    let pop_total: f64 = sorted.iter().map(|r| r.pop).sum();
    std::fs::write(out_path, &out).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "counterfactual: {} BGs; conserve pop-wtd mean R={:.2}; scale_pop={scale_pop:.4} scale_amb={scale_amb:.4} -> {out_path}",
        sorted.len(),
        if pop_total > 0.0 { sum_pop_r / pop_total } else { 0.0 },
    );
    Ok(())
}

/// Per-BG "need"/land-use covariates for the §5c crime-control ladder, all measured over the same
/// 10-min pedestrian walkshed as R_i so they are directly comparable to the exposure outcome. For
/// each block group we count, within its walkshed: crime complaints (total + felony), reachable
/// inbound jobs (commercial/employment density), reachable residents (ambient population), and
/// transit access (nearest subway-station distance + stations within the walk radius). The ladder
/// (Python) then regresses exposure on demographics with staged controls: (1) none, (2) +commercial
/// /transit/land-use, (3) +crime — the (2)→(3) coefficient move is the share of the disparity that
/// runs through the crime-justification channel.
fn covariates(
    graph_path: &str,
    centroids_path: &str,
    od_path: &str,
    stations_path: &str,
    crime_path: &str,
    req311_path: Option<&str>,
    out_path: &str,
    walk_min: f64,
) -> Result<()> {
    use rayon::prelude::*;
    use sim_core::{EnuProjection, StreetGraph, DEFAULT_WALK_SPEED_MPS};
    use std::collections::HashMap;

    let graph = StreetGraph::from_asset(
        GraphAsset::from_bytes(&read(graph_path)?).context("decoding graph")?,
    );
    let proj = EnuProjection::default();
    let max_seconds = walk_min * 60.0;
    let reach_r2 = (max_seconds * DEFAULT_WALK_SPEED_MPS).powi(2);

    // Inbound jobs per work BG (Σ over home origins of LODES OD flow): commercial/employment density.
    let mut jobs: HashMap<String, f64> = HashMap::new();
    for line in std::fs::read_to_string(od_path)
        .with_context(|| format!("reading {od_path}"))?
        .lines()
    {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(w), Ok(j)) = (
            f.get(1).map(|s| s.trim()),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) else {
            continue;
        };
        *jobs.entry(w.to_string()).or_default() += j;
    }

    // BG centroids with population + inbound jobs, snapped to a graph node.
    struct Bg {
        id: String,
        lat: f64,
        lon: f64,
        enu: Enu,
        node: u32,
        pop: f64,
        jobs: f64,
    }
    let mut bgs: Vec<Bg> = Vec::new();
    for line in std::fs::read_to_string(centroids_path)
        .with_context(|| format!("reading {centroids_path}"))?
        .lines()
    {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(id), Ok(lat), Ok(lon)) = (
            f.first().map(|s| s.trim()),
            f.get(1).unwrap_or(&"").trim().parse::<f64>(),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) else {
            continue;
        };
        let pop: f64 = f.get(3).unwrap_or(&"0").trim().parse().unwrap_or(0.0);
        let enu = proj.to_enu(lat, lon);
        if let Some(node) = graph.snap_nearest(enu) {
            let j = jobs.get(id).copied().unwrap_or(0.0);
            bgs.push(Bg { id: id.to_string(), lat, lon, enu, node, pop, jobs: j });
        }
    }
    let bg_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        bgs.iter().enumerate().map(|(i, b)| GeomWithData::new([b.enu.x, b.enu.y], i)).collect(),
    );

    // Subway stations (transit access).
    let mut station_enu: Vec<Enu> = Vec::new();
    for line in std::fs::read_to_string(stations_path)
        .with_context(|| format!("reading {stations_path}"))?
        .lines()
    {
        let f: Vec<&str> = line.split(',').collect();
        if let (Ok(lat), Ok(lon)) = (
            f.get(1).unwrap_or(&"").trim().parse::<f64>(),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) {
            station_enu.push(proj.to_enu(lat, lon));
        }
    }
    let station_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        station_enu.iter().enumerate().map(|(i, p)| GeomWithData::new([p.x, p.y], i)).collect(),
    );
    // Each station's nearest street node, snapped once — `stations_wsh` filters on walkshed
    // membership of this node, the same rule as every other _wsh covariate.
    let station_nodes: Vec<Option<u32>> =
        station_enu.iter().map(|p| graph.snap_nearest(*p)).collect();

    // Crime points (lat,lon,felony) → ENU snapped to a graph node once (parallel), so a BG's
    // walkshed can count the crime reachable within it (same footprint as camera exposure).
    let crime_raw: Vec<(f64, f64, bool)> = std::fs::read_to_string(crime_path)
        .with_context(|| format!("reading {crime_path}"))?
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(',').collect();
            let (Ok(lat), Ok(lon)) = (
                f.first().unwrap_or(&"").trim().parse::<f64>(),
                f.get(1).unwrap_or(&"").trim().parse::<f64>(),
            ) else {
                return None; // header / malformed
            };
            let felony = f.get(2).map(|s| s.trim() == "1").unwrap_or(false);
            Some((lat, lon, felony))
        })
        .collect();
    struct Crime {
        enu: Enu,
        node: u32,
        felony: bool,
    }
    let crimes: Vec<Crime> = crime_raw
        .par_iter()
        .filter_map(|&(lat, lon, felony)| {
            let enu = proj.to_enu(lat, lon);
            graph.snap_nearest(enu).map(|node| Crime { enu, node, felony })
        })
        .collect();
    let crime_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        crimes.iter().enumerate().map(|(i, c)| GeomWithData::new([c.enu.x, c.enu.y], i)).collect(),
    );

    // Optional 311 public-disorder points (lat,lon) — the second justification-channel proxy
    // (broken-windows / disorder-policing demand). Snapped + counted like crime.
    let req311: Vec<(Enu, u32)> = match req311_path {
        Some(p) => std::fs::read_to_string(p)
            .with_context(|| format!("reading {p}"))?
            .lines()
            .filter_map(|line| {
                let f: Vec<&str> = line.split(',').collect();
                let (Ok(lat), Ok(lon)) = (
                    f.first().unwrap_or(&"").trim().parse::<f64>(),
                    f.get(1).unwrap_or(&"").trim().parse::<f64>(),
                ) else {
                    return None;
                };
                Some(proj.to_enu(lat, lon))
            })
            .collect::<Vec<_>>()
            .par_iter()
            .filter_map(|&enu| graph.snap_nearest(enu).map(|node| (enu, node)))
            .collect(),
        None => Vec::new(),
    };
    let req311_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        req311.iter().enumerate().map(|(i, (e, _))| GeomWithData::new([e.x, e.y], i)).collect(),
    );
    eprintln!(
        "  covariates: {} BGs, {} crime points ({} snapped), {} 311 points, {} stations",
        bgs.len(), crime_raw.len(), crimes.len(), req311.len(), station_enu.len()
    );

    let mut rows: Vec<(String, String)> = bgs
        .par_iter()
        .map(|b| {
            let ws = graph.walkshed(b.node, max_seconds, DEFAULT_WALK_SPEED_MPS);
            // Reachable residents + jobs (nodes of BG centroids inside the walkshed).
            let (mut pop_wsh, mut jobs_wsh) = (0.0f64, 0.0f64);
            for cand in bg_tree.locate_within_distance([b.enu.x, b.enu.y], reach_r2) {
                let j = &bgs[cand.data];
                if ws.node_time.contains_key(&j.node) {
                    pop_wsh += j.pop;
                    jobs_wsh += j.jobs;
                }
            }
            // Reachable crime (points whose snapped node is inside the walkshed).
            let (mut crime_wsh, mut felony_wsh) = (0.0f64, 0.0f64);
            for cand in crime_tree.locate_within_distance([b.enu.x, b.enu.y], reach_r2) {
                let c = &crimes[cand.data];
                if ws.node_time.contains_key(&c.node) {
                    crime_wsh += 1.0;
                    if c.felony {
                        felony_wsh += 1.0;
                    }
                }
            }
            // Reachable 311 disorder complaints.
            let mut req311_wsh = 0.0f64;
            for cand in req311_tree.locate_within_distance([b.enu.x, b.enu.y], reach_r2) {
                if ws.node_time.contains_key(&req311[cand.data].1) {
                    req311_wsh += 1.0;
                }
            }
            // Transit access. `transit_dist_m` is deliberately the nearest-station
            // STRAIGHT-LINE distance (a simple access proxy; stations are points, not graph
            // nodes, so a network distance would itself be a snap approximation — disclose,
            // don't fake precision). `stations_wsh`, by contrast, is a walkshed count like
            // every other _wsh covariate: a station counts only if its snapped street node is
            // actually reachable in the 10-minute walk. The pre-2026-07-14 Euclidean disk
            // diverged from the outcome's geography exactly where the network is most
            // constrained (waterfront/highway-severed BGs).
            let transit_dist = station_tree
                .nearest_neighbor(&[b.enu.x, b.enu.y])
                .map(|n| b.enu.distance(Enu::new(n.geom()[0], n.geom()[1])))
                .unwrap_or(1.0e6);
            let stations_wsh = station_tree
                .locate_within_distance([b.enu.x, b.enu.y], reach_r2)
                .filter(|cand| {
                    station_nodes[cand.data].is_some_and(|n| ws.node_time.contains_key(&n))
                })
                .count();
            let row = format!(
                "{},{:.6},{:.6},{:.0},{:.0},{:.0},{crime_wsh:.0},{felony_wsh:.0},{transit_dist:.0},{stations_wsh},{req311_wsh:.0}",
                b.id, b.lat, b.lon, b.pop, pop_wsh, jobs_wsh,
            );
            (b.id.clone(), row)
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::from(
        "id,lat,lon,population,pop_wsh,jobs_wsh,crime_wsh,felony_wsh,transit_dist_m,stations_wsh,req311_wsh\n",
    );
    for (_, r) in &rows {
        out.push_str(r);
        out.push('\n');
    }
    std::fs::write(out_path, &out).with_context(|| format!("writing {out_path}"))?;
    eprintln!("covariates: {} BGs -> {out_path}", rows.len());
    Ok(())
}

/// Distinct road-facing camera groups (ALPR + DOT traffic + photo-enforcement) that capture
/// any point along a drive route. The polyline is densified to ≤`STEP` m so a camera sitting
/// between sparse CSCL vertices isn't missed; each physical-camera *group* is counted once
/// (recall-corrected only for the rare CCTV-only group — road cameras are surveyed/confirmed).
/// Generic pedestrian-FOV CCTV is intentionally excluded from the drive leg (a driver has
/// minimal pedestrian-FOV dwell; plan §2). Returns (corrected_total, alpr, dot, enforcement).
fn route_road_cameras(
    pts: &[Enu],
    sensors: &[SensorInstance],
    cam_tree: &RTree<GeomWithData<[f64; 2], usize>>,
    occ: &sim_core::OccluderIndex,
    recall: f64,
) -> (f64, f64, f64, f64) {
    use sim_core::SourceKind;
    use std::collections::HashMap;
    // The one shared exposure stride (10 m): route legs and walksheds must sample street
    // geometry identically or the place-vs-trajectory comparison inherits a sampling bias.
    const STEP: f64 = sim_core::EXPOSURE_SAMPLE_STRIDE_M;
    // Generous cull: beyond this no fixed street camera can capture the sample point.
    const CULL_R2: f64 = 350.0 * 350.0;

    // Densify the route into ≤STEP-spaced sample points (segment interiors + the final vertex).
    let mut samples: Vec<Enu> = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = ((a.distance(b) / STEP).ceil() as usize).max(1);
        for k in 0..n {
            let t = k as f64 / n as f64;
            samples.push(Enu::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
        }
    }
    if let Some(last) = pts.last() {
        samples.push(*last);
    }

    let mut seen: HashMap<u32, (bool, SourceKind)> = HashMap::new();
    for p in &samples {
        for g in cam_tree.locate_within_distance([p.x, p.y], CULL_R2) {
            let s = &sensors[g.data];
            if matches!(
                s.kind,
                SourceKind::Alpr | SourceKind::DotLiveView | SourceKind::EnforcementCamera
            ) && !seen.contains_key(&s.group)
                && s.wedge.covers_unoccluded(*p)
                && !occ.blocked(s.wedge.apex, *p, s.host_poly)
            {
                seen.insert(s.group, (s.confirmed, s.kind));
            }
        }
    }

    let (mut tot, mut alpr, mut dot, mut enf) = (0.0, 0.0, 0.0, 0.0);
    for (confirmed, kind) in seen.values() {
        let w = if *confirmed { 1.0 } else { recall };
        tot += w;
        match kind {
            SourceKind::Alpr => alpr += w,
            SourceKind::DotLiveView => dot += w,
            SourceKind::EnforcementCamera => enf += w,
            _ => {}
        }
    }
    (tot, alpr, dot, enf)
}

/// Drive-mode activity-space exposure (`A_i` backbone, drive leg) from LODES home→work flows.
/// For each home block group *i*, route its top-`top_k` work destinations *j* (by job flow)
/// over the drive graph, count the distinct road-facing cameras (ALPR + DOT + enforcement)
/// capturing the *i→j* path, add the destination-*j* walkshed exposure (all fixed cameras —
/// the park-and-walk-to-work leg; this equals R_j so it's cached once per work-BG node), and
/// flow-weight over *j*. Emits per-home-BG mean exposure per commuter. This is the drive-only
/// tier of the plan's mode-specific `A_i` (docs/surveillance-exposure-disparity-plan.md §2/§3);
/// transit itineraries + income-heterogeneous mode-choice weighting layer on top later.
fn od_exposure(
    graph_path: &str,
    walk_path: &str,
    centroids_path: &str,
    od_path: &str,
    out_path: &str,
    walk_min: f64,
    top_k: usize,
) -> Result<()> {
    use sim_core::{EnuProjection, StreetGraph, DEFAULT_WALK_SPEED_MPS};
    use std::collections::HashMap;

    // Two networks, deliberately: `graph` is the **drive** graph (the commute route is driven)
    // and `walk` is the **pedestrian** graph (every walkshed and every access/egress leg is
    // walked). Routing a walkshed on the drive graph floods 724 km of limited-access highway no
    // pedestrian may use, and cannot reach 564 km of park path / boardwalk they do — see
    // `graph_osm::CsclNetwork`.
    let graph = StreetGraph::from_asset(
        GraphAsset::from_bytes(&read(graph_path)?).context("decoding drive graph")?,
    );
    let walk = StreetGraph::from_asset(
        GraphAsset::from_bytes(&read(walk_path)?).context("decoding walk graph")?,
    );
    let (sensors, recall, occ) = load_fixed_sensors()?;
    let fov = sim_core::FovModel::from_env();
    eprintln!("  census FOV: {}", fov.describe());
    let mob = MobileLayers::load()?;
    let proj = EnuProjection::default();
    let max_seconds = walk_min * 60.0;
    let cull_r2 = (max_seconds * DEFAULT_WALK_SPEED_MPS + 300.0).powi(2);
    let cam_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        sensors
            .iter()
            .enumerate()
            .map(|(i, s)| GeomWithData::new([s.wedge.apex.x, s.wedge.apex.y], i))
            .collect(),
    );

    // 1. Block-group centroids → snap each to a drive-graph node once (skip unroutable/off-graph).
    struct Bg {
        lat: f64,
        lon: f64,
        enu: Enu,
        /// Node in the **drive** graph (commute routing).
        node: u32,
        /// Node in the **pedestrian** graph (walksheds, access/egress legs). A different index
        /// space from `node` — the two graphs have different node sets, so mixing them up would
        /// silently walk from the wrong place.
        wnode: u32,
    }
    let mut bg: HashMap<String, Bg> = HashMap::new();
    for line in std::fs::read_to_string(centroids_path)
        .with_context(|| format!("reading {centroids_path}"))?
        .lines()
    {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(id), Ok(lat), Ok(lon)) = (
            f.first().map(|s| s.trim()),
            f.get(1).unwrap_or(&"").trim().parse::<f64>(),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) else {
            continue;
        };
        let enu = proj.to_enu(lat, lon);
        if let (Some(node), Some(wnode)) = (graph.snap_nearest(enu), walk.snap_nearest(enu)) {
            bg.insert(id.to_string(), Bg { lat, lon, enu, node, wnode });
        }
    }

    // 2. LODES OD → home_bg → Vec<(work_bg, jobs)>.
    let mut od: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for line in std::fs::read_to_string(od_path)
        .with_context(|| format!("reading {od_path}"))?
        .lines()
    {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(h), Some(w), Ok(jobs)) = (
            f.first().map(|s| s.trim()),
            f.get(1).map(|s| s.trim()),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) else {
            continue; // header row (jobs = "jobs") and malformed lines
        };
        od.entry(h.to_string()).or_default().push((w.to_string(), jobs));
    }

    // 3. Destination-walkshed exposure cache (per work-BG node = R_j; distinct nodes ≤ #BGs).
    let mut dest_cache: HashMap<u32, f64> = HashMap::new();

    let mut out = String::from(
        "home_bg,lat,lon,snap_m,n_dest,jobs_covered,jobs_total,\
         A_drive,route_cams,dest_cams,route_alpr,route_dot,route_enf,m_ace_act,m_dash_act\n",
    );
    let mut home_ids: Vec<&String> = od.keys().collect();
    home_ids.sort(); // deterministic output order
    let (mut done, mut skipped, mut no_path, mut routed) = (0usize, 0usize, 0u64, 0u64);

    for hid in home_ids {
        let Some(home) = bg.get(hid) else {
            skipped += 1;
            continue;
        };
        let mut dests = od[hid].clone();
        dests.sort_by(|a, b| b.1.total_cmp(&a.1)); // most job flow first
        let jobs_total: f64 = dests.iter().map(|(_, j)| j).sum();
        dests.truncate(top_k);

        let (mut w_route, mut w_dest) = (0.0f64, 0.0f64);
        let (mut w_alpr, mut w_dot, mut w_enf) = (0.0f64, 0.0f64, 0.0f64);
        let (mut w_ace, mut w_dash) = (0.0f64, 0.0f64);
        let (mut jobs_cov, mut n_dest) = (0.0f64, 0usize);
        for (wid, jobs) in &dests {
            let Some(work) = bg.get(wid) else { continue };
            let Ok((route, _t, _edges)) = graph.route_timed_pen(home.node, work.node, 1.0) else {
                no_path += 1;
                continue;
            };
            routed += 1;
            let (rc, ra, rd, re) = route_road_cameras(&route.points, &sensors, &cam_tree, &occ, recall);
            // M_i^act mobile terms: expected encounters for one traversal of the driven leg.
            if mob.mobile.ace.is_some() || mob.dashcam_on {
                let (ma, md) = route_mobile_exposure(&route.points, &mob, &occ, recall);
                w_ace += jobs * ma;
                w_dash += jobs * md;
            }
            let de = *dest_cache.entry(work.wnode).or_insert_with(|| {
                let ws = walk.walkshed(work.wnode, max_seconds, DEFAULT_WALK_SPEED_MPS);
                let wp = walk.node_pos(work.wnode);
                let nearby: Vec<SensorInstance> = cam_tree
                    .locate_within_distance([wp.x, wp.y], cull_r2)
                    .map(|g| sensors[g.data])
                    .collect();
                sim_core::walkshed_exposure_with(&walk, &ws, &nearby, &occ, recall, &fov, None)
                    .cameras_corrected
                    .max(0.0)
            });
            w_route += jobs * rc;
            w_dest += jobs * de;
            w_alpr += jobs * ra;
            w_dot += jobs * rd;
            w_enf += jobs * re;
            jobs_cov += jobs;
            n_dest += 1;
        }
        if jobs_cov <= 0.0 {
            skipped += 1;
            continue;
        }
        // Flow-weighted mean exposure per commuter from this home BG.
        let (rcm, dcm) = (w_route / jobs_cov, w_dest / jobs_cov);
        let snap_m = home.enu.distance(graph.node_pos(home.node));
        out.push_str(&format!(
            "{hid},{:.6},{:.6},{snap_m:.1},{n_dest},{jobs_cov:.0},{jobs_total:.0},\
             {:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.4},{:.4}\n",
            home.lat,
            home.lon,
            rcm + dcm,
            rcm,
            dcm,
            w_alpr / jobs_cov,
            w_dot / jobs_cov,
            w_enf / jobs_cov,
            w_ace / jobs_cov,
            w_dash / jobs_cov,
        ));
        done += 1;
    }
    std::fs::write(out_path, out).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "od-exposure: {done} home BGs ({skipped} skipped), top-{top_k} dests, \
         {routed} pairs routed ({no_path} no-path), {} dest walksheds cached -> {out_path}",
        dest_cache.len()
    );
    Ok(())
}

/// Expected devices/min from the observed mobile classes along a routed leg, day-averaged
/// over `day_weights()` and integrated over the leg's *traversal time* — i.e. an expected
/// encounter count for one traversal at the mean daily rate, not a per-minute rate. The
/// commute happens at ~8–9 AM in reality, but hour-folding over the representative day
/// keeps M_i^act comparable with M_i^res (same averaging convention); the AM-peak-only
/// variant is one env knob away (`OURSPACE_COMMUTE_HOUR`).
///
/// Returns (ace_encounters, dashcam_encounters) for one traversal of the polyline.
fn route_mobile_exposure(
    pts: &[Enu],
    mob: &MobileLayers,
    occ: &sim_core::OccluderIndex,
    recall: f64,
) -> (f64, f64) {
    const STEP: f64 = 50.0; // smooth zone fields; no need for the fixed-camera 10 m stride

    let mut samples: Vec<Enu> = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = ((a.distance(b) / STEP).ceil() as usize).max(1);
        for k in 0..n {
            let t = k as f64 / n as f64;
            samples.push(Enu::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
        }
    }
    if let Some(last) = pts.last() {
        samples.push(*last);
    }
    if samples.is_empty() {
        return (0.0, 0.0);
    }

    let weights = day_weights();
    let wsum: f64 = weights.iter().map(|(_, w)| w).sum();
    // Traversal time per sample: assume the drive-graph free-flow pace is roughly uniform
    // across the leg, so each sample "costs" total_time / n_samples minutes. Total time is
    // NOT known here (callers have it); we instead integrate rate × (leg_m / speed). NYC
    // average bus+car street speed ≈ 5 m/s (11 mph door-to-door including lights) — the
    // same constant the heatmap's per-edge rates implicitly assume for midpoints.
    const STREET_SPEED_MPS: f64 = 5.0;
    let min_per_sample = STEP / STREET_SPEED_MPS / 60.0;

    let mut acc = [0.0_f64; 2];
    for p in &samples {
        let near_ace = mob
            .ace_tree
            .as_ref()
            .is_some_and(|t| t.locate_within_distance([p.x, p.y], mob.ace_cap_r2).next().is_some());
        for &(h, w) in &weights {
            let r = sim_core::exposure_rates_per_minute(
                sim_core::Vec2::new(p.x, p.y),
                h,
                &[],
                occ,
                near_ace,
                &mob.mobile,
                recall,
                mob.dashcam.as_ref(),
                None,
                None,
                None,
            );
            acc[0] += r.ace * w;
            acc[1] += r.dashcam * w;
        }
    }
    let scale = min_per_sample / wsum;
    (acc[0] * scale, acc[1] * scale)
}

/// Fixed-camera exposure along a route, split by traveller mode. Always tallies distinct
/// road-facing groups (ALPR + DOT + enforcement) — a driver's exposure. If `pedestrian`, also
/// tallies ALL distinct fixed groups (adds street CCTV) — a walker's exposure along the same
/// street path. Each physical-camera group is counted once (recall-corrected for CCTV-only
/// groups). Returns (drive_leg, walk_leg); walk_leg is 0.0 when `pedestrian` is false.
fn route_leg_exposure(
    pts: &[Enu],
    sensors: &[SensorInstance],
    cam_tree: &RTree<GeomWithData<[f64; 2], usize>>,
    occ: &sim_core::OccluderIndex,
    recall: f64,
    pedestrian: bool,
    fov: &sim_core::FovModel,
    // (home-walkshed groups, destination-walkshed groups) to exclude — pass empty
    // sets for legs that should count everything (e.g. standalone probes).
    exclude: (&std::collections::HashSet<u32>, &std::collections::HashSet<u32>),
) -> (f64, f64) {
    use sim_core::SourceKind;
    use std::collections::HashMap;
    const STEP: f64 = sim_core::EXPOSURE_SAMPLE_STRIDE_M; // shared with walksheds
    const CULL_R2: f64 = 350.0 * 350.0;

    let mut samples: Vec<Enu> = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = ((a.distance(b) / STEP).ceil() as usize).max(1);
        for k in 0..n {
            let t = k as f64 / n as f64;
            samples.push(Enu::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
        }
    }
    if let Some(last) = pts.last() {
        samples.push(*last);
    }
    // group id -> accumulator (certainty + heading bearings for census cameras).
    struct Acc {
        confirmed: bool,
        is_road: bool,
        certain: bool,
        bearings: Vec<f64>,
    }
    let mut seen: HashMap<u32, Acc> = HashMap::new();
    for p in &samples {
        for g in cam_tree.locate_within_distance([p.x, p.y], CULL_R2) {
            let s = &sensors[g.data];
            let is_road = matches!(
                s.kind,
                SourceKind::Alpr | SourceKind::DotLiveView | SourceKind::EnforcementCamera
            );
            if !pedestrian && !is_road {
                continue; // driver: only road cameras matter, skip the FOV test for CCTV
            }
            // Cameras already covering the home/destination walkshed are not NEW
            // exposure on this leg: A = commute + destination walkshed has union
            // semantics, and this is where the (mode-correlated) double count died.
            if exclude.0.contains(&s.group) || exclude.1.contains(&s.group) {
                continue;
            }
            if seen.get(&s.group).is_some_and(|a| a.certain) {
                continue;
            }
            if s.wedge.covers_unoccluded(*p) && !occ.blocked(s.wedge.apex, *p, s.host_poly) {
                let apex = s.wedge.apex;
                let acc = seen.entry(s.group).or_insert(Acc {
                    confirmed: s.confirmed,
                    is_road,
                    certain: false,
                    bearings: Vec::new(),
                });
                acc.is_road |= is_road;
                if fov.enabled && s.kind == SourceKind::FixedCctv {
                    acc.bearings.push((p.y - apex.y).atan2(p.x - apex.x));
                } else {
                    acc.certain = true;
                }
            }
        }
    }
    let (mut drive, mut walk) = (0.0, 0.0);
    for a in seen.values() {
        let mut w = if a.certain { 1.0 } else { fov.weight_for(&a.bearings) };
        if !a.confirmed {
            w *= recall;
        }
        if pedestrian {
            walk += w;
        }
        if a.is_road {
            drive += w;
        }
    }
    (drive, walk)
}

/// Mode-weighted drive/transit/walk activity-space exposure (`A_i`, plan §2/§3b). Extends
/// `od-exposure` with the mode split: each commute leg gets mode-appropriate exposure —
///   drive   → road cameras (ALPR/DOT/enforcement) along the i→j drive route,
///   transit → street cameras on the access walk (home→nearest station) + egress walk
///             (dest station→work); the underground line-haul is invisible to the street-level
///             census, so no RAPTOR is needed for exposure,
///   walk    → all street cameras along the (short) i→j walk,
/// plus the mode-independent destination-BG walkshed. Weighted by **observed ACS commute-mode
/// shares** (B08301) per home BG, with the walk share reassigned to drive/transit for trips
/// beyond `WALK_MAX_M`. (Observed shares are already income-patterned; an income-heterogeneous
/// MNL over these times/costs is the planned refinement.) Access/egress/destination terms are
/// cached per BG, so only the i→j drive route is per-pair.
#[allow(clippy::too_many_arguments)]
fn od_exposure_modal(
    graph_path: &str,
    walk_path: &str,
    centroids_path: &str,
    od_path: &str,
    acs_path: &str,
    stations_path: &str,
    out_path: &str,
    walk_min: f64,
    top_k: usize,
    subway_path: Option<&str>,
) -> Result<()> {
    use sim_core::{EnuProjection, StreetGraph, DEFAULT_WALK_SPEED_MPS};
    use std::collections::HashMap;
    const WALK_MAX_M: f64 = 2500.0; // a commute past this isn't walked; walk share reassigned

    // Two networks, deliberately: `graph` is the **drive** graph (the commute route is driven)
    // and `walk` is the **pedestrian** graph (every walkshed and every access/egress leg is
    // walked). Routing a walkshed on the drive graph floods 724 km of limited-access highway no
    // pedestrian may use, and cannot reach 564 km of park path / boardwalk they do — see
    // `graph_osm::CsclNetwork`.
    let graph = StreetGraph::from_asset(
        GraphAsset::from_bytes(&read(graph_path)?).context("decoding drive graph")?,
    );
    let walk = StreetGraph::from_asset(
        GraphAsset::from_bytes(&read(walk_path)?).context("decoding walk graph")?,
    );
    let (sensors, recall, occ) = load_fixed_sensors()?;
    let fov = sim_core::FovModel::from_env();
    eprintln!("  census FOV: {}", fov.describe());
    let proj = EnuProjection::default();
    let max_seconds = walk_min * 60.0;
    let cull_r2 = (max_seconds * DEFAULT_WALK_SPEED_MPS + 300.0).powi(2);
    let cam_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        sensors
            .iter()
            .enumerate()
            .map(|(i, s)| GeomWithData::new([s.wedge.apex.x, s.wedge.apex.y], i))
            .collect(),
    );

    // Subway stations → nearest-station R-tree (ENU) for transit access/egress walks.
    // With a subway matrix the matrix's station list IS the target set (indices must
    // align with its all-pairs tables); the CSV serves the crow-flies fallback.
    let submat = load_subway(subway_path)?;
    let station_enu: Vec<Enu> = match &submat {
        Some(m) => m.stations.iter().map(|s| proj.to_enu(s.lat, s.lon)).collect(),
        None => {
            let mut v = Vec::new();
            for line in std::fs::read_to_string(stations_path)
                .with_context(|| format!("reading {stations_path}"))?
                .lines()
            {
                let f: Vec<&str> = line.split(',').collect();
                if let (Ok(lat), Ok(lon)) = (
                    f.get(1).unwrap_or(&"").trim().parse::<f64>(),
                    f.get(2).unwrap_or(&"").trim().parse::<f64>(),
                ) {
                    v.push(proj.to_enu(lat, lon));
                }
            }
            v
        }
    };
    let station_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        station_enu.iter().enumerate().map(|(i, p)| GeomWithData::new([p.x, p.y], i)).collect(),
    );
    let station_wnode: Vec<Option<u32>> =
        station_enu.iter().map(|p| walk.snap_nearest(*p)).collect();
    eprintln!("  transit: {} subway stations", station_enu.len());

    struct Bg {
        lat: f64,
        lon: f64,
        enu: Enu,
        /// Node in the **drive** graph (commute routing).
        node: u32,
        /// Node in the **pedestrian** graph (walksheds, access/egress legs). A different index
        /// space from `node` — the two graphs have different node sets, so mixing them up would
        /// silently walk from the wrong place.
        wnode: u32,
    }
    let mut bg: HashMap<String, Bg> = HashMap::new();
    for line in std::fs::read_to_string(centroids_path)
        .with_context(|| format!("reading {centroids_path}"))?
        .lines()
    {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(id), Ok(lat), Ok(lon)) = (
            f.first().map(|s| s.trim()),
            f.get(1).unwrap_or(&"").trim().parse::<f64>(),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) else {
            continue;
        };
        let enu = proj.to_enu(lat, lon);
        if let (Some(node), Some(wnode)) = (graph.snap_nearest(enu), walk.snap_nearest(enu)) {
            bg.insert(id.to_string(), Bg { lat, lon, enu, node, wnode });
        }
    }

    let mut od: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for line in std::fs::read_to_string(od_path)
        .with_context(|| format!("reading {od_path}"))?
        .lines()
    {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(h), Some(w), Ok(jobs)) = (
            f.first().map(|s| s.trim()),
            f.get(1).map(|s| s.trim()),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) else {
            continue;
        };
        od.entry(h.to_string()).or_default().push((w.to_string(), jobs));
    }

    let acs = Table::load(acs_path, "id")?;
    // Observed ACS commute-mode shares for a home BG (car / transit / walk), normalized over the
    // three travelling modes; NYC-commuter default when the BG's B08301 is missing/zero.
    let mode_shares = |geoid: &str| -> (f64, f64, f64) {
        let car = fnum(acs.get(geoid, "commute_car")).unwrap_or(0.0);
        let tr = fnum(acs.get(geoid, "commute_transit")).unwrap_or(0.0);
        let wk = fnum(acs.get(geoid, "commute_walk")).unwrap_or(0.0);
        let base = car + tr + wk;
        if base > 0.0 {
            (car / base, tr / base, wk / base)
        } else {
            (0.30, 0.55, 0.15) // citywide-ish fallback for GQ/suppressed BGs
        }
    };
    // Bus share WITHIN the transit mode (ACS B08301 sub-modes; the remainder rides the
    // subway/rail model). Fallback: citywide-ish split when the BG's transit count is 0.
    let bus_share = |geoid: &str| -> f64 {
        let b = fnum(acs.get(geoid, "commute_bus")).unwrap_or(0.0);
        let tr = fnum(acs.get(geoid, "commute_transit")).unwrap_or(0.0);
        if tr > 0.0 { (b / tr).clamp(0.0, 1.0) } else { 0.35 }
    };

    // One walkshed pass over every BG walk-node (each BG is a home AND a work
    // somewhere): the destination-term exposure AND the captured camera-group set —
    // the dedup mask that keeps commute legs from re-counting endpoint coverage.
    let all_wnodes: std::collections::HashSet<u32> = bg.values().map(|b| b.wnode).collect();
    let dest_exp: HashMap<u32, (f64, std::collections::HashSet<u32>)> = all_wnodes
        .par_iter()
        .map(|&node| {
            let ws = walk.walkshed(node, max_seconds, DEFAULT_WALK_SPEED_MPS);
            let wp = walk.node_pos(node);
            let nearby: Vec<SensorInstance> = cam_tree
                .locate_within_distance([wp.x, wp.y], cull_r2)
                .map(|g| sensors[g.data])
                .collect();
            let s = sim_core::walkshed_exposure_with(&walk, &ws, &nearby, &occ, recall, &fov, None);
            (node, (s.cameras_corrected.max(0.0), s.groups.into_iter().collect()))
        })
        .collect();
    let empty_groups: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let groups_of = |wnode: u32| -> &std::collections::HashSet<u32> {
        dest_exp.get(&wnode).map(|v| &v.1).unwrap_or(&empty_groups)
    };
    // Precompute the per-BG caches in parallel. `station_cands`: the BG's candidate access
    // stations, each with its walk-routed access exposure/distance (serves the home access
    // walk and the work egress walk; the router picks the best origin×dest station pair per
    // trip, the fallback uses the single nearest). `dest_exp`: destination-BG walkshed R_j
    // (mode-independent). Computing these once up front makes them read-only, so the per-BG
    // routing below parallelizes cleanly.
    use rayon::prelude::*;
    let k_cands = if submat.is_some() { 3 } else { 1 };
    let station_cands: HashMap<String, Vec<StaCand>> = bg
        .par_iter()
        .map(|(g, b)| {
            let cands = station_candidates(
                b.enu, b.wnode, k_cands, &station_tree, &station_enu, &station_wnode, &walk,
                &sensors, &cam_tree, &occ, recall, &fov, groups_of(b.wnode),
            );
            (g.clone(), cands)
        })
        .collect();



    // Fan the per-home-BG routing out across cores — each BG is independent and the caches are now
    // read-only, so results are deterministic; rows are re-sorted by GEOID before writing.
    let no_path = std::sync::atomic::AtomicU64::new(0);
    let routed = std::sync::atomic::AtomicU64::new(0);
    let subway = SubwayParams::from_env();
    let bus = BusParams::from_env();
    // M1 mobile layers: shared read-only state for the parallel routing pass below
    // (RTree and field layer are immutably queried; MobileScenario is Copy-config).
    let mob = std::sync::Arc::new(MobileLayers::load()?);
    eprintln!("  transit bus sub-mode: [{}]", bus.describe());
    let circuity = subway_circuity();
    match &submat {
        Some(_) => eprintln!("  transit: MTA subway cameras/trip [{}]", subway.describe()),
        None => eprintln!(
            "  transit: MTA subway cameras/trip [{}], line-haul circuity ×{circuity:.2}",
            subway.describe()
        ),
    }
    let mut home_ids: Vec<&String> = od.keys().collect();
    home_ids.sort();
    let mut rows: Vec<(String, String)> = home_ids
        .par_iter()
        .filter_map(|&hid| {
            let home = bg.get(hid)?;
            let (sd, st, sw) = mode_shares(hid);
            let home_cands = station_cands.get(hid).map(Vec::as_slice).unwrap_or(&[]);
            let home_groups = groups_of(home.wnode);
            let sub_bus = bus_share(hid);
            let mut dests = od[hid].clone();
            dests.sort_by(|a, b| b.1.total_cmp(&a.1));
            let jobs_total: f64 = dests.iter().map(|(_, j)| j).sum();
            dests.truncate(top_k);

            let (mut w_dest, mut w_commute) = (0.0f64, 0.0f64);
            let (mut w_drive, mut w_transit, mut w_walk, mut w_subway) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let (mut w_pd, mut w_pt, mut w_pw) = (0.0f64, 0.0f64, 0.0f64);
            let (mut w_ace, mut w_dash) = (0.0f64, 0.0f64);
            let (mut jobs_cov, mut n_dest) = (0.0f64, 0usize);
            // Mobile layers behind a shared Arc: clone the handle into the closure's scope.
            let mob = mob.clone();

            for (wid, jobs) in &dests {
                let Some(work) = bg.get(wid) else { continue };
                let Ok((route, _t, _e)) = graph.route_timed_pen(home.node, work.node, 1.0) else {
                    no_path.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                };
                routed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // Drive-leg cameras along the driven route (road-facing kinds only),
                // net of cameras already covering the home/destination walksheds.
                let dest_groups = groups_of(work.wnode);
                let (drive_leg, _) = route_leg_exposure(
                    &route.points, &sensors, &cam_tree, &occ, recall, false, &fov,
                    (home_groups, dest_groups),
                );
                // M_i^act mobile terms for the driven leg (expected encounters per traversal).
                let (m_ace, m_dash) = if mob.mobile.ace.is_some() || mob.dashcam_on {
                    route_mobile_exposure(&route.points, &mob, &occ, recall)
                } else {
                    (0.0, 0.0)
                };
                // The WALK share walks the WALK graph (see od_exposure_mnl for why routing a
                // pedestrian on the drive geometry is wrong). Drive distance pre-gates the A*.
                let (walk_leg, short) = if route.total_m <= WALK_MAX_M * 1.5 {
                    match walk.route(home.wnode, work.wnode) {
                        Ok(wr) if wr.total_m <= WALK_MAX_M => (
                            route_leg_exposure(
                                &wr.points, &sensors, &cam_tree, &occ, recall, true, &fov,
                                (home_groups, dest_groups),
                            )
                            .1,
                            true,
                        ),
                        _ => (0.0, false),
                    }
                } else {
                    (0.0, false)
                };
                // access/egress street walks + guaranteed MTA station/train cameras. With a
                // subway matrix the itinerary is routed (best candidate station pair; exact
                // boardings set the camera complement); without one, crow-flies × circuity
                // estimates transfers. The underground line-haul is surveilled, not invisible.
                let work_cands = station_cands.get(wid).map(Vec::as_slice).unwrap_or(&[]);
                let leg = best_transit_leg(
                    submat.as_ref(), &subway, circuity, DEFAULT_WALK_SPEED_MPS, home_cands,
                    work_cands,
                );
                // Bus/rail mixture (see od_exposure_mnl): the bus rides the street corridor
                // (drive_leg, deduped) + its onboard complement; ACS sub-shares weight the
                // two, and a rail-infeasible pair is all-bus rather than transit-infeasible.
                let bus_cams = bus.cameras(route.total_m);
                let e_bus = drive_leg + bus_cams;
                let (transit_ok, transit_leg, subway_cams) = match &leg {
                    Some(l) => (
                        true,
                        sub_bus * e_bus + (1.0 - sub_bus) * (l.e_acc + l.e_subway),
                        sub_bus * bus_cams + (1.0 - sub_bus) * l.e_subway,
                    ),
                    None => (true, e_bus, bus_cams),
                };
                let dest_j = dest_exp.get(&work.wnode).map(|v| v.0).unwrap_or(0.0);

                // Trip-distance-gated mode shares: past WALK_MAX_M the walk share is reassigned to
                // drive/transit in proportion (you can't walk that far to work), and an infeasible
                // transit share is reassigned to drive/walk the same way.
                let (pd, pt, pw) = if short {
                    (sd, st, sw)
                } else {
                    let nz = sd + st;
                    if nz > 0.0 {
                        (sd + sw * sd / nz, st + sw * st / nz, 0.0)
                    } else {
                        (1.0, 0.0, 0.0)
                    }
                };
                let (pd, pt, pw) = if transit_ok {
                    (pd, pt, pw)
                } else {
                    let nz = pd + pw;
                    if nz > 0.0 {
                        (pd + pt * pd / nz, 0.0, pw + pt * pw / nz)
                    } else {
                        (1.0, 0.0, 0.0)
                    }
                };
                let commute = pd * drive_leg + pt * transit_leg + pw * walk_leg;
                w_dest += jobs * dest_j;
                w_commute += jobs * commute;
                w_drive += jobs * drive_leg;
                w_transit += jobs * transit_leg;
                w_walk += jobs * walk_leg; // already 0 unless the walk-graph route is short
                w_subway += jobs * pt * subway_cams; // subway contribution to the commute leg
                // Mobile encounters ride the drive leg (both the drive and bus sub-modes
                // traverse the same corridor; transit riders on rail skip it, so weight
                // by (drive share + bus sub-share of transit) — the street-exposed part).
                let street_share = pd + pt * sub_bus;
                w_ace += jobs * m_ace * street_share;
                w_dash += jobs * m_dash * street_share;
                w_pd += jobs * pd;
                w_pt += jobs * pt;
                w_pw += jobs * pw;
                jobs_cov += jobs;
                n_dest += 1;
            }
            if jobs_cov <= 0.0 {
                return None;
            }
            let inv = 1.0 / jobs_cov;
            let (dest, commute) = (w_dest * inv, w_commute * inv);
            let row = format!(
                "{hid},{:.6},{:.6},{n_dest},{jobs_cov:.0},{jobs_total:.0},\
                 {:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.4},{:.4}",
                home.lat, home.lon, dest + commute, dest, commute,
                w_drive * inv, w_transit * inv, w_walk * inv, w_pd * inv, w_pt * inv, w_pw * inv,
                w_subway * inv,
                w_ace * inv,
                w_dash * inv,
            );
            Some((hid.clone(), row))
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    use std::io::Write as _;
    let mut w = std::io::BufWriter::new(
        std::fs::File::create(out_path).with_context(|| format!("creating {out_path}"))?,
    );
    writeln!(
        w,
        "home_bg,lat,lon,n_dest,jobs_covered,jobs_total,\
         A_modal,A_dest,commute_modal,commute_drive,commute_transit,commute_walk,\
         p_drive,p_transit,p_walk,commute_subway,m_ace_act,m_dash_act"
    )?;
    for (_, row) in &rows {
        writeln!(w, "{row}")?;
    }
    w.flush()?;
    eprintln!(
        "od-exposure-modal: {} home BGs, top-{top_k}, {} pairs routed ({} no-path) -> {out_path}",
        rows.len(),
        routed.load(std::sync::atomic::Ordering::Relaxed),
        no_path.load(std::sync::atomic::Ordering::Relaxed)
    );
    Ok(())
}

/// One routed home→work pair with the inputs the mode-choice model needs: the job flow, the
/// per-mode generalized-cost ingredients (times in minutes, monetary costs in $), and the
/// per-mode commute-leg exposure + the mode-independent destination exposure.
struct ModePair {
    flow: f64,
    t_drive: f64,
    t_walk: f64,
    t_transit: f64,
    c_drive: f64,
    c_transit: f64,
    e_drive: f64,
    e_transit: f64,
    e_subway: f64, // MTA-system portion of e_transit (for the commute_subway sensitivity column)
    e_walk: f64,
    dest: f64,
    /// M_i^act mobile terms for the driven leg: expected (ACE, dashcam) encounters per
    /// traversal, day-averaged. Weighted by street-exposed mode share at aggregation.
    m_ace: f64,
    m_dash: f64,
    /// Destination BG (for M3 pair emission; empty in the modal command's variant).
    work_geoid: String,
    /// Walkable commute: the WALK-graph route exists and is ≤ WALK_MAX_M. Gates the
    /// observed-shares mode model for income-suppressed homes (the MNL gates itself
    /// through t_walk).
    short: bool,
    /// Transit is a real alternative: both ends have a routable station walk and the two
    /// nearest stations differ (a same-station "trip" would ride nowhere yet book full
    /// station+train cameras).
    transit_ok: bool,
}
struct HomeData {
    geoid: String,
    lat: f64,
    lon: f64,
    income: f64,
    vot: f64,     // value of time, $/min (income-derived)
    group: usize, // income quintile 0..5, set after all incomes are known
    /// ACS median income exists for this BG. When it does not (~15%, non-random), the home
    /// skips the MNL (no fabricated VOT/quintile) and uses `shares` instead; it is also
    /// excluded from the quintile edges and the ASC calibration.
    has_income: bool,
    /// Observed ACS (car, transit, walk) commute shares — the mode model for
    /// income-suppressed homes.
    shares: (f64, f64, f64),
    jobs_total: f64,
    pairs: Vec<ModePair>,
}

/// Multinomial-logit mode probabilities over {walk, drive, transit} for one pair, given a
/// home's value of time and the current alternative-specific constants (walk = reference, 0).
/// Utility = −(VOT·time + cost) + ASC (utilities in dollars, so ASC absorbs everything the
/// generalized cost misses). Returns (p_walk, p_drive, p_transit).
fn mode_probs(p: &ModePair, vot: f64, asc_drive: f64, asc_transit: f64) -> (f64, f64, f64) {
    let u_walk = -(vot * p.t_walk);
    let u_drive = -(vot * p.t_drive + p.c_drive) + asc_drive;
    let u_transit = -(vot * p.t_transit + p.c_transit) + asc_transit;
    let m = u_walk.max(u_drive).max(u_transit);
    let (ew, ed, et) = ((u_walk - m).exp(), (u_drive - m).exp(), (u_transit - m).exp());
    let s = ew + ed + et;
    (ew / s, ed / s, et / s)
}

/// Income-heterogeneous **mode-choice** variant of `od-exposure-modal`. Instead of observed ACS
/// shares, mode is a multinomial logit over {walk, drive, transit} with income-based value of
/// time (VOT ∝ ACS median household income) and alternative-specific constants calibrated to
/// reproduce the citywide ACS B08301 commute-mode marginals. So mode varies by trip distance
/// (from the routed times) *and* income (poorer → lower VOT → transit's slowness matters less →
/// more transit; richer → drive) endogenously. Routes every pair once (parallel, storing the
/// mode inputs), calibrates the two ASCs cheaply, then aggregates. Same output schema as
/// `od-exposure-modal` (A_modal→A_mnl semantics).
#[allow(clippy::too_many_arguments)]
fn od_exposure_mnl(
    graph_path: &str,
    walk_path: &str,
    centroids_path: &str,
    od_path: &str,
    acs_path: &str,
    stations_path: &str,
    out_path: &str,
    walk_min: f64,
    top_k: usize,
    subway_path: Option<&str>,
) -> Result<()> {
    use rayon::prelude::*;
    use sim_core::{EnuProjection, StreetGraph, DEFAULT_WALK_SPEED_MPS};
    use std::collections::HashMap;
    use std::io::Write as _;
    const WALK_MAX_M: f64 = 2500.0;
    const FARE: f64 = 2.90;
    const DRIVE_COST_PER_KM: f64 = 0.20; // marginal fuel + wear
    const DRIVE_FIXED_COST: f64 = 4.0; // averaged parking/toll per commute
    const ANNUAL_WORK_MIN: f64 = 2080.0 * 60.0;
    const VOT_WAGE_FRAC: f64 = 0.5;
    const DEFAULT_INCOME: f64 = 70000.0;

    // Two networks, deliberately: `graph` is the **drive** graph (the commute route is driven)
    // and `walk` is the **pedestrian** graph (every walkshed and every access/egress leg is
    // walked). Routing a walkshed on the drive graph floods 724 km of limited-access highway no
    // pedestrian may use, and cannot reach 564 km of park path / boardwalk they do — see
    // `graph_osm::CsclNetwork`.
    let graph = StreetGraph::from_asset(
        GraphAsset::from_bytes(&read(graph_path)?).context("decoding drive graph")?,
    );
    let walk = StreetGraph::from_asset(
        GraphAsset::from_bytes(&read(walk_path)?).context("decoding walk graph")?,
    );
    let (sensors, recall, occ) = load_fixed_sensors()?;
    let fov = sim_core::FovModel::from_env();
    eprintln!("  census FOV: {}", fov.describe());
    let proj = EnuProjection::default();
    let max_seconds = walk_min * 60.0;
    let cull_r2 = (max_seconds * DEFAULT_WALK_SPEED_MPS + 300.0).powi(2);
    let cam_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        sensors
            .iter()
            .enumerate()
            .map(|(i, s)| GeomWithData::new([s.wedge.apex.x, s.wedge.apex.y], i))
            .collect(),
    );

    // With a subway matrix the matrix's station list IS the access/egress target set
    // (indices must align with its all-pairs tables); the CSV serves the fallback.
    let submat = load_subway(subway_path)?;
    let station_enu: Vec<Enu> = match &submat {
        Some(m) => m.stations.iter().map(|s| proj.to_enu(s.lat, s.lon)).collect(),
        None => {
            let mut v = Vec::new();
            for line in std::fs::read_to_string(stations_path)
                .with_context(|| format!("reading {stations_path}"))?
                .lines()
            {
                let f: Vec<&str> = line.split(',').collect();
                if let (Ok(lat), Ok(lon)) = (
                    f.get(1).unwrap_or(&"").trim().parse::<f64>(),
                    f.get(2).unwrap_or(&"").trim().parse::<f64>(),
                ) {
                    v.push(proj.to_enu(lat, lon));
                }
            }
            v
        }
    };
    let station_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        station_enu.iter().enumerate().map(|(i, p)| GeomWithData::new([p.x, p.y], i)).collect(),
    );
    let station_wnode: Vec<Option<u32>> =
        station_enu.iter().map(|p| walk.snap_nearest(*p)).collect();
    eprintln!("  transit: {} subway stations", station_enu.len());

    struct Bg {
        lat: f64,
        lon: f64,
        enu: Enu,
        /// Node in the **drive** graph (commute routing).
        node: u32,
        /// Node in the **pedestrian** graph (walksheds, access/egress legs). A different index
        /// space from `node` — the two graphs have different node sets, so mixing them up would
        /// silently walk from the wrong place.
        wnode: u32,
    }
    let mut bg: HashMap<String, Bg> = HashMap::new();
    for line in std::fs::read_to_string(centroids_path)
        .with_context(|| format!("reading {centroids_path}"))?
        .lines()
    {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(id), Ok(lat), Ok(lon)) = (
            f.first().map(|s| s.trim()),
            f.get(1).unwrap_or(&"").trim().parse::<f64>(),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) else {
            continue;
        };
        let enu = proj.to_enu(lat, lon);
        if let (Some(node), Some(wnode)) = (graph.snap_nearest(enu), walk.snap_nearest(enu)) {
            bg.insert(id.to_string(), Bg { lat, lon, enu, node, wnode });
        }
    }

    let mut od: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for line in std::fs::read_to_string(od_path)
        .with_context(|| format!("reading {od_path}"))?
        .lines()
    {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(h), Some(w), Ok(jobs)) = (
            f.first().map(|s| s.trim()),
            f.get(1).map(|s| s.trim()),
            f.get(2).unwrap_or(&"").trim().parse::<f64>(),
        ) else {
            continue;
        };
        od.entry(h.to_string()).or_default().push((w.to_string(), jobs));
    }

    let acs = Table::load(acs_path, "id")?;
    // Observed ACS commute-mode shares (car / transit / walk) for a home BG. Used as the
    // mode model for INCOME-SUPPRESSED home BGs: ~15% of BGs have no ACS median income, and
    // they are non-random (small, group-quarters, public housing), so imputing a middle
    // income (the old $70k fallback) mis-assigned their VOT, pinned a quintile edge to the
    // fallback value, and contaminated the ASC calibration targets. Observed behavior needs
    // no income.
    let mode_shares = |geoid: &str| -> (f64, f64, f64) {
        let car = fnum(acs.get(geoid, "commute_car")).unwrap_or(0.0);
        let tr = fnum(acs.get(geoid, "commute_transit")).unwrap_or(0.0);
        let wk = fnum(acs.get(geoid, "commute_walk")).unwrap_or(0.0);
        let base = car + tr + wk;
        if base > 0.0 {
            (car / base, tr / base, wk / base)
        } else {
            (0.30, 0.55, 0.15) // citywide-ish fallback for GQ/suppressed BGs
        }
    };
    // Bus share WITHIN the transit mode (ACS B08301 sub-modes; the remainder rides the
    // subway/rail model). Fallback: citywide-ish split when the BG's transit count is 0.
    let bus_share = |geoid: &str| -> f64 {
        let b = fnum(acs.get(geoid, "commute_bus")).unwrap_or(0.0);
        let tr = fnum(acs.get(geoid, "commute_transit")).unwrap_or(0.0);
        if tr > 0.0 { (b / tr).clamp(0.0, 1.0) } else { 0.35 }
    };
    // One walkshed pass over every BG walk-node (each BG is a home AND a work
    // somewhere): the destination-term exposure AND the captured camera-group set —
    // the dedup mask that keeps commute legs from re-counting endpoint coverage.
    let all_wnodes: std::collections::HashSet<u32> = bg.values().map(|b| b.wnode).collect();
    let dest_exp: HashMap<u32, (f64, std::collections::HashSet<u32>)> = all_wnodes
        .par_iter()
        .map(|&node| {
            let ws = walk.walkshed(node, max_seconds, DEFAULT_WALK_SPEED_MPS);
            let wp = walk.node_pos(node);
            let nearby: Vec<SensorInstance> = cam_tree
                .locate_within_distance([wp.x, wp.y], cull_r2)
                .map(|g| sensors[g.data])
                .collect();
            let s = sim_core::walkshed_exposure_with(&walk, &ws, &nearby, &occ, recall, &fov, None);
            (node, (s.cameras_corrected.max(0.0), s.groups.into_iter().collect()))
        })
        .collect();
    let empty_groups: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let groups_of = |wnode: u32| -> &std::collections::HashSet<u32> {
        dest_exp.get(&wnode).map(|v| &v.1).unwrap_or(&empty_groups)
    };
    // Per-BG candidate access stations (parallel), each with its walk-routed access
    // exposure and distance. Serves both the home access walk and the work egress walk;
    // the router picks the best origin×dest station pair per trip, the fallback uses the
    // single nearest.
    let k_cands = if submat.is_some() { 3 } else { 1 };
    let station_cands: HashMap<String, Vec<StaCand>> = bg
        .par_iter()
        .map(|(g, b)| {
            let cands = station_candidates(
                b.enu, b.wnode, k_cands, &station_tree, &station_enu, &station_wnode, &walk,
                &sensors, &cam_tree, &occ, recall, &fov, groups_of(b.wnode),
            );
            (g.clone(), cands)
        })
        .collect();



    // Routing pass (parallel): route every pair once, storing the mode-choice inputs.
    let no_path = std::sync::atomic::AtomicU64::new(0);
    let subway = SubwayParams::from_env();
    let bus = BusParams::from_env();
    // M1 mobile layers (see od_exposure_modal): shared read-only state for the pass below.
    let mob = std::sync::Arc::new(MobileLayers::load()?);
    // M3 incidence-inversion emission (OUTLINE §8): when OURSPACE_EMIT_PAIRS is set, every
    // routed (home, work) pair also appends one row — home_bg, work_bg, jobs, and the
    // per-traversal mobile encounters — so the transpose (exposure each WORK BG generates,
    // decomposed by the HOME of the people captured) can be computed downstream. Off by
    // default: the pair file is ~1.9M rows for the full OD matrix.
    let pairs_path = std::env::var("OURSPACE_EMIT_PAIRS").ok().filter(|s| !s.is_empty());
    let pair_rows: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    eprintln!(
        "  M3 pair emission: {}",
        pairs_path.as_deref().unwrap_or("off")
    );
    eprintln!("  transit bus sub-mode: [{}]", bus.describe());
    let circuity = subway_circuity();
    match &submat {
        Some(_) => eprintln!("  transit: MTA subway cameras/trip [{}]", subway.describe()),
        None => eprintln!(
            "  transit: MTA subway cameras/trip [{}], line-haul circuity ×{circuity:.2}",
            subway.describe()
        ),
    }
    let mut home_ids: Vec<&String> = od.keys().collect();
    home_ids.sort();
    let walk_mps = DEFAULT_WALK_SPEED_MPS;
    let mut homes: Vec<HomeData> = home_ids
        .par_iter()
        .filter_map(|&hid| {
            let home = bg.get(hid)?;
            let home_cands = station_cands.get(hid).map(Vec::as_slice).unwrap_or(&[]);
            let home_groups = groups_of(home.wnode);
            let sub_bus = bus_share(hid);
            // Missing income (ACS suppression) must not become a fabricated middle earner:
            // such homes skip the MNL and use their observed ACS shares (see HomeData). The
            // placeholder below only keeps the struct populated; it never reaches a utility,
            // a quintile edge, or a calibration target.
            let income_obs = fnum(acs.get(hid, "median_hh_income"));
            let has_income = income_obs.is_some();
            let income = income_obs.unwrap_or(DEFAULT_INCOME);
            let vot = income / ANNUAL_WORK_MIN * VOT_WAGE_FRAC;
            let shares = mode_shares(hid);

            let mut dests = od[hid].clone();
            dests.sort_by(|a, b| b.1.total_cmp(&a.1));
            let jobs_total: f64 = dests.iter().map(|(_, j)| j).sum();
            dests.truncate(top_k);

            let mut pairs = Vec::with_capacity(dests.len());
            for (wid, jobs) in &dests {
                let Some(work) = bg.get(wid) else { continue };
                let Ok((route, t_drive_s, _e)) = graph.route_timed_pen(home.node, work.node, 1.0)
                else {
                    no_path.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                };
                // Drive-leg cameras along the driven route (road-facing kinds only),
                // net of cameras already covering the home/destination walksheds.
                let dest_groups = groups_of(work.wnode);
                let (e_drive, _) = route_leg_exposure(
                    &route.points, &sensors, &cam_tree, &occ, recall, false, &fov,
                    (home_groups, dest_groups),
                );
                // M_i^act mobile terms for the driven leg (expected encounters per traversal).
                let (m_ace, m_dash) = if mob.mobile.ace.is_some() || mob.dashcam_on {
                    route_mobile_exposure(&route.points, &mob, &occ, recall)
                } else {
                    (0.0, 0.0)
                };

                // The WALK alternative walks the WALK graph. Routing it on the drive geometry
                // (the pre-2026-07-14 behavior) denied walkers park paths and promenades and
                // let the drive network's one-way/turn shape distort their times — the same
                // class of error the walkshed fix removed. The drive distance pre-gates the
                // A* so the (majority) clearly-unwalkable pairs skip it.
                let (e_walk, t_walk, short) = if route.total_m <= WALK_MAX_M * 1.5 {
                    match walk.route(home.wnode, work.wnode) {
                        Ok(wr) if wr.total_m <= WALK_MAX_M => {
                            let e = route_leg_exposure(
                                &wr.points, &sensors, &cam_tree, &occ, recall, true, &fov,
                                (home_groups, dest_groups),
                            )
                            .1;
                            (e, wr.total_m / walk_mps / 60.0, true)
                        }
                        Ok(wr) => (0.0, wr.total_m / walk_mps / 60.0, false),
                        Err(_) => (0.0, 1.0e6 / 60.0, false), // no pedestrian path (e.g. across the Verrazzano)
                    }
                } else {
                    (0.0, route.total_m / walk_mps / 60.0, false)
                };

                // Transit exposure = street access/egress walks + guaranteed MTA station/
                // train cameras. With a subway matrix the itinerary is routed (best
                // candidate station pair; waits/transfers/run times from the timetable and
                // exact boardings setting the camera complement); without one, crow-flies ×
                // circuity estimates line-haul and transfers. The underground line-haul is
                // surveilled, not invisible.
                let work_cands = station_cands.get(wid).map(Vec::as_slice).unwrap_or(&[]);
                let leg = best_transit_leg(
                    submat.as_ref(), &subway, circuity, walk_mps, home_cands, work_cands,
                );
                // The BUS sub-mode rides street-level along (approximately) the drive
                // corridor: road-facing cameras see the vehicle — e_drive, already deduped
                // against the endpoint walksheds — plus the onboard MTA complement. The
                // observed ACS bus/subway split within transit weights the two sub-modes;
                // where the subway leg is infeasible the bus IS the transit alternative
                // (it exists wherever the drive route does), which finally gives express-bus
                // commuters (Staten Island, southeast Queens) a real itinerary.
                let bus_cams = bus.cameras(route.total_m);
                let e_bus = e_drive + bus_cams;
                let t_bus = (t_drive_s * bus.slowdown + bus.wait_s + bus.access_s) / 60.0;
                let (transit_ok, e_transit, e_subway, t_transit) = match &leg {
                    Some(l) => (
                        true,
                        sub_bus * e_bus + (1.0 - sub_bus) * (l.e_acc + l.e_subway),
                        sub_bus * bus_cams + (1.0 - sub_bus) * l.e_subway,
                        sub_bus * t_bus + (1.0 - sub_bus) * l.t_min,
                    ),
                    None => (true, e_bus, bus_cams, t_bus), // rail-infeasible ⇒ all-bus
                };
                // `dest_exp` is keyed by **walk**-graph node — `work.node` is the drive-graph
                // index, a different index space entirely, and would silently fetch an unrelated
                // block group's exposure.
                let dest = dest_exp.get(&work.wnode).map(|v| v.0).unwrap_or(0.0);

                let t_drive = t_drive_s / 60.0;
                let c_drive = route.total_m / 1000.0 * DRIVE_COST_PER_KM + DRIVE_FIXED_COST;

                pairs.push(ModePair {
                    flow: *jobs,
                    t_drive,
                    t_walk,
                    t_transit,
                    c_drive,
                    c_transit: FARE,
                    e_drive,
                    e_transit,
                    e_subway,
                    e_walk,
                    dest,
                    m_ace,
                    m_dash,
                    short,
                    transit_ok,
                    work_geoid: wid.clone(),
                });
            }
            if pairs.is_empty() {
                return None;
            }
            Some(HomeData {
                geoid: hid.to_string(),
                lat: home.lat,
                lon: home.lon,
                income,
                vot,
                group: 0,
                has_income,
                shares,
                jobs_total,
                pairs,
            })
        })
        .collect();

    // Assign each home BG to an income quintile (population-weighted by resident count is
    // overkill here; equal-count over home BGs is fine for calibration groups). A pure-VOT MNL
    // over-steepens the income→mode gradient vs. observed ACS (rich NYers still ride the subway),
    // so we don't calibrate a single citywide ASC — we calibrate ASCs *per income group* so each
    // group reproduces its own observed ACS mode split. VOT/time then only shapes the
    // within-group, distance-dependent variation; the income gradient itself comes from data.
    const N_GROUPS: usize = 5;
    // Quintile edges over PRESENT incomes only. Including the ~15% income-suppressed homes
    // at a fallback value pinned an edge to that exact number and dumped every suppressed BG
    // into the middle quintile; those homes now bypass the MNL entirely (observed shares).
    let mut incs: Vec<f64> = homes.iter().filter(|h| h.has_income).map(|h| h.income).collect();
    anyhow::ensure!(!incs.is_empty(), "no home BG has ACS income; cannot form quintiles");
    incs.sort_by(f64::total_cmp);
    let quintile_edges: Vec<f64> = (1..N_GROUPS)
        .map(|k| incs[(incs.len() * k / N_GROUPS).min(incs.len() - 1)])
        .collect();
    let group_of = |income: f64| -> usize {
        quintile_edges.iter().filter(|&&e| income >= e).count().min(N_GROUPS - 1)
    };
    let n_no_income = homes.iter().filter(|h| !h.has_income).count();
    for h in homes.iter_mut() {
        h.group = group_of(h.income); // meaningful only when has_income; unused otherwise
    }
    eprintln!(
        "  mode-choice: {} home BGs with ACS income (MNL), {} suppressed (observed ACS shares)",
        homes.len() - n_no_income,
        n_no_income
    );

    // Per-group observed ACS marginals (drive/transit/walk), summed over the MNL-scored
    // (income-present) home BGs — the suppressed homes are neither targets nor predictions.
    let mut tgt = [[0.0f64; 3]; N_GROUPS]; // [group][car,transit,walk]
    for h in homes.iter().filter(|h| h.has_income) {
        let g = h.group;
        tgt[g][0] += fnum(acs.get(&h.geoid, "commute_car")).unwrap_or(0.0);
        tgt[g][1] += fnum(acs.get(&h.geoid, "commute_transit")).unwrap_or(0.0);
        tgt[g][2] += fnum(acs.get(&h.geoid, "commute_walk")).unwrap_or(0.0);
    }
    let tgt_share: Vec<[f64; 3]> = tgt
        .iter()
        .map(|t| {
            let s = (t[0] + t[1] + t[2]).max(1e-9);
            [t[0] / s, t[1] / s, t[2] / s]
        })
        .collect();

    // Calibrate one (ASC_drive, ASC_transit) pair per group via the incremental-logit update
    // ASC_m += ln(target_m / predicted_m), iterated to convergence. Steps are clamped and the
    // constants bounded so a group whose transit is largely infeasible cannot run its ASC to
    // infinity chasing an unreachable target; convergence stops the loop early.
    let mut asc = vec![(0.0f64, 0.0f64); N_GROUPS];
    for it in 0..40 {
        // Flow-weighted predicted split per group, over the MNL-scored homes.
        let mut acc = vec![(0.0f64, 0.0f64, 0.0f64); N_GROUPS];
        let per = homes
            .par_iter()
            .filter(|h| h.has_income)
            .map(|h| {
                let (ad, at) = asc[h.group];
                let (mut sw, mut sd, mut st) = (0.0, 0.0, 0.0);
                for p in &h.pairs {
                    let (a, b, c) = mode_probs(p, h.vot, ad, at);
                    sw += p.flow * a;
                    sd += p.flow * b;
                    st += p.flow * c;
                }
                (h.group, sw, sd, st)
            })
            .collect::<Vec<_>>();
        for (g, sw, sd, st) in per {
            acc[g].0 += sw;
            acc[g].1 += sd;
            acc[g].2 += st;
        }
        let mut max_step = 0.0f64;
        for g in 0..N_GROUPS {
            let tot = (acc[g].0 + acc[g].1 + acc[g].2).max(1e-9);
            let pred_drive = (acc[g].1 / tot).max(1e-9);
            let pred_transit = (acc[g].2 / tot).max(1e-9);
            let step_d = (tgt_share[g][0] / pred_drive).ln().clamp(-2.0, 2.0);
            let step_t = (tgt_share[g][1] / pred_transit).ln().clamp(-2.0, 2.0);
            asc[g].0 = (asc[g].0 + step_d).clamp(-30.0, 30.0);
            asc[g].1 = (asc[g].1 + step_t).clamp(-30.0, 30.0);
            max_step = max_step.max(step_d.abs()).max(step_t.abs());
        }
        if max_step < 1e-4 {
            eprintln!("  mode-choice: ASC calibration converged after {} iterations", it + 1);
            break;
        }
    }
    for g in 0..N_GROUPS {
        eprintln!(
            "  mode-choice group {g} (inc<{:.0}): ACS target drive/transit/walk {:.2}/{:.2}/{:.2}; ASC_drive={:.2} ASC_transit={:.2}",
            quintile_edges.get(g).copied().unwrap_or(f64::INFINITY),
            tgt_share[g][0], tgt_share[g][1], tgt_share[g][2], asc[g].0, asc[g].1
        );
    }

    // Aggregate with the calibrated model → one row per home BG. Income-present homes use
    // the calibrated MNL; income-suppressed homes use their observed ACS shares, gated the
    // way `od-exposure-modal` gates them (no walking past WALK_MAX_M, no infeasible transit,
    // displaced shares reassigned proportionally to the remaining modes).
    let gated_shares = |shares: (f64, f64, f64), p: &ModePair| -> (f64, f64, f64) {
        let (mut pd, mut pt, mut pw) = shares; // (drive, transit, walk)
        if !p.short {
            let nz = pd + pt;
            if nz > 0.0 {
                pd += pw * pd / nz;
                pt += pw * pt / nz;
            } else {
                pd = 1.0;
                pt = 0.0;
            }
            pw = 0.0;
        }
        if !p.transit_ok {
            let nz = pd + pw;
            if nz > 0.0 {
                pd += pt * pd / nz;
                pw += pt * pw / nz;
            } else {
                pd = 1.0;
                pw = 0.0;
            }
            pt = 0.0;
        }
        (pw, pd, pt) // mode_probs order: (walk, drive, transit)
    };
    let mut rows: Vec<(String, String)> = homes
        .par_iter()
        .map(|h| {
            let (mut w_dest, mut w_commute) = (0.0f64, 0.0f64);
            let (mut w_drive, mut w_transit, mut w_walk, mut w_subway) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let (mut w_pd, mut w_pt, mut w_pw) = (0.0f64, 0.0f64, 0.0f64);
            let (mut w_ace, mut w_dash) = (0.0f64, 0.0f64);
            let (asc_drive, asc_transit) = asc[h.group];
            let mut jobs_cov = 0.0f64;
            for p in &h.pairs {
                let (pw, pd, pt) = if h.has_income {
                    mode_probs(p, h.vot, asc_drive, asc_transit)
                } else {
                    gated_shares(h.shares, p)
                };
                let commute = pd * p.e_drive + pt * p.e_transit + pw * p.e_walk;
                w_dest += p.flow * p.dest;
                w_commute += p.flow * commute;
                w_drive += p.flow * p.e_drive;
                w_transit += p.flow * p.e_transit;
                w_walk += p.flow * p.e_walk;
                w_subway += p.flow * pt * p.e_subway; // subway contribution to the commute leg
                // Street-exposed mode share: drive rides the corridor fully; the bus sub-share
                // of transit does too (rail riders skip it) — ACS B08301 bus/subway split,
                // same source as the transit sub-mode model. See od_exposure_modal.
                let bus_share = bus_share_of(&acs, &h.geoid);
                let street_share = (pd + pt * bus_share).min(1.0);
                w_ace += p.flow * p.m_ace * street_share;
                w_dash += p.flow * p.m_dash * street_share;
                w_pd += p.flow * pd;
                w_pt += p.flow * pt;
                w_pw += p.flow * pw;
                jobs_cov += p.flow;
            }
            let inv = 1.0 / jobs_cov;
            let (dest, commute) = (w_dest * inv, w_commute * inv);
            let row = format!(
                "{},{:.6},{:.6},{},{jobs_cov:.0},{:.0},\
                 {:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.4},{:.4}",
                h.geoid, h.lat, h.lon, h.pairs.len(), h.jobs_total,
                dest + commute, dest, commute,
                w_drive * inv, w_transit * inv, w_walk * inv, w_pd * inv, w_pt * inv, w_pw * inv,
                w_subway * inv,
                w_ace * inv,
                w_dash * inv,
            );
            // M3 emission: per-pair rows with the street-exposure-weighted mobile encounters
            // this (home, work) pair generates — exactly what the incidence-inversion
            // transpose consumes. Computed here while the mode shares are in scope.
            if let Some(pp) = &pairs_path {
                let mut buf = pair_rows.lock().unwrap();
                for p in &h.pairs {
                    let (pw, pd, pt) = if h.has_income {
                        mode_probs(p, h.vot, asc_drive, asc_transit)
                    } else {
                        gated_shares(h.shares, p)
                    };
                    let bus_share = bus_share_of(&acs, &h.geoid);
                    let street = (pd + pt * bus_share).min(1.0);
                    buf.push(format!(
                        "{},{},{},{:.4},{:.4},{:.4}",
                        h.geoid,
                        p.work_geoid,
                        p.flow,
                        street,
                        p.m_ace * street,
                        p.m_dash * street,
                    ));
                }
            }
            (h.geoid.clone(), row)
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    // M3: write the per-pair file (sorted for determinism) when emission is on.
    if let Some(pp) = &pairs_path {
        let mut buf = pair_rows.into_inner().unwrap();
        buf.sort();
        use std::io::Write as _;
        let pw = std::io::BufWriter::new(
            std::fs::File::create(pp).with_context(|| format!("creating {pp}"))?,
        );
        let mut w = pw;
        writeln!(w, "home_bg,work_bg,jobs,street_share,m_ace,m_dash")?;
        for line in &buf {
            writeln!(w, "{line}")?;
        }
        w.flush()?;
        eprintln!("  M3 pairs: {} rows -> {pp}", buf.len());
    }

    let mut w = std::io::BufWriter::new(
        std::fs::File::create(out_path).with_context(|| format!("creating {out_path}"))?,
    );
    writeln!(
        w,
        "home_bg,lat,lon,n_dest,jobs_covered,jobs_total,\
         A_modal,A_dest,commute_modal,commute_drive,commute_transit,commute_walk,\
         p_drive,p_transit,p_walk,commute_subway,m_ace_act,m_dash_act"
    )?;
    for (_, row) in &rows {
        writeln!(w, "{row}")?;
    }
    w.flush()?;
    eprintln!(
        "od-exposure-mnl: {} home BGs, top-{top_k}, {} no-path -> {out_path}",
        rows.len(),
        no_path.load(std::sync::atomic::Ordering::Relaxed)
    );
    Ok(())
}

/// A tiny header-indexed CSV, keyed by one column, for the exposure-table join. These files
/// are numeric/GEOID only (no embedded commas or quotes), so a plain `split(',')` is exact.
struct Table {
    idx: std::collections::HashMap<String, usize>,
    rows: std::collections::HashMap<String, Vec<String>>,
}

impl Table {
    fn load(path: &str, key_col: &str) -> Result<Table> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
        let mut lines = text.lines();
        let header: Vec<String> =
            lines.next().unwrap_or("").split(',').map(|s| s.trim().to_string()).collect();
        let idx: std::collections::HashMap<String, usize> =
            header.iter().enumerate().map(|(i, c)| (c.clone(), i)).collect();
        let kpos = *idx
            .get(key_col)
            .with_context(|| format!("{path} has no column '{key_col}'"))?;
        let mut rows = std::collections::HashMap::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let f: Vec<String> = line.split(',').map(|s| s.trim().to_string()).collect();
            if let Some(k) = f.get(kpos) {
                rows.insert(k.clone(), f);
            }
        }
        Ok(Table { idx, rows })
    }

    fn get<'a>(&'a self, key: &str, col: &str) -> Option<&'a str> {
        let i = *self.idx.get(col)?;
        self.rows.get(key)?.get(i).map(String::as_str)
    }

    fn keys(&self) -> impl Iterator<Item = &String> {
        self.rows.keys()
    }
}

fn fnum(s: Option<&str>) -> Option<f64> {
    s.and_then(|v| v.trim().parse::<f64>().ok())
}

/// `num/den` as a formatted share, or "" if either is missing/zero-denominator.
fn share(num: Option<&str>, den: Option<&str>) -> String {
    match (fnum(num), fnum(den)) {
        (Some(n), Some(d)) if d != 0.0 => format!("{:.5}", n / d),
        _ => String::new(),
    }
}

/// Join the exposure instruments + demographics into one tidy block-group table for the
/// disparity econometrics (plan §8 step 4): residential `R_i`, drive activity-space
/// `A_i^drive` (+ per-camera-type splits), ACS demographics, and a convenience composite
/// `E_i` on the plan's illustrative time-budget weights (14 h home / 1 h commute / 9 h work).
/// The census centroid set defines the populated-BG analysis rows; missing joins leave blanks.
/// Kept in Rust (the simulation/data-product layer) — the downstream spatial econometrics are
/// the only Python.
fn exposure_table(
    centroids: &str,
    ri_path: &str,
    ai_path: &str,
    acs_path: &str,
    out_path: &str,
    modal_path: Option<&str>,
    mnl_path: Option<&str>,
) -> Result<()> {
    const W_HOME: f64 = 14.0;
    const W_COMMUTE: f64 = 1.0;
    const W_WORK: f64 = 9.0;

    let empty = || Table {
        idx: std::collections::HashMap::new(),
        rows: std::collections::HashMap::new(),
    };
    let cent = Table::load(centroids, "id")?;
    let ri = Table::load(ri_path, "id")?;
    // E_i's home term deliberately uses OBSERVED counts (`cameras_raw`), but the OD bakes
    // apply whatever recall `load_fixed_sensors` ran with. At the default r = 1.0 the two are
    // identical; if the R_i bake ran at r ≠ 1 (OURSPACE_CENSUS_RECALL), the commute/dest
    // terms are inflated while the home term is not, and the composite silently mixes units.
    // Detect and warn rather than guess which the caller wanted.
    let recall_baked = ri.rows.values().any(|f| {
        match (ri.idx.get("cameras_raw"), ri.idx.get("cameras_corrected")) {
            (Some(&a), Some(&b)) => match (f.get(a), f.get(b)) {
                (Some(x), Some(y)) => x.parse::<f64>().ok().zip(y.parse::<f64>().ok())
                    .is_some_and(|(x, y)| (x - y).abs() > 1e-6),
                _ => false,
            },
            _ => false,
        }
    });
    if recall_baked {
        eprintln!(
            "  WARNING: {ri_path} was baked at recall != 1 — E_i mixes an observed home term \
             with recall-corrected commute/destination terms. Re-bake everything at one recall."
        );
    }
    // A_i rows are keyed by home_bg; empty file (job still running) → all-blank A columns.
    let ai = Table::load(ai_path, "home_bg").unwrap_or_else(|_| empty());
    // Optional mode-weighted A_i (od-exposure-modal, observed ACS shares), keyed by home_bg.
    let modal = modal_path.map(|p| Table::load(p, "home_bg")).transpose()?.unwrap_or_else(empty);
    // Optional MNL A_i (od-exposure-mnl, per-quintile-calibrated mode choice), same schema/key.
    let mnl = mnl_path.map(|p| Table::load(p, "home_bg")).transpose()?.unwrap_or_else(empty);
    let acs = Table::load(acs_path, "id")?;

    let boro = |g: &str| match &g[..5.min(g.len())] {
        "36005" => "Bronx",
        "36047" => "Brooklyn",
        "36061" => "Manhattan",
        "36081" => "Queens",
        "36085" => "Staten Island",
        _ => "?",
    };

    // R_i is the OBSERVED count of distinct camera groups — not a recall-corrected estimate.
    // `R_i_unconfirmed` is the CCTV-census-only sub-population (the only part a recall correction
    // inflates), and `R_i_recall_corrected` applies sim_core::CENSUS_RECALL to it. Because the
    // correction is linear, any other recall r is reconstructable downstream from R_i and
    // R_i_unconfirmed alone — which is what lets the undercount bootstrap propagate the recall CI
    // ([0.458, 0.544]) without re-running this bake. See `census_recall()`.
    let header = "GEOID,borough,lat,lon,population,\
        R_i,R_i_unconfirmed,R_i_recall_corrected,R_cctv,R_alpr,R_dot,R_enf,R_snap_m,\
        A_drive,A_route,A_dest,A_route_alpr,A_route_dot,A_route_enf,A_flow_coverage,A_n_dest,\
        A_modal,commute_modal,mode_p_drive,mode_p_transit,mode_p_walk,\
        A_mnl,commute_mnl,mnl_p_drive,mnl_p_transit,mnl_p_walk,\
        M_ace_res,M_dash_res,M_ace_act_mnl,M_dash_act_mnl,M_ace_act_modal,M_dash_act_modal,\
        E_i,E_i_mnl,\
        pop_acs,pct_white_nh,pct_black_nh,pct_asian_nh,pct_hispanic,median_hh_income,\
        pct_commute_transit,pct_commute_car,pct_commute_walk,pct_renter,poverty_rate\n";
    let mut out = String::from(header);

    let mut geoids: Vec<&String> = cent.keys().collect();
    geoids.sort();
    let (mut n, mut with_ai, mut with_modal, mut with_mnl) = (0usize, 0usize, 0usize, 0usize);
    for g in geoids {
        let g = g.as_str();
        let r = |c: &str| ri.get(g, c).unwrap_or("");
        let a = |c: &str| ai.get(g, c).unwrap_or("");
        let m = |c: &str| modal.get(g, c).unwrap_or("");
        let x = |c: &str| mnl.get(g, c).unwrap_or("");
        let d = |c: &str| acs.get(g, c).unwrap_or("");

        if ai.get(g, "A_drive").is_some() {
            with_ai += 1;
        }
        if modal.get(g, "A_modal").is_some() {
            with_modal += 1;
        }
        if mnl.get(g, "A_modal").is_some() {
            with_mnl += 1;
        }
        // E_i = (w_home·R_i + w_commute·commute-leg + w_work·destination-walkshed) / Σw (plan §2).
        // Prefer the mode-weighted commute leg (od-exposure-modal); fall back to the drive route.
        // NOTE: E_i is built on the OBSERVED R_i (`cameras_raw`), not the recall-corrected estimate,
        // so every headline number is an observation. The correction is reported alongside.
        let ri_c = fnum(ri.get(g, "cameras_raw"));
        // The recall-corrected estimate, reconstructed arithmetically (no re-run needed):
        //     R(r) = unconfirmed / r + (raw - unconfirmed)
        let ri_unconf = fnum(ri.get(g, "cameras_unconfirmed"));
        let ri_corrected = match (ri_c, ri_unconf) {
            (Some(raw), Some(unc)) => {
                format!("{:.3}", unc / sim_core::CENSUS_RECALL + (raw - unc))
            }
            _ => String::new(),
        };
        let ei = |commute_leg: Option<f64>, dest: Option<f64>| match (ri_c, commute_leg, dest) {
            (Some(rr), Some(cl), Some(de)) => format!(
                "{:.3}",
                (W_HOME * rr + W_COMMUTE * cl + W_WORK * de) / (W_HOME + W_COMMUTE + W_WORK)
            ),
            _ => String::new(),
        };
        let ei_modal = ei(
            fnum(modal.get(g, "commute_modal")).or_else(|| fnum(ai.get(g, "route_cams"))),
            fnum(modal.get(g, "A_dest")).or_else(|| fnum(ai.get(g, "dest_cams"))),
        );
        // MNL variant of E_i: MNL commute leg + MNL destination walkshed (A_dest is mode-independent,
        // so it matches the modal file's dest; fall back through modal → drive as needed).
        let ei_mnl = ei(
            fnum(mnl.get(g, "commute_modal")).or_else(|| fnum(ai.get(g, "route_cams"))),
            fnum(mnl.get(g, "A_dest"))
                .or_else(|| fnum(modal.get(g, "A_dest")))
                .or_else(|| fnum(ai.get(g, "dest_cams"))),
        );

        out.push_str(&format!(
            "{g},{},{},{},{},\
             {},{},{},{},{},{},{},{},\
             {},{},{},{},{},{},{},{},\
             {},{},{},{},{},\
             {},{},{},{},{},\
             {},{},{},{},{},{},\
             {},{},\
             {},{},{},{},{},{},{},{},{},{},{}\n",
            boro(g),
            cent.get(g, "lat").unwrap_or(""),
            cent.get(g, "lon").unwrap_or(""),
            cent.get(g, "population").unwrap_or(""),
            r("cameras_raw"), r("cameras_unconfirmed"), ri_corrected,
            r("cctv"), r("alpr"), r("dot"), r("enforcement"), r("snap_m"),
            a("A_drive"), a("route_cams"), a("dest_cams"), a("route_alpr"), a("route_dot"),
            a("route_enf"), share(ai.get(g, "jobs_covered"), ai.get(g, "jobs_total")), a("n_dest"),
            m("A_modal"), m("commute_modal"), m("p_drive"), m("p_transit"), m("p_walk"),
            x("A_modal"), x("commute_modal"), x("p_drive"), x("p_transit"), x("p_walk"),
            // M1 mobile terms (blank when the source bake predates them, so the
            // M2/M3 scripts can distinguish "layer off" from "zero exposure").
            r("m_ace_res"), r("m_dash_res"),
            x("m_ace_act"), x("m_dash_act"),
            m("m_ace_act"), m("m_dash_act"),
            ei_modal, ei_mnl,
            d("pop_total"),
            share(acs.get(g, "white_nh"), acs.get(g, "pop_total")),
            share(acs.get(g, "black_nh"), acs.get(g, "pop_total")),
            share(acs.get(g, "asian_nh"), acs.get(g, "pop_total")),
            share(acs.get(g, "hispanic"), acs.get(g, "pop_total")),
            d("median_hh_income"),
            share(acs.get(g, "commute_transit"), acs.get(g, "commute_total")),
            share(acs.get(g, "commute_car"), acs.get(g, "commute_total")),
            share(acs.get(g, "commute_walk"), acs.get(g, "commute_total")),
            share(acs.get(g, "renter_occ"), acs.get(g, "occ_units")),
            share(acs.get(g, "below_poverty"), acs.get(g, "pov_universe")),
        ));
        n += 1;
    }
    std::fs::write(out_path, out).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "exposure-table: {n} block groups ({with_ai} with A_i^drive, {with_modal} with A_modal, {with_mnl} with A_mnl) -> {out_path}"
    );
    Ok(())
}

fn heatmap(out: &str, hour: f64) -> Result<()> {
    let graph = GraphAsset::from_bytes(&read(GRAPH_PATH)?).context("decoding graph")?;
    let cam_layer = CctvCameraLayer::from_bytes(&read(CAMERAS_PATH)?)
        .context("decoding cameras")?
        .to_fixed_layer();
    let mut sensors = sensors_from_layer(&cam_layer, FixedCameraDefaults::default());
    // Add DeFlock ALPRs to the fixed-camera set.
    if let Ok(b) = std::fs::read(ALPR_PATH) {
        if let Ok(al) = AlprReaderLayer::from_bytes(&b) {
            sensors.extend(sensors_from_layer(&al.to_fixed_layer(), FixedCameraDefaults::default()));
        }
    }
    // Add NYC DOT traffic cameras (monitoring defaults: omnidirectional, low fps).
    if let Ok(b) = std::fs::read(DOT_PATH) {
        if let Ok(dot) = FixedSensorLayer::from_bytes(&b) {
            sensors.extend(sensors_from_layer(&dot, FixedCameraDefaults::dot_monitoring()));
        }
    }
    for (i, s) in sensors.iter_mut().enumerate() {
        s.id = i as u64;
    }
    let recall = 1.0 / cam_layer.recall.unwrap_or(1.0);

    // Same building-occlusion index the paper's exposure instrument uses, so the field the
    // app renders and the numbers we report are the same model (OURSPACE_OCCLUSION=0 to disable).
    let occ = load_occluders()?;
    for s in sensors.iter_mut() {
        s.host_poly = occ.containing_polygon(s.wedge.apex);
    }

    // Spatial index of camera positions (generous query radius; the FOV test
    // enforces the true per-camera range).
    let cam_tree: RTree<GeomWithData<[f64; 2], usize>> = RTree::bulk_load(
        sensors
            .iter()
            .enumerate()
            .map(|(i, s)| GeomWithData::new([s.wedge.apex.x, s.wedge.apex.y], i))
            .collect(),
    );
    let cam_query_r2 = 60.0_f64.powi(2);

    // ACE corridors -> densified point index + config.
    let mut mobile = MobileScenario::fields_only();
    let mut ace_tree: Option<RTree<[f64; 2]>> = None;
    let mut ace_cap_r2 = 0.0;
    let mut ace_routes = 0usize;
    if let Ok(bytes) = std::fs::read(ACE_PATH) {
        if let Ok(ace) = AceCorridorLayer::from_bytes(&bytes) {
            ace_routes = ace.routes.len();
            let cfg = AceConfig::new(
                ace.segments
                    .iter()
                    .map(|s| [Enu::new(s[0][0], s[0][1]), Enu::new(s[1][0], s[1][1])])
                    .collect(),
            );
            ace_cap_r2 = cfg.capture_range_m.powi(2);
            // Densify each segment to ~10 m points so proximity queries don't
            // miss the middle of long segments.
            let mut pts = Vec::new();
            for s in &ace.segments {
                let a = Enu::new(s[0][0], s[0][1]);
                let b = Enu::new(s[1][0], s[1][1]);
                let n = (a.distance(b) / 10.0).ceil().max(1.0) as usize;
                for k in 0..=n {
                    let p = a.lerp(b, k as f64 / n as f64);
                    pts.push([p.x, p.y]);
                }
            }
            ace_tree = Some(RTree::bulk_load(pts));
            mobile.ace = Some(cfg);
        }
    }

    // Spatial rideshare-camera field (real TLC trip density).
    let dashcam_field: Option<DashcamFieldLayer> = std::fs::read(DASHCAM_PATH)
        .ok()
        .and_then(|b| DashcamFieldLayer::from_bytes(&b).ok());

    let n = graph.edges.len();
    let (mut fixed, mut ace_v, mut dashcam, mut glasses) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let mut max_total = 0.0_f64;
    for e in &graph.edges {
        let mid = edge_midpoint(e);
        let nearby: Vec<SensorInstance> = cam_tree
            .locate_within_distance([mid.x, mid.y], cam_query_r2)
            .map(|g| sensors[g.data])
            .collect();
        let near_ace = ace_tree
            .as_ref()
            .is_some_and(|t| t.locate_within_distance([mid.x, mid.y], ace_cap_r2).next().is_some());
        let r = exposure_rates_per_minute(
            mid, hour, &nearby, &occ, near_ace, &mobile, recall, dashcam_field.as_ref(), None, None,
            None,
        );
        max_total = max_total.max(r.total());
        fixed.push(r.fixed);
        ace_v.push(r.ace);
        dashcam.push(r.dashcam);
        glasses.push(r.glasses);
    }

    let layer = HeatmapLayer {
        reference_hour: hour,
        fixed,
        ace: ace_v,
        dashcam,
        glasses,
        provenance: Provenance {
            source: "our-space batch coverage aggregation (fixed CCTV + ACE + dashcam/glasses fields)".into(),
            url: String::new(),
            license: "derived".into(),
            as_of: "2026-06-14".into(),
            notes: format!(
                "expected devices/min of presence per edge @ {hour:02.0}:00; \
                 {ace_routes} ACE routes; dashcam/glasses are scenario fields."
            ),
        },
    };
    std::fs::write(out, layer.to_bytes()?).with_context(|| format!("writing {out}"))?;
    eprintln!(
        "heatmap: {} edges, max total {:.1} devices/min @ {:02.0}:00 ({ace_routes} ACE routes) -> {out}",
        graph.edges.len(),
        max_total,
        hour
    );
    Ok(())
}

#[cfg(test)]
mod transit_tests {
    use super::*;
    use sim_core::subway::SubwayStation;
    use sim_core::GeoOrigin;

    fn params() -> SubwayParams {
        // The documented central defaults, fixed (not from_env: tests must not read the
        // environment): station 3, train 2, 12 km/transfer, cap 3, scale 1.
        SubwayParams {
            cams_station: 3.0,
            cams_train: 2.0,
            km_per_transfer: 12.0,
            max_transfers: 3.0,
            scale: 1.0,
        }
    }

    /// 3-station matrix with hand-set itineraries: 0->1 = 600 s / 1 boarding,
    /// 0->2 = 1500 s / 2 boardings, 1->2 = 300 s / 1 boarding (symmetric).
    fn mat3() -> sim_core::SubwayMatrix {
        let n = 3;
        let mut time_s = vec![f32::INFINITY; n * n];
        let mut boardings = vec![0u8; n * n];
        let mut set = |a: usize, b: usize, t: f32, brd: u8| {
            time_s[a * n + b] = t;
            time_s[b * n + a] = t;
            boardings[a * n + b] = brd;
            boardings[b * n + a] = brd;
        };
        set(0, 1, 600.0, 1);
        set(0, 2, 1500.0, 2);
        set(1, 2, 300.0, 1);
        for i in 0..n {
            time_s[i * n + i] = 0.0;
        }
        sim_core::SubwayMatrix {
            origin: GeoOrigin::MANHATTAN,
            stations: (0..n)
                .map(|i| SubwayStation {
                    id: format!("S{i}"),
                    name: format!("S{i}"),
                    lat: 40.7 + i as f64 * 0.01,
                    lon: -74.0,
                })
                .collect(),
            time_s,
            boardings,
            line_haul_m: vec![0.0; n * n],
            provenance: Provenance {
                source: String::new(),
                url: String::new(),
                license: String::new(),
                as_of: String::new(),
                notes: String::new(),
            },
        }
    }

    fn cand(idx: usize, e: f64, d_m: f64, x: f64) -> StaCand {
        StaCand { idx, e, d_m, enu: Enu::new(x, 0.0) }
    }

    #[test]
    fn cameras_for_boardings_matches_documented_complement() {
        let p = params();
        assert_eq!(p.cameras_for_boardings(0), 0.0);
        assert_eq!(p.cameras_for_boardings(1), 8.0); // 3*2 + 2*1: no transfer
        assert_eq!(p.cameras_for_boardings(2), 13.0); // 3*3 + 2*2: one transfer
        // Transfer cap holds: 6 boardings -> n capped at 3 -> 3*5 + 2*4.
        assert_eq!(p.cameras_for_boardings(6), 23.0);
    }

    #[test]
    fn router_picks_fastest_candidate_pair_and_prices_boardings() {
        let m = mat3();
        // Home can reach station 0 (short walk) or 1 (long); work sits at station 2.
        let home = [cand(0, 1.0, 140.0, 0.0), cand(1, 5.0, 2800.0, 1.0)];
        let work = [cand(2, 2.0, 140.0, 10.0)];
        let leg = best_transit_leg(Some(&m), &params(), 1.3, 1.4, &home, &work).unwrap();
        // Via 0: (140+140)/1.4 + 1500 = 1700 s; via 1: (2800+140)/1.4 + 300 = 2400 s.
        assert!((leg.t_min - 1700.0 / 60.0).abs() < 1e-9, "t {}", leg.t_min);
        assert_eq!(leg.e_acc, 3.0); // 1.0 + 2.0 access+egress walk exposure
        assert_eq!(leg.e_subway, 13.0); // 2 boardings -> one transfer complement
    }

    #[test]
    fn router_rejects_same_station_and_empty() {
        let m = mat3();
        let a = [cand(1, 0.0, 100.0, 0.0)];
        assert!(best_transit_leg(Some(&m), &params(), 1.3, 1.4, &a, &a).is_none());
        assert!(best_transit_leg(Some(&m), &params(), 1.3, 1.4, &a, &[]).is_none());
    }

    #[test]
    fn fallback_reproduces_legacy_formula() {
        // No matrix: single nearest stations 5 km apart crow-flies.
        let home = [cand(0, 1.5, 400.0, 0.0)];
        let work = [cand(1, 0.5, 300.0, 5000.0)];
        let p = params();
        let leg = best_transit_leg(None, &p, 1.3, 1.4, &home, &work).unwrap();
        let line = 5000.0 * 1.3;
        let want = (400.0 / 1.4 + line / SUBWAY_MPS + TRANSIT_WAIT_S + 300.0 / 1.4) / 60.0;
        assert!((leg.t_min - want).abs() < 1e-9);
        assert_eq!(leg.e_acc, 2.0);
        assert!((leg.e_subway - p.cameras(line)).abs() < 1e-12);
        // Same nearest station on both ends: rides nowhere, not a transit trip.
        let same = [cand(0, 0.0, 100.0, 0.0)];
        assert!(best_transit_leg(None, &p, 1.3, 1.4, &same, &same).is_none());
    }
}
