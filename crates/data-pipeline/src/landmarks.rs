//! Bake the curated landmark LoD2 massings (from `tools/extract_landmarks.py`'s
//! JSON) into an ENU asset the app renders as recognizable 2.5D buildings.
//!
//! Input JSON: `{ landmarks: [ { name, bin, height_m, surfaces: [ { type, ring:
//! [[lon,lat,height_above_base_m], ...] } ] } ] }`. We project lon/lat → ENU, keep
//! the height as-is, drop ground surfaces (hidden under everything), and record a
//! base-footprint centroid as the label/sort anchor.

use anyhow::{Context, Result};
use serde::Deserialize;
use sim_core::assets::{Landmark, LandmarkMassing, LandmarkSurface, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

#[derive(Deserialize)]
struct InSurface {
    #[serde(rename = "type")]
    ty: String,
    ring: Vec<[f64; 3]>, // [lon, lat, height_above_base_m]
}
#[derive(Deserialize)]
struct InLandmark {
    name: String,
    #[allow(dead_code)]
    bin: String,
    height_m: f64,
    surfaces: Vec<InSurface>,
}
#[derive(Deserialize)]
struct InFile {
    landmarks: Vec<InLandmark>,
}

pub fn bake(json_path: &str, out_path: &str) -> Result<usize> {
    let bytes = std::fs::read(json_path).with_context(|| format!("reading {json_path}"))?;
    let parsed: InFile = serde_json::from_slice(&bytes).context("parsing landmarks JSON")?;
    let proj = EnuProjection::default();

    let mut landmarks: Vec<Landmark> = Vec::new();
    for lm in parsed.landmarks {
        let mut surfaces: Vec<LandmarkSurface> = Vec::new();
        // Base-footprint centroid: average of near-ground vertices (h < 2 m).
        let (mut bx, mut by, mut bn) = (0.0f64, 0.0f64, 0usize);
        for s in &lm.surfaces {
            let kind = match s.ty.as_str() {
                "RoofSurface" => 1u8,
                "WallSurface" => 0u8,
                _ => continue, // drop GroundSurface
            };
            let verts: Vec<[f32; 3]> = s
                .ring
                .iter()
                .map(|p| {
                    let e = proj.to_enu(p[1], p[0]);
                    if p[2] < 2.0 {
                        bx += e.x;
                        by += e.y;
                        bn += 1;
                    }
                    [e.x as f32, e.y as f32, p[2] as f32]
                })
                .collect();
            if verts.len() >= 3 {
                surfaces.push(LandmarkSurface { kind, verts });
            }
        }
        if surfaces.is_empty() {
            continue;
        }
        let anchor = if bn > 0 {
            [(bx / bn as f64) as f32, (by / bn as f64) as f32]
        } else {
            // Fallback: mean of all vertices.
            let (mut ax, mut ay, mut an) = (0.0f32, 0.0f32, 0u32);
            for s in &surfaces {
                for v in &s.verts {
                    ax += v[0];
                    ay += v[1];
                    an += 1;
                }
            }
            [ax / an as f32, ay / an as f32]
        };
        landmarks.push(Landmark {
            name: lm.name,
            anchor,
            height_m: lm.height_m as f32,
            surfaces,
        });
    }
    anyhow::ensure!(!landmarks.is_empty(), "no landmarks parsed");

    let layer = LandmarkMassing {
        origin: GeoOrigin::MANHATTAN,
        landmarks,
        provenance: Provenance {
            source: "NYC 3D Building Model, LoD2 CityGML (OTI/DoITT)".into(),
            url: "https://www.nyc.gov/site/doitt/initiatives/3d-building.page".into(),
            license: "NYC Open Data — public domain".into(),
            as_of: "2016".into(),
            notes: "Curated landmark massings; oblique-rendered for orientation.".into(),
        },
    };
    let n = layer.landmarks.len();
    let surfs: usize = layer.landmarks.iter().map(|l| l.surfaces.len()).sum();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("landmarks: {n} buildings, {surfs} surfaces -> {out_path}");
    Ok(n)
}
