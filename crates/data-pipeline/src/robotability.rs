//! Bake the robotability field: a coarse ENU grid of the NYC "Robotability Score"
//! (IRL-CT robotability project) over the walk-graph extent. Drives where the
//! speculative sidewalk delivery robots spawn (per-node weight) and how dense their
//! exposure is along a route.
//!
//! Input: the baked walk graph (for the grid extent + projection) and the project's
//! `sidewalks.geojson` — an all-NYC FeatureCollection of short sidewalk `LineString`
//! segments, each with a `score` property in [0,1] (higher = more robot-friendly).
//! We bin each segment's centroid into a grid cell, average, then flood-fill empty
//! cells from their neighbors so the field is smooth and gap-free over Manhattan.

use anyhow::{Context, Result};
use sim_core::assets::{GraphAsset, Provenance, RobotabilityField};
use sim_core::graph::StreetGraph;
use sim_core::projection::{EnuProjection, GeoOrigin};

/// Grid cell size (m) — ~1 Manhattan block, matching the score's block-level scale.
const CELL_M: f64 = 100.0;
/// Pad the graph bbox so robots near the edge still see scored cells.
const MARGIN_M: f64 = 250.0;
/// Neighbor-average passes that fill cells with no sidewalk data.
const FILL_PASSES: usize = 8;

pub fn bake(graph_path: &str, geojson_path: &str, out_path: &str) -> Result<usize> {
    let proj = EnuProjection::default();

    // 1. Graph extent (Manhattan) → grid bounds.
    let g_bytes = std::fs::read(graph_path).with_context(|| format!("reading {graph_path}"))?;
    let graph = StreetGraph::from_asset(GraphAsset::from_bytes(&g_bytes).context("decoding graph")?);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for i in 0..graph.node_count() {
        let p = graph.node_pos(i as u32);
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    anyhow::ensure!(max_x > min_x, "empty graph extent");
    min_x -= MARGIN_M;
    min_y -= MARGIN_M;
    max_x += MARGIN_M;
    max_y += MARGIN_M;
    let cols = (((max_x - min_x) / CELL_M).ceil() as usize).max(1);
    let rows = (((max_y - min_y) / CELL_M).ceil() as usize).max(1);

    // 2. Parse sidewalk scores; bin each segment centroid into a cell (running mean).
    let json = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(&json).context("parsing sidewalks GeoJSON")?;
    let mut sum = vec![0.0f64; cols * rows];
    let mut cnt = vec![0u32; cols * rows];
    let (mut kept, total) = (0usize, fc.features.len());
    for f in &fc.features {
        let Some(score) = f
            .properties
            .as_ref()
            .and_then(|p| p.get("score"))
            .and_then(|v| v.as_f64())
        else {
            continue;
        };
        let Some(geom) = &f.geometry else { continue };
        let coords: Vec<&Vec<f64>> = match &geom.value {
            geojson::Value::LineString(l) => l.iter().collect(),
            geojson::Value::MultiLineString(ml) => ml.iter().flatten().collect(),
            _ => continue,
        };
        if coords.is_empty() {
            continue;
        }
        let (mut lon, mut lat, mut n) = (0.0, 0.0, 0.0);
        for c in &coords {
            if c.len() >= 2 {
                lon += c[0];
                lat += c[1];
                n += 1.0;
            }
        }
        if n == 0.0 {
            continue;
        }
        let e = proj.to_enu(lat / n, lon / n); // proj is to_enu(lat, lon)
        if e.x < min_x || e.x > max_x || e.y < min_y || e.y > max_y {
            continue; // outside the Manhattan grid (other boroughs)
        }
        let cx = (((e.x - min_x) / CELL_M).floor() as usize).min(cols - 1);
        let cy = (((e.y - min_y) / CELL_M).floor() as usize).min(rows - 1);
        let idx = cy * cols + cx;
        sum[idx] += score;
        cnt[idx] += 1;
        kept += 1;
    }
    anyhow::ensure!(kept > 0, "no sidewalk scores fell within the graph extent");

    // 3. Cell mean; -1 marks no data.
    let mut scores: Vec<f32> = (0..cols * rows)
        .map(|i| if cnt[i] > 0 { (sum[i] / cnt[i] as f64) as f32 } else { -1.0 })
        .collect();
    let filled_from_data = scores.iter().filter(|&&v| v >= 0.0).count();

    // 4. Flood-fill empty cells from their non-empty neighbors (a few passes).
    for _ in 0..FILL_PASSES {
        let snap = scores.clone();
        for cy in 0..rows {
            for cx in 0..cols {
                let i = cy * cols + cx;
                if snap[i] >= 0.0 {
                    continue;
                }
                let (mut s, mut c) = (0.0f64, 0u32);
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let nx = cx as i64 + dx;
                        let ny = cy as i64 + dy;
                        if nx < 0 || ny < 0 || nx as usize >= cols || ny as usize >= rows {
                            continue;
                        }
                        let v = snap[ny as usize * cols + nx as usize];
                        if v >= 0.0 {
                            s += v as f64;
                            c += 1;
                        }
                    }
                }
                if c > 0 {
                    scores[i] = (s / c as f64) as f32;
                }
            }
        }
    }

    let with_data = scores.iter().filter(|&&v| v >= 0.0).count();
    let (lo, hi) = scores.iter().filter(|&&v| v >= 0.0).fold((1.0f32, 0.0f32), |(lo, hi), &v| {
        (lo.min(v), hi.max(v))
    });

    let field = RobotabilityField {
        origin: GeoOrigin::MANHATTAN,
        min_x,
        min_y,
        cell_m: CELL_M,
        cols: cols as u32,
        rows: rows as u32,
        scores,
        provenance: Provenance {
            source: "IRL-CT Robotability Score (per-sidewalk), aggregated to a grid".into(),
            url: "https://github.com/IRL-CT/robotability".into(),
            license: "IRL-CT / Cornell Tech (research project)".into(),
            as_of: "robotability sidewalks.geojson".into(),
            notes: "Score 0..1 (higher = more robot-friendly); 19 AHP-survey-weighted \
                    features. Grid-binned segment centroids over the walk-graph extent."
                .into(),
        },
    };
    std::fs::write(out_path, field.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "robotability: {cols}x{rows} grid ({CELL_M:.0}m), {kept}/{total} segments kept, \
         {filled_from_data} cells w/ data → {with_data} after fill, score [{lo:.2},{hi:.2}] -> {out_path}"
    );
    Ok(kept)
}
