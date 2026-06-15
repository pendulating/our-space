//! Bake the fixed-CCTV layer from the Dahir et al. Stanford deposit.
//!
//! Source: `map_data.csv` from purl.stanford.edu/jr882ny4955 (CC BY 4.0).
//! Columns: `panoid,heading,lat,lon,city,year,month,camera_count`.
//!
//! Each row is a Google Street View *sample point*; `camera_count >= 1` means a
//! camera was detected as visible from that panorama (NOT a surveyed device
//! location). We keep New York rows within a rough Manhattan bounding box,
//! dedupe by panoid, and carry the detector recall (~0.63) so the app can show
//! an honest uncertainty band rather than claim exact camera positions.

use std::collections::HashSet;

use anyhow::Context;
use sim_core::assets::{FixedSensorData, FixedSensorLayer, Provenance};
use sim_core::exposure::SourceKind;
use sim_core::projection::{EnuProjection, GeoOrigin};

/// Detector recall reported by Dahir et al.
const RECALL: f64 = 0.63;

#[derive(Debug, serde::Deserialize)]
struct Row {
    panoid: String,
    heading: f64,
    lat: f64,
    lon: f64,
    city: String,
    #[allow(dead_code)]
    year: i32,
    #[allow(dead_code)]
    month: i32,
    camera_count: i32,
}

/// Loose Manhattan bounding box (keeps the island + immediate waterfront,
/// excludes the other boroughs present in the NYC rows).
fn in_manhattan_bbox(lat: f64, lon: f64) -> bool {
    (40.698..=40.882).contains(&lat) && (-74.022..=-73.906).contains(&lon)
}

pub fn bake(csv_path: &str, out_path: &str) -> anyhow::Result<usize> {
    let proj = EnuProjection::default();
    let mut rdr = csv::Reader::from_path(csv_path)
        .with_context(|| format!("opening {csv_path}"))?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut sensors = Vec::new();
    let (mut ny_rows, mut detections) = (0usize, 0usize);

    for rec in rdr.deserialize::<Row>() {
        let row = rec.context("parsing map_data.csv row")?;
        if row.city != "New York" {
            continue;
        }
        ny_rows += 1;
        if row.camera_count < 1 {
            continue;
        }
        detections += 1;
        if !in_manhattan_bbox(row.lat, row.lon) {
            continue;
        }
        if !seen.insert(row.panoid.clone()) {
            continue;
        }
        let p = proj.to_enu(row.lat, row.lon);
        sensors.push(FixedSensorData {
            x: p.x,
            y: p.y,
            // GSV capture heading: an approximation of the camera's bearing.
            heading_deg: Some(row.heading),
            kind: SourceKind::FixedCctv,
        });
    }

    let layer = FixedSensorLayer {
        origin: GeoOrigin::MANHATTAN,
        sensors,
        recall: Some(RECALL),
        provenance: Provenance {
            source: "Dahir et al. 2025, Stanford Digital Repository (map_data.csv)".into(),
            url: "https://purl.stanford.edu/jr882ny4955".into(),
            license: "CC BY 4.0".into(),
            as_of: "2025".into(),
            notes: "GSV sample-points where a camera was detected (recall ~0.63); \
                    panorama-level coordinates, not surveyed devices."
                .into(),
        },
    };

    let count = layer.sensors.len();
    std::fs::write(out_path, layer.to_bytes()?)
        .with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "Dahir: {ny_rows} NY sample-points, {detections} detections, {count} unique Manhattan cameras baked -> {out_path}"
    );
    Ok(count)
}
