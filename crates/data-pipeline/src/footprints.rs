//! Bake Manhattan building footprints into flat ENU polygon rings — the subtle
//! ground-fabric layer under the street network.
//!
//! Input: the NYC Building Footprints GeoJSON (5zhs-2jue), pre-filtered to Manhattan
//! (BIN 1xxxxxx). Optionally clipped to the borough main-island boundary so it lines
//! up with the clipped streets (drops the detached-island footprints).

use anyhow::{Context, Result};
use sim_core::assets::{BuildingFootprints, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

/// Ring-simplification tolerance (m). Footprints are a subtle ground-fabric layer
/// only visible when zoomed in; collapsing vertices within ~1 m of the simplified
/// edge is imperceptible there but roughly halves the per-building vertex count
/// (NYC footprints average ~14 vertices/ring). The dominant size lever citywide.
const SIMPLIFY_EPS_M: f64 = 1.0;
/// Drop footprints below this area (m²) — tiny sheds/garages/slivers that read as
/// noise. Manhattan is dense large buildings (only ~2% < 50 m²), so this mostly
/// trims the outer boroughs' clutter; keep it conservative so row houses survive.
const MIN_AREA_M2: f64 = 18.0;

/// Map a borough name (or a single BIN digit) to the BIN leading digit that
/// selects it: Manhattan 1, Bronx 2, Brooklyn 3, Queens 4, Staten Island 5.
/// `all`/unrecognized → `None` = no filter (process every feature, for inputs
/// that are already a single borough like `manhattan_footprints.geojson`).
fn borough_bin_digit(borough: &str) -> Option<char> {
    match borough
        .to_ascii_lowercase()
        .replace([' ', '_', '-'], "")
        .as_str()
    {
        "manhattan" | "1" => Some('1'),
        "bronx" | "2" => Some('2'),
        "brooklyn" | "3" => Some('3'),
        "queens" | "4" => Some('4'),
        "statenisland" | "si" | "5" => Some('5'),
        _ => None,
    }
}

/// First character of a feature's `bin` property (string or numeric), for the
/// per-borough filter. `None` if the feature carries no usable bin.
fn feature_bin_lead(f: &geojson::Feature) -> Option<char> {
    let v = f.properties.as_ref()?.get("bin")?;
    let s = v
        .as_str()
        .map(str::to_string)
        .or_else(|| v.as_f64().map(|n| format!("{}", n as u64)))?;
    s.chars().next()
}

/// Shoelace area of a closed/near-closed ENU ring (m²).
pub(crate) fn ring_area_m2(r: &[[f64; 2]]) -> f64 {
    let n = r.len();
    if n < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    let mut j = n - 1;
    for i in 0..n {
        s += (r[j][0] + r[i][0]) * (r[j][1] - r[i][1]);
        j = i;
    }
    (s * 0.5).abs()
}

/// Perpendicular distance from `p` to the segment line through `a`,`b` (or the
/// point distance to `a` when the segment is degenerate).
fn perp_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-12 {
        let (px, py) = (p[0] - a[0], p[1] - a[1]);
        return (px * px + py * py).sqrt();
    }
    ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len2.sqrt()
}

/// Ramer–Douglas–Peucker on a polyline, keeping the endpoints. Iterative (an
/// explicit stack) so a pathological ring can't blow the call stack.
pub(crate) fn rdp(pts: &[[f64; 2]], eps: f64) -> Vec<[f64; 2]> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((lo, hi)) = stack.pop() {
        if hi <= lo + 1 {
            continue;
        }
        let (a, b) = (pts[lo], pts[hi]);
        let (mut idx, mut dmax) = (lo, 0.0);
        for (i, p) in pts.iter().enumerate().take(hi).skip(lo + 1) {
            let d = perp_dist(*p, a, b);
            if d > dmax {
                dmax = d;
                idx = i;
            }
        }
        if dmax > eps {
            keep[idx] = true;
            stack.push((lo, idx));
            stack.push((idx, hi));
        }
    }
    pts.iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, p)| *p)
        .collect()
}

