//! Bake the NYC neighborhood boundary layer (Pedia Cities) into ENU polygons.
//!
//! Geometry only — the app aggregates fixed-camera counts per neighborhood at
//! runtime (it already holds every sensor + an R-tree). All five boroughs are
//! baked; the app renders Manhattan by default and toggles the rest.
//!
//! Input: a GeoJSON FeatureCollection with `neighborhood` + `borough` string
//! properties (e.g. `custom-pedia-cities-nyc-Mar2018.geojson`).

use anyhow::{Context, Result};
use sim_core::assets::{Neighborhood, NeighborhoodLayer, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

pub fn bake(geojson_path: &str, out_path: &str) -> Result<usize> {
    let bytes = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(&bytes).context("parsing neighborhoods GeoJSON")?;

    let proj = EnuProjection::default();
    let mut neighborhoods = Vec::new();
    for f in fc.features {
        let props = f.properties.as_ref();
        let name = props
            .and_then(|p| p.get("neighborhood").or_else(|| p.get("name")))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let borough = props
            .and_then(|p| p.get("borough"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let Some(geom) = f.geometry else { continue };
        // Exterior ring(s): Polygon → first ring; MultiPolygon → each part's exterior.
        let rings: Vec<Vec<[f64; 2]>> = match geom.value {
            geojson::Value::Polygon(rings) => rings
                .into_iter()
                .take(1)
                .map(|ring| ring.iter().map(|p| [p[0], p[1]]).collect())
                .collect(),
            geojson::Value::MultiPolygon(polys) => polys
                .into_iter()
                .filter_map(|poly| poly.into_iter().next())
                .map(|ring| ring.iter().map(|p| [p[0], p[1]]).collect())
                .collect(),
            _ => continue,
        };
        for ring in rings {
            if ring.len() < 4 {
                continue;
            }
            // Project lon/lat → ENU meters (mirrors equity.rs).
            let exterior: Vec<[f64; 2]> = ring
                .iter()
                .map(|p| {
                    let e = proj.to_enu(p[1], p[0]);
                    [e.x, e.y]
                })
                .collect();
            let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for p in &exterior {
                minx = minx.min(p[0]);
                miny = miny.min(p[1]);
                maxx = maxx.max(p[0]);
                maxy = maxy.max(p[1]);
            }
            neighborhoods.push(Neighborhood {
                name: name.clone(),
                borough: borough.clone(),
                exterior,
                bbox: [minx, miny, maxx, maxy],
            });
        }
    }
    anyhow::ensure!(!neighborhoods.is_empty(), "no neighborhoods parsed");

    let manhattan = neighborhoods.iter().filter(|n| n.borough == "Manhattan").count();
    let layer = NeighborhoodLayer {
        origin: GeoOrigin::MANHATTAN,
        neighborhoods,
        provenance: Provenance {
            source: "Pedia Cities NYC neighborhoods (custom)".into(),
            url: "https://github.com/HodgesWardElliott/custom-nyc-neighborhoods".into(),
            license: "Pedia Cities / data.beta.nyc — see source repo".into(),
            as_of: "2018-03".into(),
            notes: "312 neighborhoods across 5 boroughs; East Williamsburg + Hamilton \
                    Heights amended. Geometry only; per-neighborhood camera counts \
                    aggregated at runtime."
                .into(),
        },
    };
    let n = layer.neighborhoods.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("neighborhoods: {n} polygons ({manhattan} Manhattan) -> {out_path}");
    Ok(n)
}
