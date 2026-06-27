//! Bake the vehicle (rideshare) route pool for the animated dashcam agents.
//!
//! Inputs:
//!   - the baked routable graph (`.osgraph`)
//!   - taxi-zone polygons GeoJSON (`LocationID` -> ENU rings)
//!   - a zone O-D matrix CSV (`PULocationID,DOLocationID,trips`) aggregated from
//!     the NYC TLC HVFHV records via DuckDB (Manhattan↔Manhattan)
//!
//! For the top O-D pairs by trip volume we route the pickup-zone centroid to the
//! dropoff-zone centroid **offline** over the pedestrian walk graph (v1: no drive
//! network, so one-way/turn restrictions are ignored — decorative agents only),
//! decimate the polyline, and store it with a sampling weight ∝ trip volume. The
//! runtime samples this pool with replacement, so a few hundred concurrent cars
//! drawn from ~1000 distinct routes reads as endless, density-weighted traffic.

use std::collections::HashMap;

use anyhow::{Context, Result};
use sim_core::assets::{GraphAsset, Provenance, VehicleRoute, VehicleRoutesLayer};
use sim_core::graph::StreetGraph;
use sim_core::math::Vec2;
use sim_core::projection::{EnuProjection, GeoOrigin};

use crate::dashcam::parse_zones;

/// Douglas–Peucker tolerance (m): vehicle dots don't need full street curvature.
const DECIMATE_TOL_M: f64 = 5.0;

/// Area-weighted centroid of a zone's largest ring (snapping handles the rest).
pub(crate) fn zone_centroid(rings: &[Vec<[f64; 2]>]) -> Option<Vec2> {
    let ring = rings.iter().max_by(|a, b| {
        ring_area(a).partial_cmp(&ring_area(b)).unwrap_or(std::cmp::Ordering::Equal)
    })?;
    if ring.len() < 3 {
        return None;
    }
    // Shoelace centroid.
    let (mut cx, mut cy, mut a2) = (0.0, 0.0, 0.0);
    let n = ring.len();
    let mut j = n - 1;
    for i in 0..n {
        let cross = ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
        a2 += cross;
        cx += (ring[j][0] + ring[i][0]) * cross;
        cy += (ring[j][1] + ring[i][1]) * cross;
        j = i;
    }
    if a2.abs() < 1e-9 {
        // Degenerate: fall back to vertex average.
        let (sx, sy) = ring.iter().fold((0.0, 0.0), |(x, y), p| (x + p[0], y + p[1]));
        return Some(Vec2::new(sx / n as f64, sy / n as f64));
    }
    Some(Vec2::new(cx / (3.0 * a2), cy / (3.0 * a2)))
}

fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    let mut j = n - 1;
    for i in 0..n {
        s += (ring[j][0] + ring[i][0]) * (ring[j][1] - ring[i][1]);
        j = i;
    }
    (s * 0.5).abs()
}

/// Douglas–Peucker polyline simplification on ENU points.
pub(crate) fn decimate(points: &[Vec2], tol: f64) -> Vec<Vec2> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    *keep.last_mut().unwrap() = true;
    dp(points, 0, points.len() - 1, tol, &mut keep);
    points.iter().zip(keep).filter_map(|(p, k)| k.then_some(*p)).collect()
}

fn dp(pts: &[Vec2], lo: usize, hi: usize, tol: f64, keep: &mut [bool]) {
    if hi <= lo + 1 {
        return;
    }
    let (a, b) = (pts[lo], pts[hi]);
    let (mut max_d, mut idx) = (0.0, lo);
    for (i, p) in pts.iter().enumerate().take(hi).skip(lo + 1) {
        let d = sim_core::math::point_segment_distance(*p, a, b);
        if d > max_d {
            max_d = d;
            idx = i;
        }
    }
    if max_d > tol {
        keep[idx] = true;
        dp(pts, lo, idx, tol, keep);
        dp(pts, idx, hi, tol, keep);
    }
}

pub fn bake(
    graph_path: &str,
    geojson_path: &str,
    od_csv: &str,
    out_path: &str,
    max_routes: usize,
) -> Result<usize> {
    let proj = EnuProjection::default();

    // Graph (offline routing).
    let g_bytes = std::fs::read(graph_path).with_context(|| format!("reading {graph_path}"))?;
    let graph = StreetGraph::from_asset(GraphAsset::from_bytes(&g_bytes).context("decoding graph")?);

    // Zone centroids by LocationID.
    let zone_bytes = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let zones = parse_zones(&zone_bytes, &proj)?;
    let mut centroid: HashMap<i64, Vec2> = HashMap::new();
    for (loc, rings) in &zones {
        if let Some(c) = zone_centroid(rings) {
            centroid.insert(*loc, c);
        }
    }

    // O-D pairs, sorted by trips descending (the CSV is already sorted, but be safe).
    let mut rdr = csv::Reader::from_path(od_csv).with_context(|| format!("opening {od_csv}"))?;
    let mut od: Vec<(i64, i64, f64)> = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        let pu: i64 = rec.get(0).unwrap_or("").parse().unwrap_or(-1);
        let dolc: i64 = rec.get(1).unwrap_or("").parse().unwrap_or(-1);
        let trips: f64 = rec.get(2).unwrap_or("0").parse().unwrap_or(0.0);
        if pu >= 0 && dolc >= 0 && trips > 0.0 {
            od.push((pu, dolc, trips));
        }
    }
    od.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut routes = Vec::new();
    let (mut tried, mut no_path) = (0usize, 0usize);
    for (pu, dolc, trips) in od.into_iter().take(max_routes) {
        let (Some(&from), Some(&to)) = (centroid.get(&pu), centroid.get(&dolc)) else {
            continue;
        };
        tried += 1;
        let route = match graph.route_points(from, to) {
            Ok(r) => r,
            Err(_) => {
                no_path += 1;
                continue;
            }
        };
        let pts = decimate(&route.points, DECIMATE_TOL_M);
        if pts.len() < 2 {
            continue;
        }
        routes.push(VehicleRoute {
            polyline: pts.iter().map(|p| [p.x as f32, p.y as f32]).collect(),
            length_m: route.total_m as f32,
            weight: trips as f32,
        });
    }
    anyhow::ensure!(!routes.is_empty(), "no vehicle routes baked");

    // Normalize weights to sum 1.0.
    let total: f32 = routes.iter().map(|r| r.weight).sum();
    if total > 0.0 {
        for r in &mut routes {
            r.weight /= total;
        }
    }

    let n = routes.len();
    let avg_pts = routes.iter().map(|r| r.polyline.len()).sum::<usize>() as f64 / n as f64;
    let layer = VehicleRoutesLayer {
        origin: GeoOrigin::MANHATTAN,
        routes,
        provenance: Provenance {
            source: "NYC TLC High-Volume FHV (Uber/Lyft) trip O-D, top Manhattan zone pairs".into(),
            url: "https://www.nyc.gov/site/tlc/about/tlc-trip-record-data.page".into(),
            license: "NYC OpenData / TLC terms".into(),
            as_of: "TLC 2024-06 HVFHV".into(),
            notes: "Zone-centroid routes over the pedestrian walk graph (decorative; \
                    ignores one-way/turn restrictions). Weighted by real trip volume."
                .into(),
        },
    };
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "Vehicle routes: {n} baked ({no_path}/{tried} no-path skipped), avg {avg_pts:.0} pts/route -> {out_path}"
    );
    Ok(n)
}