pub fn bake(
    geojson_path: &str,
    out_path: &str,
    borough: Option<&str>,
    boundary_geojson: Option<&str>,
) -> Result<usize> {
    let bytes = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(&bytes).context("parsing footprints GeoJSON")?;
    let proj = EnuProjection::default();
    let boundary = boundary_geojson
        .map(crate::boundary::ManhattanBoundary::load)
        .transpose()?;
    // Per-borough selection by BIN leading digit (None = keep all features).
    let want_bin = borough.and_then(borough_bin_digit);

    let mut polygons: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut dropped = 0usize;
    let mut dropped_small = 0usize;
    let mut dropped_boro = 0usize;
    let (mut verts_in, mut verts_out) = (0usize, 0usize);
    for f in fc.features {
        // Borough filter first (cheap, before geometry projection).
        if let Some(d) = want_bin {
            if feature_bin_lead(&f) != Some(d) {
                dropped_boro += 1;
                continue;
            }
        }
        let Some(geom) = f.geometry else { continue };
        // Each footprint part's exterior ring (Polygon → ring; MultiPolygon → parts).
        let raw_rings: Vec<Vec<[f64; 2]>> = match geom.value {
            geojson::Value::Polygon(rings) => rings
                .into_iter()
                .take(1)
                .map(|r| r.iter().map(|p| [p[0], p[1]]).collect())
                .collect(),
            geojson::Value::MultiPolygon(polys) => polys
                .into_iter()
                .filter_map(|poly| poly.into_iter().next())
                .map(|r| r.iter().map(|p| [p[0], p[1]]).collect())
                .collect(),
            _ => continue,
        };
        for ring in raw_rings {
            if ring.len() < 4 {
                continue;
            }
            // Project to ENU (f64), accumulate centroid for the boundary test.
            let enu64: Vec<[f64; 2]> = ring
                .iter()
                .map(|p| {
                    let e = proj.to_enu(p[1], p[0]);
                    [e.x, e.y]
                })
                .collect();
            if let Some(b) = &boundary {
                let n = enu64.len() as f64;
                let cx = enu64.iter().map(|p| p[0]).sum::<f64>() / n;
                let cy = enu64.iter().map(|p| p[1]).sum::<f64>() / n;
                if !b.contains([cx, cy]) {
                    dropped += 1;
                    continue;
                }
            }
            // Drop sub-threshold slivers, then simplify the ring (the citywide size lever).
            if ring_area_m2(&enu64) < MIN_AREA_M2 {
                dropped_small += 1;
                continue;
            }
            verts_in += enu64.len();
            let simplified = rdp(&enu64, SIMPLIFY_EPS_M);
            // RDP can collapse a degenerate ring below 3 distinct points; skip those.
            if simplified.len() < 4 {
                dropped_small += 1;
                continue;
            }
            verts_out += simplified.len();
            polygons.push(simplified.iter().map(|p| [p[0] as f32, p[1] as f32]).collect());
        }
    }
    anyhow::ensure!(!polygons.is_empty(), "no footprints parsed");

    let layer = BuildingFootprints {
        origin: GeoOrigin::MANHATTAN,
        polygons,
        provenance: Provenance {
            source: "NYC Building Footprints (Department of City Planning / DOITT)".into(),
            url: "https://data.cityofnewyork.us/City-Government/BUILDING/5zhs-2jue".into(),
            license: "NYC Open Data — public domain".into(),
            as_of: "2017".into(),
            notes: format!(
                "Footprint exterior rings, RDP-simplified @ {SIMPLIFY_EPS_M} m, ≥ {MIN_AREA_M2} m²; \
                 flat ground-fabric layer."
            ),
        },
    };
    let n = layer.polygons.len();
    let ratio = if verts_in > 0 { verts_out as f64 / verts_in as f64 } else { 1.0 };
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "footprints: {n} polygons ({dropped_boro} other-borough, {dropped} off-boundary, \
         {dropped_small} sub-min-area dropped); vertices {verts_in} -> {verts_out} \
         ({:.0}% after RDP) -> {out_path}",
        ratio * 100.0
    );
    Ok(n)
}
