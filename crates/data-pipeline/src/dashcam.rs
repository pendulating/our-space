//! Bake the spatial rideshare-camera (dashcam) field from NYC TLC trip density.
//!
//! Inputs:
//!   - taxi-zone polygons GeoJSON (WGS84, keyed by `LocationID`)
//!   - per-zone rideshare trip counts CSV (`loc,trips`) from the HVFHV records
//!
//! NYC requires for-hire vehicles to carry cameras, so dashcam exposure follows
//! where Uber/Lyft actually drive. We compute trip *density* (trips / zone area)
//! and normalize to the median zone, giving each zone a relative intensity.

use std::collections::HashMap;

use anyhow::{Context, Result};
use sim_core::assets::{DashcamFieldLayer, DashcamZone, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

fn parse_trips(csv_path: &str) -> Result<HashMap<i64, f64>> {
    let mut rdr = csv::Reader::from_path(csv_path).with_context(|| format!("opening {csv_path}"))?;
    let mut map = HashMap::new();
    for rec in rdr.records() {
        let rec = rec?;
        let loc: i64 = rec.get(0).unwrap_or("").parse().unwrap_or(-1);
        let trips: f64 = rec.get(1).unwrap_or("0").parse().unwrap_or(0.0);
        if loc >= 0 {
            map.insert(loc, trips);
        }
    }
    Ok(map)
}

/// Per-feature: `LocationID` + its exterior rings (one per polygon part) in ENU.
fn parse_zones(json: &[u8], proj: &EnuProjection) -> Result<Vec<(i64, Vec<Vec<[f64; 2]>>)>> {
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(json).context("parsing taxi-zone GeoJSON")?;
    let to_enu = |ring: &[Vec<f64>]| -> Vec<[f64; 2]> {
        ring.iter()
            .filter(|p| p.len() >= 2)
            .map(|p| {
                let e = proj.to_enu(p[1], p[0]); // [lon, lat]
                [e.x, e.y]
            })
            .collect()
    };
    let mut out = Vec::new();
    for f in fc.features {
        let loc = f
            .properties
            .as_ref()
            .and_then(|p| p.get("LocationID"))
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x as i64)))
            .unwrap_or(-1);
        let Some(geom) = f.geometry else { continue };
        let rings: Vec<Vec<[f64; 2]>> = match geom.value {
            geojson::Value::Polygon(p) => p.iter().take(1).map(|r| to_enu(r)).collect(),
            geojson::Value::MultiPolygon(mp) => {
                mp.iter().filter_map(|poly| poly.first()).map(|r| to_enu(r)).collect()
            }
            _ => continue,
        };
        if loc >= 0 {
            out.push((loc, rings));
        }
    }
    Ok(out)
}

fn shoelace_area(ring: &[[f64; 2]]) -> f64 {
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

fn bbox(ring: &[[f64; 2]]) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in ring {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    }
    b
}

pub fn bake(geojson_path: &str, trips_csv: &str, out_path: &str) -> Result<usize> {
    let proj = EnuProjection::default();
    let trips = parse_trips(trips_csv)?;
    let zones = parse_zones(&std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?, &proj)?;
    anyhow::ensure!(!zones.is_empty(), "no taxi zones parsed");

    // Total area per zone (summing multipolygon parts).
    let mut total_area: HashMap<i64, f64> = HashMap::new();
    for (loc, rings) in &zones {
        for r in rings {
            *total_area.entry(*loc).or_default() += shoelace_area(r);
        }
    }

    // Median trip density (trips / m²) over zones with data — robust to airport outliers.
    let mut densities: Vec<f64> = total_area
        .iter()
        .filter_map(|(loc, &area)| {
            let t = *trips.get(loc).unwrap_or(&0.0);
            (area > 0.0 && t > 0.0).then_some(t / area)
        })
        .collect();
    densities.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = densities.get(densities.len() / 2).copied().unwrap_or(1.0).max(1e-12);

    let mut out_zones = Vec::new();
    let mut max_intensity = 0.0_f64;
    for (loc, rings) in zones {
        let area = total_area.get(&loc).copied().unwrap_or(0.0);
        let t = trips.get(&loc).copied().unwrap_or(0.0);
        let intensity = if area > 0.0 {
            ((t / area) / median).clamp(0.0, 8.0)
        } else {
            0.0
        };
        max_intensity = max_intensity.max(intensity);
        for ring in rings {
            if ring.len() >= 3 {
                out_zones.push(DashcamZone {
                    bbox: bbox(&ring),
                    exterior: ring,
                    intensity,
                });
            }
        }
    }

    let n = out_zones.len();
    let layer = DashcamFieldLayer {
        origin: GeoOrigin::MANHATTAN,
        zones: out_zones,
        provenance: Provenance {
            source: "NYC TLC High-Volume FHV trip records (rideshare) + NYC taxi zones".into(),
            url: "https://www.nyc.gov/site/tlc/about/tlc-trip-record-data.page".into(),
            license: "NYC OpenData / TLC terms".into(),
            as_of: "2024-12".into(),
            notes: "Per-zone Uber/Lyft trip density (PU+DO), normalized to the median zone; \
                    dashcams ride in for-hire vehicles the city requires to carry cameras."
                .into(),
        },
    };
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("dashcam field: {n} zone parts, max intensity {max_intensity:.1}× median -> {out_path}");
    Ok(n)
}
