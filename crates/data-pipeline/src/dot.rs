//! Bake the NYC DOT traffic-camera layer from the Traffic Management Center feed.
//!
//! Source: the public camera list at `nyctmc.org/map`, served as JSON from
//! `https://webcams.nyctmc.org/api/cameras/`. Each record carries an `id`,
//! `name`, `latitude`, `longitude`, `area`, `isOnline`, and an `imageUrl`.
//!
//! IMPORTANT — legal/ethics guardrail: the DOT feed has **no open license**, and
//! DOT has objected to reuse of the camera *images*. We therefore ingest **only
//! the published coordinates** (location is the modeled quantity) and discard the
//! `imageUrl` entirely — no image is fetched, stored, or redistributed. These are
//! modeled as a low-frame-rate *monitoring* class (Tier A: mapped locations).

use std::collections::HashSet;

use anyhow::{Context, Result};
use serde::Deserialize;
use sim_core::assets::{FixedSensorData, FixedSensorLayer, Provenance};
use sim_core::exposure::SourceKind;
use sim_core::projection::{EnuProjection, GeoOrigin};

#[derive(Deserialize)]
struct Cam {
    id: String,
    #[serde(default)]
    area: String,
    latitude: f64,
    longitude: f64,
    #[serde(rename = "isOnline", default)]
    is_online: String,
    // `name` and `imageUrl` are intentionally NOT deserialized: we keep only the
    // location, never the image or its URL.
}

/// Loose Manhattan bounding box (matches the Dahir/CCTV baker), as a second
/// guard alongside the feed's `area` field.
fn in_manhattan_bbox(lat: f64, lon: f64) -> bool {
    (40.698..=40.882).contains(&lat) && (-74.022..=-73.906).contains(&lon)
}

pub fn bake(json_path: &str, out_path: &str) -> Result<usize> {
    let proj = EnuProjection::default();
    let bytes = std::fs::read(json_path).with_context(|| format!("reading {json_path}"))?;
    let cams: Vec<Cam> = serde_json::from_slice(&bytes).context("parsing DOT cameras JSON")?;

    let total = cams.len();
    let mut seen: HashSet<String> = HashSet::new();
    let mut sensors = Vec::new();
    let mut offline = 0usize;
    for c in cams {
        if c.area != "Manhattan" || !in_manhattan_bbox(c.latitude, c.longitude) {
            continue;
        }
        // An offline camera isn't capturing; skip it (status fluctuates, so this
        // is a snapshot-time filter, noted in provenance).
        if c.is_online.eq_ignore_ascii_case("false") {
            offline += 1;
            continue;
        }
        if !seen.insert(c.id.clone()) {
            continue;
        }
        let p = proj.to_enu(c.latitude, c.longitude);
        sensors.push(FixedSensorData {
            x: p.x,
            y: p.y,
            // The feed publishes no bearing; PTZ cameras pan, so omnidirectional.
            heading_deg: None,
            kind: SourceKind::DotLiveView,
        });
    }
    anyhow::ensure!(!sensors.is_empty(), "no Manhattan DOT cameras parsed");

    let layer = FixedSensorLayer {
        origin: GeoOrigin::MANHATTAN,
        sensors,
        recall: None, // mapped device locations, not detections
        provenance: Provenance {
            source: "NYC DOT Traffic Management Center traffic cameras (nyctmc.org) — locations only".into(),
            url: "https://nyctmc.org/map".into(),
            license: "No open license; camera locations only (images not used or redistributed)".into(),
            as_of: "2026-06-16".into(),
            notes: "Public PTZ traffic-monitoring cameras; we ingest only published \
                    coordinates, never the images. Online-at-snapshot only. Modeled \
                    as a low-frame-rate monitoring class."
                .into(),
        },
    };

    let n = layer.sensors.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "DOT cameras: {total} citywide, {n} Manhattan online baked ({offline} offline skipped) -> {out_path}"
    );
    Ok(n)
}
