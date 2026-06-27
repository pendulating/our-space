//! Bake the Tesla-camera field: normalized private-Tesla registration density by ZIP
//! (NYS DMV), as polygon zones over NYC. Teslas run always-on cameras (Sentry when
//! parked + Autopilot while driving), so exposure follows where Teslas are garaged /
//! driven — a residential pattern distinct from the rideshare-dashcam field.
//!
//! Inputs: a NYC ZIP-polygon GeoJSON (WGS84, `postalCode` per feature) and a `zip,n`
//! CSV of Tesla registration counts per ZIP (NYS DMV `w4pv-hbkt`, filtered to
//! make='TESLA' and the five NYC counties).

use std::collections::HashMap;

use anyhow::{Context, Result};
use sim_core::assets::{DashcamZone, Provenance, TeslaField};
use sim_core::projection::{EnuProjection, GeoOrigin};

use crate::dashcam::{bbox, shoelace_area};

fn parse_counts(csv_path: &str) -> Result<HashMap<String, f64>> {
    let mut rdr = csv::Reader::from_path(csv_path).with_context(|| format!("opening {csv_path}"))?;
    let mut map = HashMap::new();
    for rec in rdr.records() {
        let rec = rec?;
        let zip = rec.get(0).unwrap_or("").trim().to_string();
        let n: f64 = rec.get(1).unwrap_or("0").trim().parse().unwrap_or(0.0);
        if !zip.is_empty() && n > 0.0 {
            map.insert(zip, n);
        }
    }
    Ok(map)
}

/// Per-feature: `postalCode` + exterior rings (one per polygon part) in ENU.
fn parse_zip_polys(json: &[u8], proj: &EnuProjection) -> Result<Vec<(String, Vec<Vec<[f64; 2]>>)>> {
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(json).context("parsing ZIP GeoJSON")?;
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
        let zip = f
            .properties
            .as_ref()
            .and_then(|p| p.get("postalCode"))
            .and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| v.as_i64().map(|x| x.to_string())))
            .unwrap_or_default();
        let Some(geom) = f.geometry else { continue };
        let rings: Vec<Vec<[f64; 2]>> = match geom.value {
            geojson::Value::Polygon(p) => p.iter().take(1).map(|r| to_enu(r)).collect(),
            geojson::Value::MultiPolygon(mp) => {
                mp.iter().filter_map(|poly| poly.first()).map(|r| to_enu(r)).collect()
            }
            _ => continue,
        };
        if !zip.is_empty() {
            out.push((zip, rings));
        }
    }
    Ok(out)
}

pub fn bake(geojson_path: &str, counts_csv: &str, out_path: &str) -> Result<usize> {
    let proj = EnuProjection::default();
    let counts = parse_counts(counts_csv)?;
    let zips = parse_zip_polys(
        &std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?,
        &proj,
    )?;
    anyhow::ensure!(!zips.is_empty(), "no ZIP polygons parsed");

    // Area per ZIP (sum parts).
    let mut total_area: HashMap<String, f64> = HashMap::new();
    for (zip, rings) in &zips {
        for r in rings {
            *total_area.entry(zip.clone()).or_default() += shoelace_area(r);
        }
    }

    // Median Tesla density (registrations / m²) over ZIPs with data.
    let mut dens: Vec<f64> = total_area
        .iter()
        .filter_map(|(zip, &a)| {
            let c = *counts.get(zip).unwrap_or(&0.0);
            (a > 0.0 && c > 0.0).then_some(c / a)
        })
        .collect();
    dens.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = dens.get(dens.len() / 2).copied().unwrap_or(1.0).max(1e-12);

    let mut out_zones = Vec::new();
    let (mut matched, mut max_i) = (0usize, 0.0f64);
    for (zip, rings) in zips {
        let area = total_area.get(&zip).copied().unwrap_or(0.0);
        let c = counts.get(&zip).copied().unwrap_or(0.0);
        if c > 0.0 {
            matched += 1;
        }
        let intensity = if area > 0.0 { ((c / area) / median).clamp(0.0, 8.0) } else { 0.0 };
        max_i = max_i.max(intensity);
        for ring in rings {
            if ring.len() >= 3 {
                out_zones.push(DashcamZone { bbox: bbox(&ring), exterior: ring, intensity });
            }
        }
    }

    let n = out_zones.len();
    let field = TeslaField {
        origin: GeoOrigin::MANHATTAN,
        zones: out_zones,
        provenance: Provenance {
            source: "NYS DMV Vehicle Registrations (make=TESLA, NYC counties) by ZIP".into(),
            url: "https://data.ny.gov/d/w4pv-hbkt".into(),
            license: "NY Open Data terms".into(),
            as_of: "DMV registrations snapshot".into(),
            notes: "Private Tesla registration density per ZIP, normalized to the median NYC ZIP. \
                    Teslas run always-on Sentry/Autopilot cameras. Commercial Teslas (~7% of TLC \
                    for-hire vehicles) are additionally represented by the rideshare-dashcam layer."
                .into(),
        },
    };
    std::fs::write(out_path, field.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "tesla field: {n} ZIP zone parts ({matched} ZIPs w/ registrations), max {max_i:.1}× median -> {out_path}"
    );
    Ok(n)
}
