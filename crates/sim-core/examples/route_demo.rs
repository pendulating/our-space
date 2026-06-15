//! Headless end-to-end demo: load the baked graph + real Manhattan camera layer,
//! route between two lat/lon points, and print the exposure summary.
//!
//! Run from the workspace root (so the asset paths resolve):
//!   cargo run -p sim-core --example route_demo -- 40.758 -73.9855 40.7359 -73.9911

use sim_core::assets::{AceCorridorLayer, FixedSensorLayer, GraphAsset};
use sim_core::scenario::{run_route, sensors_from_layer, FixedCameraDefaults};
use sim_core::simulation::SimParams;
use sim_core::{AceConfig, EnuProjection, MobileScenario, StreetGraph, Vec2};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<f64> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    // Defaults: Times Square -> Union Square.
    let (from_lat, from_lon, to_lat, to_lon) = match a.as_slice() {
        [a, b, c, d] => (*a, *b, *c, *d),
        _ => (40.7580, -73.9855, 40.7359, -73.9911),
    };

    let proj = EnuProjection::default();
    let base = "crates/app-interactive/assets/processed";
    let graph_path = std::env::var("GRAPH").unwrap_or_else(|_| format!("{base}/graph_manhattan.osgraph"));
    let graph = StreetGraph::from_asset(GraphAsset::from_bytes(&std::fs::read(&graph_path)?)?);
    let layer = FixedSensorLayer::from_bytes(&std::fs::read(format!("{base}/cameras_fixed.oscam"))?)?;

    let recall = layer.recall.unwrap_or(1.0);
    let sensors = sensors_from_layer(&layer, FixedCameraDefaults::default());
    let params = SimParams {
        recall_factor: 1.0 / recall,
        ..SimParams::default()
    };

    // Departure hour from env (default 5pm) and the mobile classes on at defaults.
    let departure_hour: f64 = std::env::var("HOUR").ok().and_then(|s| s.parse().ok()).unwrap_or(17.0);
    let mut mobile = MobileScenario::fields_only(); // dashcam + glasses
    if let Ok(bytes) = std::fs::read(format!("{base}/ace_corridors.osace")) {
        if let Ok(ace) = AceCorridorLayer::from_bytes(&bytes) {
            let segs = ace
                .segments
                .iter()
                .map(|s| [Vec2::new(s[0][0], s[0][1]), Vec2::new(s[1][0], s[1][1])])
                .collect();
            mobile.ace = Some(AceConfig::new(segs));
        }
    }

    let from = proj.to_enu(from_lat, from_lon);
    let to = proj.to_enu(to_lat, to_lon);
    let (_route, sum) = run_route(&graph, &sensors, &[], &mobile, from, to, params, departure_hour)?;

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
        "route: {:.0} m  (~{:.0} min walk @ 1.34 m/s)  ·  departing {:02.0}:00",
        sum.route_len_m,
        sum.duration_s / 60.0,
        departure_hour,
    );
    println!();
    println!("  HEADLINE → ~{} devices could have captured you", sum.headline_devices);
    println!("  expected capture-events (frames): {:.0}", sum.total_expected_frames);
    println!("  fraction of route under fixed surveillance: {:.1}%", sum.fraction_surveilled * 100.0);
    println!();
    println!("  by source (tier · expected devices · P≥1 capture):");
    for b in &sum.breakdown {
        println!(
            "    {:>13}  [{:?}]  {:>5.2}   {:>5.1}%",
            b.kind.label(),
            b.tier,
            b.devices,
            b.p_at_least_one * 100.0,
        );
    }
    println!("─────────────────────────────────────────────────────────");
    println!("source: {} · mobile classes are scenario estimates, not measurements", layer.provenance.source);
    Ok(())
}
