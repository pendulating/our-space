//! Bake the unified fixed-CCTV layer by **aggregating and de-duplicating** two
//! independent Google-Street-View camera censuses:
//!
//! 1. **Amnesty International, Decode Surveillance NYC** — crowdsourced camera
//!    counts per intersection (median over 3 volunteers), the
//!    `counts_per_intersections.csv` aggregate. This is the dominant, far more
//!    complete source (≈4,266 Manhattan cameras at ≈2,019 intersections).
//!    License: CC BY-NC-ND 4.0 (per the project methodology note). This project
//!    is non-commercial/research (satisfies NC) and attributes the source.
//! 2. **Dahir et al. 2025** — ML-detected camera points (`map_data.csv`, CC BY
//!    4.0), sampled along street panoramas, so it catches mid-block cameras the
//!    intersection-only Amnesty sampling missed.
//!
//! Both detect the **same physical camera population** from street view, so they
//! cannot simply be summed. We take Amnesty as the base census and add only the
//! Dahir cameras that fall **outside** `DEDUP_RADIUS_M` of any Amnesty
//! intersection that already reports cameras — i.e. Dahir's genuinely-additional
//! detections. The merged layer carries `recall: None`: the headline is a direct
//! crowdsourced census, not the recall-corrected Dahir estimate.

use std::collections::HashSet;

use anyhow::{Context, Result};
use serde::Deserialize;
use sim_core::assets::{FixedSensorData, FixedSensorLayer, Provenance};
use sim_core::exposure::SourceKind;
use sim_core::projection::{EnuProjection, GeoOrigin};

/// A Dahir camera within this distance of an Amnesty intersection that already
/// reports ≥1 camera is treated as the same corner's cameras and dropped.
/// Intersection-scale: Manhattan corners are ~80–200 m apart.
const DEDUP_RADIUS_M: f64 = 50.0;

/// Loose Manhattan bounding box (matches the other fixed-camera bakers).
fn in_manhattan_bbox(lat: f64, lon: f64) -> bool {
    (40.698..=40.882).contains(&lat) && (-74.022..=-73.906).contains(&lon)
}

/// One row of Amnesty `counts_per_intersections.csv` (only the fields we use;
/// the CSV's other columns are ignored by the deserializer).
#[derive(Deserialize)]
struct AmnestyRow {
    // Some rows have empty numeric fields (panorama not resolved); treat as absent.
    n_cameras_median: Option<f64>,
    #[serde(rename = "Lat")]
    lat: Option<f64>,
    #[serde(rename = "Long")]
    lon: Option<f64>,
    #[serde(rename = "BoroName")]
    boro: String,
}

/// One row of Dahir `map_data.csv`.
#[derive(Deserialize)]
struct DahirRow {
    panoid: String,
    heading: f64,
    lat: f64,
    lon: f64,
    city: String,
    camera_count: i32,
}

pub fn bake(amnesty_csv: &str, dahir_csv: &str, out_path: &str) -> Result<usize> {
    let proj = EnuProjection::default();

    // --- 1. Amnesty: Manhattan intersections reporting ≥1 camera. Each becomes
    // `n` omnidirectional cameras at the panorama point (the aggregate has no
    // per-camera bearing). Track the intersection points for de-duplication. ---
    let mut sensors = Vec::new();
    let mut amnesty_points: Vec<(f64, f64)> = Vec::new(); // ENU
    let mut amnesty_cams = 0usize;
    let mut rdr = csv::Reader::from_path(amnesty_csv)
        .with_context(|| format!("opening {amnesty_csv}"))?;
    for rec in rdr.deserialize::<AmnestyRow>() {
        let row = rec.context("parsing counts_per_intersections.csv row")?;
        let (Some(lat), Some(lon)) = (row.lat, row.lon) else { continue };
        if row.boro != "Manhattan" || !in_manhattan_bbox(lat, lon) {
            continue;
        }
        let n = row.n_cameras_median.unwrap_or(0.0).round() as i64;
        if n < 1 {
            continue;
        }
        let p = proj.to_enu(lat, lon);
        amnesty_points.push((p.x, p.y));
        for _ in 0..n {
            sensors.push(FixedSensorData {
                x: p.x,
                y: p.y,
                heading_deg: None, // intersection-aggregated: omnidirectional
                kind: SourceKind::FixedCctv,
            });
            amnesty_cams += 1;
        }
    }
    anyhow::ensure!(!amnesty_points.is_empty(), "no Manhattan Amnesty intersections parsed");

    // --- 2. Dahir: keep only detections that don't duplicate an Amnesty corner. ---
    let r2 = DEDUP_RADIUS_M * DEDUP_RADIUS_M;
    let mut seen: HashSet<String> = HashSet::new();
    let (mut dahir_total, mut dahir_kept) = (0usize, 0usize);
    let mut rdr = csv::Reader::from_path(dahir_csv)
        .with_context(|| format!("opening {dahir_csv}"))?;
    for rec in rdr.deserialize::<DahirRow>() {
        let row = rec.context("parsing map_data.csv row")?;
        if row.city != "New York" || row.camera_count < 1 {
            continue;
        }
        if !in_manhattan_bbox(row.lat, row.lon) || !seen.insert(row.panoid.clone()) {
            continue;
        }
        dahir_total += 1;
        let p = proj.to_enu(row.lat, row.lon);
        let duplicate = amnesty_points
            .iter()
            .any(|&(ax, ay)| (p.x - ax).powi(2) + (p.y - ay).powi(2) <= r2);
        if duplicate {
            continue;
        }
        dahir_kept += 1;
        sensors.push(FixedSensorData {
            x: p.x,
            y: p.y,
            heading_deg: Some(row.heading), // Dahir carries a GSV capture bearing
            kind: SourceKind::FixedCctv,
        });
    }

    let layer = FixedSensorLayer {
        origin: GeoOrigin::MANHATTAN,
        sensors,
        // Merged census dominated by Amnesty's direct counts — the Dahir ML
        // detector recall (~0.63) does not apply to the set, so no correction.
        recall: None,
        provenance: Provenance {
            source: "Amnesty International, Decode Surveillance NYC (crowdsourced camera census) \
                     aggregated with Dahir et al. 2025 (street-view-detected cameras), de-duplicated".into(),
            url: "https://github.com/amnesty-crisis-evidence-lab/decode-surveillance-nyc".into(),
            license: "Amnesty data CC BY-NC-ND 4.0 (non-commercial use, attributed); Dahir et al. CC BY 4.0".into(),
            as_of: "Amnesty 2021–22 survey (GSV 2019–20) · Dahir 2025".into(),
            notes: "Per-intersection median camera counts (Amnesty) placed omnidirectional at the \
                    panorama point, plus Dahir detections >50 m from any Amnesty camera-bearing \
                    intersection. Sample-point estimates, not surveyed device coordinates."
                .into(),
        },
    };

    let n = layer.sensors.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "Fixed CCTV (merged): {amnesty_cams} Amnesty + {dahir_kept}/{dahir_total} Dahir-unique \
         = {n} cameras -> {out_path}"
    );
    Ok(n)
}
