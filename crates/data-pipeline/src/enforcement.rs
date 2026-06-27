//! Bake the automated photo-enforcement camera layer (speed / bus-lane / red-light)
//! from NYC DOT "PHOTO ENFORCED" street signs (Street Sign Work Orders, qt6m-xctn).
//!
//! Input: a deduped `lon,lat,subtype` CSV — the signs' state-plane coordinates
//! (EPSG:2263) converted to WGS84 and collapsed to distinct locations during fetch.
//! Each becomes a fixed, omnidirectional sensor (`kind=EnforcementCamera`, Tier A).
//! These mark enforcement corridors/zones, not exact camera mounts.

use anyhow::{Context, Result};
use sim_core::assets::{FixedSensorData, FixedSensorLayer, Provenance};
use sim_core::exposure::SourceKind;
use sim_core::projection::{EnuProjection, GeoOrigin};

pub fn bake(csv_path: &str, out_path: &str) -> Result<usize> {
    let proj = EnuProjection::default();
    let mut rdr = csv::Reader::from_path(csv_path).with_context(|| format!("opening {csv_path}"))?;
    let mut sensors = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        let lon: f64 = rec.get(0).unwrap_or("").trim().parse().unwrap_or(f64::NAN);
        let lat: f64 = rec.get(1).unwrap_or("").trim().parse().unwrap_or(f64::NAN);
        if !lon.is_finite() || !lat.is_finite() {
            continue;
        }
        let p = proj.to_enu(lat, lon);
        sensors.push(FixedSensorData {
            x: p.x,
            y: p.y,
            heading_deg: None, // signs don't encode the camera's aim → omnidirectional
            kind: SourceKind::EnforcementCamera,
        });
    }
    anyhow::ensure!(!sensors.is_empty(), "no enforcement signs parsed");

    let n = sensors.len();
    let layer = FixedSensorLayer {
        origin: GeoOrigin::MANHATTAN,
        sensors,
        recall: None, // mapped from posted signage, not street-view detections
        provenance: Provenance {
            source: "NYC DOT automated photo-enforcement signage (Street Sign Work Orders qt6m-xctn, Current)".into(),
            url: "https://data.cityofnewyork.us/d/qt6m-xctn".into(),
            license: "NYC OpenData terms".into(),
            as_of: "Street Sign Work Orders (Current)".into(),
            notes: "Speed + bus-lane + red-light camera locations from posted 'PHOTO ENFORCED' / \
                    'CAMERA' signs, deduped to distinct sign locations. Signs mark enforcement \
                    corridors/zones, not exact camera mounts (approximate; over-counts vs cameras)."
                .into(),
        },
    };
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("enforcement camera layer: {n} sign locations -> {out_path}");
    Ok(n)
}
