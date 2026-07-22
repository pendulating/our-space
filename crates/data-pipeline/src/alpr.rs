//! Bake the ALPR (automated license-plate reader) layer from DeFlock / OSM.
//!
//! DeFlock's crowdsourced ALPR points are synced into OpenStreetMap as
//! `man_made=surveillance` + `surveillance:type=ALPR`. We fetch them via Overpass
//! and bake them as an `AlprReaderLayer` (kind=Alpr in the exposure model). These are
//! mapped device locations (no recall correction), most carrying a `direction` heading
//! plus `manufacturer`/`operator` tags we surface in the per-camera modal + maker
//! stratification, and the OSM node id for deep-links to openstreetmap.org / deflock.me.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;
use sim_core::assets::{AlprReader, AlprReaderLayer, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

#[derive(Deserialize)]
struct OverpassResp {
    elements: Vec<El>,
}

#[derive(Deserialize)]
struct El {
    id: u64,
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

    let mut readers = Vec::new();
    let mut devices = 0usize; // OSM elements that produced >=1 sensor (a gantry may emit several)
    for el in resp.elements {
        let (lat, lon) = match (el.lat, el.lon) {
            (Some(a), Some(b)) => (a, b),
            _ => match el.center {
                Some(c) => (c.lat, c.lon),
                None => continue,
            },
        };
        // Crowdsourced metadata for the modal + maker stratification. Trim and drop
        // empties so the UI can rely on `Some` meaning "actually labeled".
        let tag = |k: &str| {
            el.tags
                .get(k)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let manufacturer = tag("manufacturer").or_else(|| tag("brand"));
        let operator = tag("operator").or_else(|| tag("operator:short"));
        // OSM `direction` is the compass bearing the reader faces (matches our
        // FrustumWedge heading). A gantry often carries several semicolon-separated
        // bearings (e.g. "28;209") -- one physical reader unit per direction. Emit ONE
        // SENSOR PER DISTINCT BEARING at the shared coordinate, so a two-way gantry
        // renders both wedges instead of one. (Numeric bearings only; a cardinal token
        // this parser doesn't read falls through to omnidirectional, as before.)
        let mut bearings: Vec<f64> = Vec::new();
        if let Some(dir) = el.tags.get("direction") {
            for tok in dir.split(';') {
                if let Ok(b) = tok.trim().parse::<f64>() {
                    if !bearings.contains(&b) {
                        bearings.push(b); // distinct, order-preserving
                    }
                }
            }
        }
        let p = proj.to_enu(lat, lon);
        devices += 1;
        // No numeric bearing -> a single omnidirectional sensor (unchanged behavior).
        let headings: Vec<Option<f64>> = if bearings.is_empty() {
            vec![None]
        } else {
            bearings.into_iter().map(Some).collect()
        };
        for heading_deg in headings {
            readers.push(AlprReader {
                x: p.x,
                y: p.y,
                heading_deg,
                osm_id: el.id,
                manufacturer: manufacturer.clone(),
                operator: operator.clone(),
            });
        }
    }
    anyhow::ensure!(!readers.is_empty(), "no ALPR points parsed");

    let directional = readers.iter().filter(|r| r.heading_deg.is_some()).count();
    let with_maker = readers.iter().filter(|r| r.manufacturer.is_some()).count();
    let layer = AlprReaderLayer {
        origin: GeoOrigin::MANHATTAN,
        readers,
        provenance: Provenance {
            source: "DeFlock crowdsourced ALPRs via OpenStreetMap (man_made=surveillance, surveillance:type=ALPR)".into(),
            url: "https://deflock.me".into(),
            license: "ODbL 1.0".into(),
            as_of: "2026-06-15".into(),
            notes: "License-plate readers (Flock + NYC agency systems); crowdsourced, coverage incomplete. Carries OSM node id + manufacturer/operator tags where mapped.".into(),
        },
    };
    let n = layer.readers.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "ALPR layer: {n} sensors from {devices} devices ({directional} directional, \
         {with_maker} with a maker) -> {out_path}"
    );
    Ok(n)
}
