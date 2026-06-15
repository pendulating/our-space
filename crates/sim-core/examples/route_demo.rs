//! Headless end-to-end demo: load the baked graph + real Manhattan camera layer,
//! route between two lat/lon points, and print the exposure summary.
//!
//! Run from the workspace root (so the asset paths resolve):
//!   cargo run -p sim-core --example route_demo -- 40.758 -73.9855 40.7359 -73.9911

use sim_core::assets::{FixedSensorLayer, GraphAsset};
use sim_core::exposure::SourceKind;
use sim_core::scenario::{run_route, sensors_from_layer, FixedCameraDefaults};
use sim_core::simulation::SimParams;
use sim_core::{EnuProjection, StreetGraph};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<f64> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    // Defaults: Times Square -> Union Square.
    let (from_lat, from_lon, to_lat, to_lon) = match a.as_slice() {
        [a, b, c, d] => (*a, *b, *c, *d),
        _ => (40.7580, -73.9855, 40.7359, -73.9911),
    };

    let proj = EnuProjection::default();
    let graph_path = std::env::var("GRAPH")
        .unwrap_or_else(|_| "assets/processed/graph_manhattan.postcard".to_string());
    let graph = StreetGraph::from_asset(GraphAsset::from_bytes(&std::fs::read(&graph_path)?)?);
    let layer = FixedSensorLayer::from_bytes(&std::fs::read(
        "assets/processed/cameras_fixed.postcard",
    )?)?;

    let recall = layer.recall.unwrap_or(1.0);
    let sensors = sensors_from_layer(&layer, FixedCameraDefaults::default());
    let params = SimParams {
        recall_factor: 1.0 / recall,
        ..SimParams::default()
    };

    let from = proj.to_enu(from_lat, from_lon);
    let to = proj.to_enu(to_lat, to_lon);
    let (_route, sum) = run_route(&graph, &sensors, &[], from, to, params)?;

    println!("── our-space exposure demo ──────────────────────────────");
    println!(
        "graph: {} nodes, {} edges   |   {} fixed cameras ({}, recall {:.2})",
        graph.node_count(),
        graph.edge_count(),
        sensors.len(),
        layer.provenance.license,
        recall,
    );
    println!(
        "route: {:.0} m  (~{:.0} min walk @ 1.34 m/s)",
        sum.route_len_m,
        sum.duration_s / 60.0
    );
    println!();
    println!("  HEADLINE → ~{} cameras could have captured you", sum.headline_devices);
    println!("  expected capture-events: {:.0}", sum.total_expected_captures);
    println!("  fraction of route surveilled: {:.1}%", sum.fraction_surveilled * 100.0);
    let fixed = sum.tally.source(SourceKind::FixedCctv);
    println!(
        "  (fixed CCTV: {} distinct devices seen, before recall correction)",
        fixed.distinct_devices
    );
    println!("─────────────────────────────────────────────────────────");
    println!("source: {}", layer.provenance.source);
    Ok(())
}
