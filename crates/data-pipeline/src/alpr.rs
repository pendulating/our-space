//! Bake the ALPR (automated license-plate reader) layer from DeFlock / OSM.
//!
//! DeFlock's crowdsourced ALPR points are synced into OpenStreetMap as
//! `man_made=surveillance` + `surveillance:type=ALPR`. We fetch them via Overpass
//! and bake them as a fixed-sensor layer (kind=Alpr). These are mapped device
//! locations (no recall correction), most carrying a `direction` heading.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;
use sim_core::assets::{FixedSensorData, FixedSensorLayer, Provenance};
use sim_core::exposure::SourceKind;
use sim_core::projection::{EnuProjection, GeoOrigin};

#[derive(Deserialize)]
struct OverpassResp {
    elements: Vec<El>,
}

#[derive(Deserialize)]
struct El {
    lat: Option<f64>,
    lon: Option<f64>,
    center: Option<Center>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Center {
    lat: f64,
    lon: f64,
}

pub fn bake(json_path: &str, out_path: &str) -> Result<usize> {
    let proj = EnuProjection::default();
    let bytes = std::fs::read(json_path).with_context(|| format!("reading {json_path}"))?;
    let resp: OverpassResp = serde_json::from_slice(&bytes).context("parsing ALPR Overpass JSON")?;

    let mut sensors = Vec::new();
    for el in resp.elements {
        let (lat, lon) = match (el.lat, el.lon) {
            (Some(a), Some(b)) => (a, b),
            _ => match el.center {
                Some(c) => (c.lat, c.lon),
                None => continue,
            },
        };
        // OSM `direction` is the compass bearing the reader faces (matches our
        // FrustumWedge heading). Non-numeric values -> omnidirectional.
        let heading_deg = el.tags.get("direction").and_then(|d| d.parse::<f64>().ok());
        let p = proj.to_enu(lat, lon);
        sensors.push(FixedSensorData {
            x: p.x,
            y: p.y,
            heading_deg,
            kind: SourceKind::Alpr,
        });
    }
    anyhow::ensure!(!sensors.is_empty(), "no ALPR points parsed");

    let directional = sensors.iter().filter(|s| s.heading_deg.is_some()).count();
    let layer = FixedSensorLayer {
        origin: GeoOrigin::MANHATTAN,
        sensors,
        recall: None, // mapped points, not detections
        provenance: Provenance {
            source: "DeFlock crowdsourced ALPRs via OpenStreetMap (man_made=surveillance, surveillance:type=ALPR)".into(),
            url: "https://deflock.me".into(),
            license: "ODbL 1.0".into(),
            as_of: "2026-06-15".into(),
            notes: "License-plate readers (Flock + NYC agency systems); crowdsourced, coverage incomplete.".into(),
        },
    };
    let n = layer.sensors.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("ALPR layer: {n} readers ({directional} with a heading) -> {out_path}");
    Ok(n)
}
