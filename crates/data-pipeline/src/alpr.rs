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
    for el in resp.elements {
        let (lat, lon) = match (el.lat, el.lon) {
            (Some(a), Some(b)) => (a, b),
            _ => match el.center {
                Some(c) => (c.lat, c.lon),
                None => continue,
            },
        };
        // OSM `direction` is the compass bearing the reader faces (matches our
        // FrustumWedge heading). Many gantries carry several semicolon-separated
        // bearings (e.g. "28;209") for their multiple units — take the first as the
        // primary heading so these still render a directional FOV (not omnidirectional).
        let heading_deg = el
            .tags
            .get("direction")
            .and_then(|d| d.split(';').find_map(|p| p.trim().parse::<f64>().ok()));
        // Crowdsourced metadata for the modal + maker stratification. Trim and drop
        // empties so the UI can rely on `Some` meaning "actually labeled".
        let tag = |k: &str| {
            el.tags
                .get(k)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let p = proj.to_enu(lat, lon);
        readers.push(AlprReader {
            x: p.x,
            y: p.y,
            heading_deg,
            osm_id: el.id,
            manufacturer: tag("manufacturer").or_else(|| tag("brand")),
            operator: tag("operator").or_else(|| tag("operator:short")),
        });
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
    eprintln!("ALPR layer: {n} readers ({directional} with a heading, {with_maker} with a maker) -> {out_path}");
    Ok(n)
}
