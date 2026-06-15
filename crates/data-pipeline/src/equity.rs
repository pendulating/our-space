//! Bake the block-group equity overlay (diversity vs. camera density), mirroring
//! Dahir et al.
//!
//! Inputs:
//!   - TIGER block-group geometry GeoJSON (NY County 36061)
//!   - Census ACS 5-year B03002 race counts JSON (Census Data API)
//!   - Dahir `map_data.csv` (camera detection points)
//!
//! Computes Shannon entropy over white/Black/Asian/Hispanic/other exactly as the
//! paper, and counts detected cameras per block group by point-in-polygon.

use std::collections::HashMap;

use anyhow::{Context, Result};
use geo::{Contains, Coord, LineString, Point, Polygon};
use sim_core::assets::{BlockGroup, EquityLayer, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

/// ACS counts for a block group: total + the four named groups (NH).
#[derive(Default, Clone, Copy)]
struct Acs {
    total: f64,
    white: f64,
    black: f64,
    asian: f64,
    hispanic: f64,
}

impl Acs {
    /// Shannon entropy over {white, Black, Asian, Hispanic, other}.
    fn entropy(&self) -> f64 {
        if self.total <= 0.0 {
            return 0.0;
        }
        let other = (self.total - self.white - self.black - self.asian - self.hispanic).max(0.0);
        [self.white, self.black, self.asian, self.hispanic, other]
            .iter()
            .map(|&c| c / self.total)
            .filter(|&p| p > 0.0)
            .map(|p| -p * p.ln())
            .sum()
    }
}

/// Parse the GEOID -> race-counts map (B03002 white/Black/Asian/Hispanic +
/// total), produced from a keyless source (Census Reporter) — see README.
fn parse_acs(json: &[u8]) -> Result<HashMap<String, Acs>> {
    #[derive(serde::Deserialize)]
    struct Raw {
        total: f64,
        white: f64,
        black: f64,
        asian: f64,
        hispanic: f64,
    }
    let raw: HashMap<String, Raw> = serde_json::from_slice(json).context("parsing ACS map JSON")?;
    Ok(raw
        .into_iter()
        .map(|(geoid, r)| {
            (
                geoid,
                Acs {
                    total: r.total,
                    white: r.white,
                    black: r.black,
                    asian: r.asian,
                    hispanic: r.hispanic,
                },
            )
        })
        .collect())
}

/// Read NY camera detection points (lon, lat) from the Dahir CSV.
fn camera_points(csv_path: &str) -> Result<Vec<Point<f64>>> {
    #[derive(serde::Deserialize)]
    struct Row {
        lat: f64,
        lon: f64,
        city: String,
        camera_count: i32,
    }
    let mut rdr = csv::Reader::from_path(csv_path).with_context(|| format!("opening {csv_path}"))?;
    let mut pts = Vec::new();
    for rec in rdr.deserialize::<Row>() {
        let r = rec?;
        if r.city == "New York" && r.camera_count >= 1 {
            pts.push(Point::new(r.lon, r.lat));
        }
    }
    Ok(pts)
}

/// Extract exterior rings (lon,lat) + GEOID from each GeoJSON feature.
fn parse_block_groups(json: &[u8]) -> Result<Vec<(String, Vec<[f64; 2]>)>> {
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(json).context("parsing block-group GeoJSON")?;
    let mut out = Vec::new();
    for f in fc.features {
        let geoid = f
            .properties
            .as_ref()
            .and_then(|p| p.get("GEOID").or_else(|| p.get("GEOID20")))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let Some(geom) = f.geometry else { continue };
        let rings: Vec<Vec<[f64; 2]>> = match geom.value {
            geojson::Value::Polygon(rings) => rings
                .into_iter()
                .take(1) // exterior only
                .map(|ring| ring.iter().map(|p| [p[0], p[1]]).collect())
                .collect(),
            geojson::Value::MultiPolygon(polys) => polys
                .into_iter()
                .filter_map(|poly| poly.into_iter().next()) // exterior of each part
                .map(|ring| ring.iter().map(|p| [p[0], p[1]]).collect())
                .collect(),
            _ => continue,
        };
        for ring in rings {
            if ring.len() >= 4 {
                out.push((geoid.clone(), ring));
            }
        }
    }
    Ok(out)
}

pub fn bake(geojson_path: &str, acs_path: &str, dahir_csv: &str, out_path: &str) -> Result<usize> {
    let acs = parse_acs(&std::fs::read(acs_path).with_context(|| format!("reading {acs_path}"))?)?;
    let cameras = camera_points(dahir_csv)?;
    let rings = parse_block_groups(
        &std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?,
    )?;
    anyhow::ensure!(!rings.is_empty(), "no block groups parsed");

    let proj = EnuProjection::default();
    let mut block_groups = Vec::new();
    for (geoid, ring) in &rings {
        let a = acs.get(geoid).copied().unwrap_or_default();
        // Camera count via point-in-polygon (lon/lat space).
        let poly = Polygon::new(
            LineString::from(ring.iter().map(|p| Coord { x: p[0], y: p[1] }).collect::<Vec<_>>()),
            vec![],
        );
        let camera_count = cameras.iter().filter(|pt| poly.contains(*pt)).count() as u32;
        // Project ring to ENU for rendering.
        let exterior = ring.iter().map(|p| {
            let e = proj.to_enu(p[1], p[0]);
            [e.x, e.y]
        }).collect();

        block_groups.push(BlockGroup {
            geoid: geoid.clone(),
            exterior,
            entropy: a.entropy(),
            population: a.total as u32,
            camera_count,
        });
    }

    let total_cams: u32 = block_groups.iter().map(|b| b.camera_count).sum();
    let layer = EquityLayer {
        origin: GeoOrigin::MANHATTAN,
        block_groups,
        provenance: Provenance {
            source: "Census TIGER block groups + ACS 5-year (B03002) + Dahir camera detections".into(),
            url: "https://api.census.gov/data/2023/acs/acs5".into(),
            license: "Census public domain; cameras CC BY 4.0".into(),
            as_of: "2023 ACS / 2026 TIGER".into(),
            notes: "Shannon entropy over white/Black/Asian/Hispanic/other, per Dahir et al. \
                    Block-group aggregation only."
                .into(),
        },
    };
    let n = layer.block_groups.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("equity overlay: {n} block groups, {total_cams} cameras attributed -> {out_path}");
    Ok(n)
}
