//! Bake ACE bus-camera corridors from MTA GTFS + the official ACE route list.
//!
//! Inputs:
//!   - a GTFS static directory (routes.txt, trips.txt, shapes.txt)
//!   - ki2b-sg5y "MTA Bus Automated Camera Enforced Routes" JSON (data.ny.gov)
//!
//! ACE cameras are mounted on the buses, so capture follows the route geometry.
//! We match the ACE route list to GTFS routes by their `M<digits>` base (ki2b
//! "M15+" ↔ GTFS "M15-SBS"), collect those routes' shapes, and emit ENU segments.

use std::collections::{HashMap, HashSet};

use anyhow::Context;
use sim_core::assets::{AceCorridorLayer, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

#[derive(serde::Deserialize)]
struct AceRow {
    route: String,
    program: String,
}

/// Manhattan-route base key: leading 'M' + run of digits ("M15+" -> "M15",
/// "M14A-SBS" -> "M14"). Returns None for non-Manhattan / unparseable routes.
fn manhattan_base(name: &str) -> Option<String> {
    let mut chars = name.chars();
    if chars.next()? != 'M' {
        return None;
    }
    let digits: String = name[1..].chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        Some(format!("M{digits}"))
    }
}

pub fn bake(gtfs_dir: &str, ace_json: &str, out_path: &str) -> anyhow::Result<(usize, usize)> {
    // 1. ACE route bases (Manhattan, Program == ACE).
    let ace_bytes = std::fs::read(ace_json).with_context(|| format!("reading {ace_json}"))?;
    let ace_rows: Vec<AceRow> = serde_json::from_slice(&ace_bytes).context("parsing ACE JSON")?;
    let ace_bases: HashSet<String> = ace_rows
        .iter()
        .filter(|r| r.program.eq_ignore_ascii_case("ACE"))
        .filter_map(|r| manhattan_base(&r.route))
        .collect();
    anyhow::ensure!(!ace_bases.is_empty(), "no Manhattan ACE routes found");

    // 2. GTFS routes whose base is ACE -> route_ids + the matched short names.
    let mut ace_route_ids: HashSet<String> = HashSet::new();
    let mut matched_routes: Vec<String> = Vec::new();
    let mut rdr = csv::Reader::from_path(format!("{gtfs_dir}/routes.txt"))
        .with_context(|| format!("opening {gtfs_dir}/routes.txt"))?;
    let headers = rdr.headers()?.clone();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let (c_id, c_short) = (
        col("route_id").context("route_id col")?,
        col("route_short_name").context("route_short_name col")?,
    );
    for rec in rdr.records() {
        let rec = rec?;
        let short = rec.get(c_short).unwrap_or("");
        if manhattan_base(short).is_some_and(|b| ace_bases.contains(&b)) {
            ace_route_ids.insert(rec.get(c_id).unwrap_or("").to_string());
            matched_routes.push(short.to_string());
        }
    }
    matched_routes.sort();
    matched_routes.dedup();

    // 3. trips.txt -> shape_ids used by ACE routes.
    let mut shape_ids: HashSet<String> = HashSet::new();
    let mut rdr = csv::Reader::from_path(format!("{gtfs_dir}/trips.txt"))?;
    let headers = rdr.headers()?.clone();
    let (c_rid, c_shape) = (
        headers.iter().position(|h| h == "route_id").context("trips route_id")?,
        headers.iter().position(|h| h == "shape_id").context("trips shape_id")?,
    );
    for rec in rdr.records() {
        let rec = rec?;
        if ace_route_ids.contains(rec.get(c_rid).unwrap_or("")) {
            let s = rec.get(c_shape).unwrap_or("");
            if !s.is_empty() {
                shape_ids.insert(s.to_string());
            }
        }
    }

    // 4. shapes.txt -> ordered points per ACE shape.
    let mut shapes: HashMap<String, Vec<(u32, f64, f64)>> = HashMap::new();
    let mut rdr = csv::Reader::from_path(format!("{gtfs_dir}/shapes.txt"))?;
    let headers = rdr.headers()?.clone();
    let (c_sid, c_lat, c_lon, c_seq) = (
        headers.iter().position(|h| h == "shape_id").context("shape_id")?,
        headers.iter().position(|h| h == "shape_pt_lat").context("shape_pt_lat")?,
        headers.iter().position(|h| h == "shape_pt_lon").context("shape_pt_lon")?,
        headers.iter().position(|h| h == "shape_pt_sequence").context("shape_pt_sequence")?,
    );
    for rec in rdr.records() {
        let rec = rec?;
        let sid = rec.get(c_sid).unwrap_or("");
        if !shape_ids.contains(sid) {
            continue;
        }
        let lat: f64 = rec.get(c_lat).unwrap_or("0").parse().unwrap_or(0.0);
        let lon: f64 = rec.get(c_lon).unwrap_or("0").parse().unwrap_or(0.0);
        let seq: u32 = rec.get(c_seq).unwrap_or("0").parse().unwrap_or(0);
        shapes.entry(sid.to_string()).or_default().push((seq, lat, lon));
    }

    // 5. Project to ENU and emit segments.
    let proj = EnuProjection::default();
    let mut segments: Vec<[[f64; 2]; 2]> = Vec::new();
    for pts in shapes.values_mut() {
        pts.sort_by_key(|(seq, _, _)| *seq);
        for w in pts.windows(2) {
            let a = proj.to_enu(w[0].1, w[0].2);
            let b = proj.to_enu(w[1].1, w[1].2);
            segments.push([[a.x, a.y], [b.x, b.y]]);
        }
    }
    anyhow::ensure!(!segments.is_empty(), "no ACE shape geometry found");

    let layer = AceCorridorLayer {
        origin: GeoOrigin::MANHATTAN,
        segments,
        routes: matched_routes.clone(),
        provenance: Provenance {
            source: "MTA GTFS (route geometry) + data.ny.gov ki2b-sg5y (ACE route list)".into(),
            url: "https://data.ny.gov/d/ki2b-sg5y".into(),
            license: "MTA / OPEN-NY Terms of Use".into(),
            as_of: "2026-06-14".into(),
            notes: format!(
                "Manhattan ACE routes ({}); enforcement hours assumed all-day.",
                matched_routes.join(", ")
            ),
        },
    };
    let (segs, routes) = (layer.segments.len(), layer.routes.len());
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "ACE corridors: {routes} routes, {segs} segments -> {out_path}\n  routes: {}",
        matched_routes.join(", ")
    );
    Ok((routes, segs))
}
