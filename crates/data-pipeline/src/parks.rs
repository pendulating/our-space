//! Bake NYC parks (Parks Properties, enfh-gkve) into flat ENU polygon rings — a
//! green context layer rendered under the street network, like the building fabric.
//!
//! Reuses the [`BuildingFootprints`] payload (a bag of flat exterior rings); the app
//! loads it under its own `ParksRes`/`.ospark` extension and tints it green. An
//! optional borough filter (`M`/`B`/`Q`/`X`/`R`) keeps the Manhattan build clean
//! while the citywide build bakes all five.
//!
//! Input: the Parks Properties GeoJSON (enfh-gkve) — `multipolygon` geometry with a
//! `borough` property.

use anyhow::{Context, Result};
use sim_core::assets::{BuildingFootprints, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

use crate::footprints::{rdp, ring_area_m2};

/// Parks are large and only a soft context wash, so a coarser simplification than
/// the buildings' 1 m is invisible and trims the citywide vertex count hard.
const SIMPLIFY_EPS_M: f64 = 2.5;
/// Drop park parts below this area (m²) — the dataset's slivers (street triangles,
/// medians, strips) read as green confetti otherwise. Keeps real pocket parks.
const MIN_AREA_M2: f64 = 400.0;

/// Single-letter NYC Parks borough code (`M`/`B`/`Q`/`X`/`R`) for a borough name or
/// code; `None` = keep every borough (citywide).
fn borough_code(borough: &str) -> Option<char> {
    match borough
        .to_ascii_lowercase()
        .replace([' ', '_', '-'], "")
        .as_str()
    {
        "manhattan" | "m" => Some('M'),
        "brooklyn" | "b" => Some('B'),
        "queens" | "q" => Some('Q'),
        "bronx" | "x" => Some('X'),
        "statenisland" | "si" | "r" => Some('R'),
        _ => None,
    }
}

pub fn bake(geojson_path: &str, out_path: &str, borough: Option<&str>) -> Result<usize> {
    let bytes = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(&bytes).context("parsing parks GeoJSON")?;
    let proj = EnuProjection::default();
    let want_boro = borough.and_then(borough_code);

    let mut polygons: Vec<Vec<[f32; 2]>> = Vec::new();
    let (mut verts_in, mut verts_out) = (0usize, 0usize);
    let (mut dropped_small, mut dropped_boro) = (0usize, 0usize);
    for f in fc.features {
        if let Some(code) = want_boro {
            let b = f
                .properties
                .as_ref()
                .and_then(|p| p.get("borough"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.chars().next());
            if b != Some(code) {
                dropped_boro += 1;
                continue;
            }
        }
        let Some(geom) = f.geometry else { continue };
        // Each part's exterior ring (Polygon → ring; MultiPolygon → every part).
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
    anyhow::ensure!(!polygons.is_empty(), "no parks parsed");

    let layer = BuildingFootprints {
        origin: GeoOrigin::MANHATTAN,
        polygons,
        provenance: Provenance {
            source: "NYC Parks Properties (NYC Parks & Recreation)".into(),
            url: "https://data.cityofnewyork.us/Recreation/Parks-Properties/enfh-gkve".into(),
            license: "NYC Open Data — public domain".into(),
            as_of: "2024".into(),
            notes: format!(
                "Park property polygons, RDP-simplified @ {SIMPLIFY_EPS_M} m, ≥ {MIN_AREA_M2} m²; \
                 flat green context layer."
            ),
        },
    };
    let n = layer.polygons.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "parks: {n} polygons ({dropped_boro} other-borough, {dropped_small} sub-min-area dropped); \
         vertices {verts_in} -> {verts_out} -> {out_path}"
    );
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::borough_code;

    #[test]
    fn borough_code_maps_names_codes_and_unknown() {
        // NYC Parks single-letter codes (note Bronx = X, Staten Island = R).
        assert_eq!(borough_code("Manhattan"), Some('M'));
        assert_eq!(borough_code("m"), Some('M'));
        assert_eq!(borough_code("brooklyn"), Some('B'));
        assert_eq!(borough_code("Queens"), Some('Q'));
        assert_eq!(borough_code("bronx"), Some('X'));
        assert_eq!(borough_code("Staten Island"), Some('R'));
        assert_eq!(borough_code("si"), Some('R'));
        // "all"/unknown → no filter.
        assert_eq!(borough_code("all"), None);
        assert_eq!(borough_code(""), None);
    }
}
