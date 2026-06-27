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
        _ => {
            eprintln!("usage: batch heatmap <out.postcard> [hour]");
            std::process::exit(2);
        }
    }
}

fn read(path: &str) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("reading {path} (bake assets first)"))
}

fn edge_midpoint(e: &EdgeData) -> Enu {
    let p = &e.polyline[e.polyline.len() / 2];
    Enu::new(p[0], p[1])
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
            mid, hour, &nearby, &[], near_ace, &mobile, recall, dashcam_field.as_ref(), None, None, None,
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
