//! Synthetic Manhattan-style grid graph generator.
//!
//! Lets the app/exposure pipeline run end-to-end before the real OSMnx/Overpass
//! walk network is wired in. Produces a regular `rows x cols` lattice of
//! intersections in local ENU meters, centered on the projection origin, with
//! bidirectional walkable edges (avenues N-S, streets E-W).

use sim_core::assets::{EdgeData, GraphAsset, NodePoint, Provenance};
use sim_core::projection::GeoOrigin;

pub fn synthetic_grid(rows: u32, cols: u32, spacing_m: f64) -> GraphAsset {
    assert!(rows >= 2 && cols >= 2, "grid must be at least 2x2");

    let id = |r: u32, c: u32| r * cols + c;
    let cx = (cols as f64 - 1.0) / 2.0;
    let cy = (rows as f64 - 1.0) / 2.0;
    let pos = |r: u32, c: u32| -> [f64; 2] {
        [(c as f64 - cx) * spacing_m, (r as f64 - cy) * spacing_m]
    };

    let mut nodes = Vec::with_capacity((rows * cols) as usize);
    for r in 0..rows {
        for c in 0..cols {
            let p = pos(r, c);
            nodes.push(NodePoint { x: p[0], y: p[1] });
        }
    }

    let mut edges = Vec::new();
    let mut seg_id: i64 = 0;
    let push_edge = |from: u32, to: u32, a: [f64; 2], b: [f64; 2], edges: &mut Vec<EdgeData>, seg: &mut i64| {
        let len = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        edges.push(EdgeData {
            from,
            to,
            length_m: len,
            polyline: vec![a, b],
            segment_id: Some(*seg),
        });
        *seg += 1;
    };

    for r in 0..rows {
        for c in 0..cols {
            // East neighbor (street running E-W).
            if c + 1 < cols {
                push_edge(id(r, c), id(r, c + 1), pos(r, c), pos(r, c + 1), &mut edges, &mut seg_id);
            }
            // North neighbor (avenue running N-S).
            if r + 1 < rows {
                push_edge(id(r, c), id(r + 1, c), pos(r, c), pos(r + 1, c), &mut edges, &mut seg_id);
            }
        }
    }

    GraphAsset {
        origin: GeoOrigin::MANHATTAN,
        nodes,
        edges,
        provenance: Provenance {
            source: "synthetic grid".into(),
            url: String::new(),
            license: "n/a (generated)".into(),
            as_of: "2026-06-14".into(),
            notes: format!("{rows}x{cols} lattice, {spacing_m}m spacing — placeholder for the real walk network"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::graph::StreetGraph;
    use sim_core::Vec2;

    #[test]
    fn grid_is_connected_and_routable() {
        let asset = synthetic_grid(5, 5, 80.0);
        assert_eq!(asset.nodes.len(), 25);
        // 5x4 horizontal + 4x5 vertical = 40 edges.
        assert_eq!(asset.edges.len(), 40);
        let g = StreetGraph::from_asset(asset);
        // Route between opposite corners must exist; Manhattan distance = 8*80.
        let r = g.route_points(Vec2::new(-1000.0, -1000.0), Vec2::new(1000.0, 1000.0)).unwrap();
        assert!((r.total_m - 640.0).abs() < 1e-6, "got {}", r.total_m);
    }
}
