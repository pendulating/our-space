//! Bake NYC DOT pedestrian plazas (k5k6-6jex) into flat ENU polygon rings — a
//! paved public-space layer the app renders with a concrete fill + a hatched
//! texture (the hatch is generated app-side from these polygons).
//!
//! Reuses the [`BuildingFootprints`] payload (flat exterior rings); loaded app-side
//! under `PlazaRes`/`.osplaza`. There are only ~93 plazas citywide, so one asset
//! serves both the Manhattan and citywide builds.
//!
//! Input: the Pedestrian Plazas GeoJSON (k5k6-6jex) — `the_geom` MultiPolygons.

use anyhow::{Context, Result};
use sim_core::assets::{BuildingFootprints, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

use crate::footprints::{rdp, ring_area_m2};

/// Plazas are small, crisp paved spaces — keep edges fairly faithful (1.5 m).
const SIMPLIFY_EPS_M: f64 = 1.5;
/// Drop parts below this area (m²) — tiny slivers of the multipart geometry.
const MIN_AREA_M2: f64 = 120.0;

pub fn bake(geojson_path: &str, out_path: &str) -> Result<usize> {
    let bytes = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(&bytes).context("parsing plazas GeoJSON")?;
    let proj = EnuProjection::default();

    let mut polygons: Vec<Vec<[f32; 2]>> = Vec::new();
    let (mut verts_in, mut verts_out, mut dropped_small) = (0usize, 0usize, 0usize);
    for f in fc.features {
        let Some(geom) = f.geometry else { continue };
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
            let enu64: Vec<[f64; 2]> = ring
                .iter()
                .map(|p| {
                    let e = proj.to_enu(p[1], p[0]);
                    [e.x, e.y]
                })
                .collect();
            if ring_area_m2(&enu64) < MIN_AREA_M2 {
                dropped_small += 1;
                continue;
            }
            verts_in += enu64.len();
            let simplified = rdp(&enu64, SIMPLIFY_EPS_M);
            if simplified.len() < 4 {
                dropped_small += 1;
                continue;
            }
            verts_out += simplified.len();
            polygons.push(simplified.iter().map(|p| [p[0] as f32, p[1] as f32]).collect());
        }
    }
    anyhow::ensure!(!polygons.is_empty(), "no plazas parsed");

    let layer = BuildingFootprints {
        origin: GeoOrigin::MANHATTAN,
        polygons,
        provenance: Provenance {
            source: "NYC DOT Pedestrian Plazas".into(),
            url: "https://data.cityofnewyork.us/Transportation/NYC-DOT-Pedestrian-Plazas-Polygon/k5k6-6jex".into(),
            license: "NYC Open Data — public domain".into(),
            as_of: "2024".into(),
            notes: format!(
                "Pedestrian-plaza polygons, RDP-simplified @ {SIMPLIFY_EPS_M} m, ≥ {MIN_AREA_M2} m²; \
                 concrete fill + app-side hatch."
            ),
        },
    };
    let n = layer.polygons.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "plazas: {n} polygons ({dropped_small} sub-min-area dropped); \
         vertices {verts_in} -> {verts_out} -> {out_path}"
    );
    Ok(n)
}
