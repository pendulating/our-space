//! Bake the LinkNYC kiosk layer — Wi-Fi/phone hubs as fixed map points (ENU meters).
//!
//! Input: the NYC "LinkNYC Kiosk Status" dataset (Socrata `n6c5-95xh`) as a JSON array
//! of rows with `latitude` / `longitude` / `status` strings. Optionally clipped to the
//! borough main-island boundary so it lines up with the clipped streets.
//!
//! A kiosk is *not* a camera: it watches you only when you connect to its Wi-Fi, so it
//! carries no FOV and never enters the exposure model — the app frames it narratively.

use anyhow::{Context, Result};
use serde::Deserialize;
use sim_core::assets::{LinkNycKiosk, LinkNycLayer, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

#[derive(Deserialize)]
struct Row {
    latitude: Option<String>,
    longitude: Option<String>,
    status: Option<String>,
}

pub fn bake(json_path: &str, out_path: &str, boundary_geojson: Option<&str>) -> Result<usize> {
    let bytes = std::fs::read(json_path).with_context(|| format!("reading {json_path}"))?;
    let rows: Vec<Row> = serde_json::from_slice(&bytes).context("parsing LinkNYC JSON")?;
    let proj = EnuProjection::default();
    let boundary = boundary_geojson
        .map(crate::boundary::ManhattanBoundary::load)
        .transpose()?;

    let mut kiosks = Vec::new();
    let mut dropped = 0usize;
    for r in &rows {
        let (Some(la), Some(lo)) = (&r.latitude, &r.longitude) else {
            continue;
        };
        let (Ok(lat), Ok(lon)) = (la.parse::<f64>(), lo.parse::<f64>()) else {
            continue;
        };
        let enu = proj.to_enu(lat, lon);
        if let Some(b) = &boundary {
            if !b.contains([enu.x, enu.y]) {
                dropped += 1;
                continue;
            }
        }
        kiosks.push(LinkNycKiosk {
            x: enu.x,
            y: enu.y,
            wifi_live: r.status.as_deref() == Some("Live"),
        });
    }
    anyhow::ensure!(!kiosks.is_empty(), "no LinkNYC kiosks parsed");

    let layer = LinkNycLayer {
        origin: GeoOrigin::MANHATTAN,
        kiosks,
        provenance: Provenance {
            source: "NYC LinkNYC Kiosk Status (n6c5-95xh)".into(),
            url: "https://data.cityofnewyork.us/City-Government/LinkNYC-Kiosk-Status/n6c5-95xh"
                .into(),
            license: "NYC Open Data terms of use".into(),
            as_of: "2025".into(),
            notes: "Manhattan LinkNYC Wi-Fi/phone kiosks; positions only (not cameras — \
                    surveillance is conditional on connecting to the kiosk Wi-Fi)."
                .into(),
        },
    };
    let n = layer.kiosks.len();
    let live = layer.kiosks.iter().filter(|k| k.wifi_live).count();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("LinkNYC: {n} kiosks ({live} live Wi-Fi, {dropped} off-island dropped) -> {out_path}");
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::assets::LinkNycLayer;

    #[test]
    fn bakes_kiosks_skips_missing_coords_and_flags_live() {
        let json = r#"[
            {"latitude":"40.7484","longitude":"-73.9857","status":"Live"},
            {"latitude":"40.75","longitude":"-73.99","status":"Repair"},
            {"status":"Live"}
        ]"#;
        let dir = std::env::temp_dir();
        let inp = dir.join("linknyc_bake_test_in.json");
        let out = dir.join("linknyc_bake_test_out.oslink");
        std::fs::write(&inp, json).unwrap();
        let n = bake(inp.to_str().unwrap(), out.to_str().unwrap(), None).unwrap();
        assert_eq!(n, 2, "the coord-less row is skipped");
        let layer = LinkNycLayer::from_bytes(&std::fs::read(&out).unwrap()).unwrap();
        assert_eq!(layer.kiosks.len(), 2);
        assert!(layer.kiosks[0].wifi_live, "Live status → wifi_live");
        assert!(!layer.kiosks[1].wifi_live, "Repair status → not wifi_live");
        let _ = std::fs::remove_file(&inp);
        let _ = std::fs::remove_file(&out);
    }
}
