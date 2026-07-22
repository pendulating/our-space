//! Bake the Institutions layer — schools, libraries, parks + plazas as fixed map
//! points (ENU meters), the *subjects* of surveillance in the "Institutions" view.
//!
//! Inputs:
//! - Schools + libraries: the NYC Facilities Database (Socrata `ji82-xba5`) as a JSON
//!   array of rows with `facname` / `latitude` / `longitude` / `boro` / `facgroup` /
//!   `factype` strings, pre-filtered to `facgroup in ('SCHOOLS (K-12)','LIBRARIES')`.
//!   These arrive as points and are used verbatim.
//! - Parks (optional): the Parks Properties GeoJSON (`enfh-gkve`) — `signname` /
//!   `typecategory` / `borough` (single-char code) / `acres` + polygon geometry. Folded
//!   in as the centroid of each park's largest polygon part; slivers (< [`MIN_PARK_ACRES`])
//!   are dropped so the view lists real parks, not gardens/traffic triangles.
//! - Plazas (optional): the Pedestrian Plazas GeoJSON (`k5k6-6jex`) — `plazaname` /
//!   `boroname` + polygon geometry. Folded in as each plaza's centroid.
//!
//! An optional borough name (e.g. `MANHATTAN`) keeps only that borough across all three
//! sources, matching the clipped street network in the default build.
//!
//! An institution is a *subject* of surveillance, not a sensor: it carries no FOV and
//! never enters the exposure model. The app ranks each by how many cameras sit within a
//! fixed radius of its point (its polygon centroid, for parks/plazas).

use anyhow::{Context, Result};
use serde::Deserialize;
use sim_core::assets::{Facility, FacilityKind, FacilityLayer, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

/// Parks smaller than this (acres) are dropped — they're community gardens, medians,
/// and traffic triangles, not the parks people picture. ~1 acre keeps real parks.
const MIN_PARK_ACRES: f64 = 1.0;

#[derive(Deserialize)]
struct Row {
    facname: Option<String>,
    latitude: Option<String>,
    longitude: Option<String>,
    boro: Option<String>,
    facgroup: Option<String>,
    factype: Option<String>,
}

/// Map the dataset's `facgroup` to our two point classes. Anything else is skipped.
fn classify(facgroup: &str) -> Option<FacilityKind> {
    match facgroup.trim().to_ascii_uppercase().as_str() {
        "SCHOOLS (K-12)" => Some(FacilityKind::School),
        "LIBRARIES" => Some(FacilityKind::Library),
        _ => None,
    }
}

/// Full borough name (as used in the facilities/plazas filters) → the single-char code
/// the Parks Properties dataset tags each park with.
fn borough_char(full: &str) -> Option<char> {
    match full.trim().to_ascii_uppercase().replace(['_', '-'], " ").as_str() {
        "MANHATTAN" | "M" => Some('M'),
        "BROOKLYN" | "B" => Some('B'),
        "QUEENS" | "Q" => Some('Q'),
        "BRONX" | "X" => Some('X'),
        "STATEN ISLAND" | "SI" | "R" => Some('R'),
        _ => None,
    }
}

/// Shoelace area (m²) and area-weighted centroid of an ENU ring; falls back to the
/// vertex mean if degenerate.
fn ring_area_centroid(ring: &[[f64; 2]]) -> (f64, [f64; 2]) {
    let n = ring.len();
    if n < 3 {
        let mx = ring.iter().map(|p| p[0]).sum::<f64>() / n.max(1) as f64;
        let my = ring.iter().map(|p| p[1]).sum::<f64>() / n.max(1) as f64;
        return (0.0, [mx, my]);
    }
    let (mut a, mut cx, mut cy) = (0.0, 0.0, 0.0);
    let mut j = n - 1;
    for i in 0..n {
        let cross = ring[j][0] * ring[i][1] - ring[i][0] * ring[j][1];
        a += cross;
        cx += (ring[j][0] + ring[i][0]) * cross;
        cy += (ring[j][1] + ring[i][1]) * cross;
        j = i;
    }
    if a.abs() < 1e-9 {
        let mx = ring.iter().map(|p| p[0]).sum::<f64>() / n as f64;
        let my = ring.iter().map(|p| p[1]).sum::<f64>() / n as f64;
        return (0.0, [mx, my]);
    }
    ((a * 0.5).abs(), [cx / (3.0 * a), cy / (3.0 * a)])
}

/// Exterior rings (lon/lat) of a GeoJSON geometry — the first ring of each polygon
/// part (holes ignored; a marker centroid doesn't need them).
fn exterior_rings(geom: &geojson::Geometry) -> Vec<Vec<[f64; 2]>> {
    let ring_of = |r: &Vec<Vec<f64>>| -> Vec<[f64; 2]> { r.iter().map(|p| [p[0], p[1]]).collect() };
    match &geom.value {
        geojson::Value::Polygon(rings) => rings.iter().take(1).map(ring_of).collect(),
        geojson::Value::MultiPolygon(polys) => {
            polys.iter().filter_map(|poly| poly.first()).map(ring_of).collect()
        }
        _ => vec![],
    }
}

/// Centroid (ENU) of the largest polygon part — the representative point for a park or
/// plaza's surveillance-proximity ranking.
fn largest_part_centroid(rings_lonlat: &[Vec<[f64; 2]>], proj: &EnuProjection) -> Option<[f64; 2]> {
    let mut best: Option<(f64, [f64; 2])> = None;
    for r in rings_lonlat {
        if r.len() < 3 {
            continue;
        }
        let enu: Vec<[f64; 2]> = r
            .iter()
            .map(|p| {
                let e = proj.to_enu(p[1], p[0]);
                [e.x, e.y]
            })
            .collect();
        let (area, c) = ring_area_centroid(&enu);
        if best.map(|(ba, _)| area > ba).unwrap_or(true) {
            best = Some((area, c));
        }
    }
    best.map(|(_, c)| c)
}

/// Read a GeoJSON FeatureCollection and append each qualifying feature's centroid as a
/// `Facility`. `str_prop` picks the name field; `subtype_of` derives the subtype; a
/// feature is kept only if `keep` passes (borough + size filters live there).
fn fold_polygons(
    path: &str,
    kind: FacilityKind,
    name_key: &str,
    proj: &EnuProjection,
    keep: impl Fn(&geojson::Feature) -> bool,
    subtype_of: impl Fn(&geojson::Feature) -> String,
    out: &mut Vec<Facility>,
) -> Result<usize> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {path}"))?;
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing GeoJSON {path}"))?;
    let mut added = 0usize;
    for f in &fc.features {
        if !keep(f) {
            continue;
        }
        let name = f
            .properties
            .as_ref()
            .and_then(|p| p.get(name_key))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if name.is_empty() {
            continue;
        }
        let Some(geom) = f.geometry.as_ref() else { continue };
        let Some(c) = largest_part_centroid(&exterior_rings(geom), proj) else { continue };
        out.push(Facility { x: c[0], y: c[1], name: name.to_string(), kind, subtype: subtype_of(f) });
        added += 1;
    }
    Ok(added)
}

pub fn bake(
    json_path: &str,
    out_path: &str,
    boro_filter: Option<&str>,
    parks_geojson: Option<&str>,
    plazas_geojson: Option<&str>,
) -> Result<usize> {
    let bytes = std::fs::read(json_path).with_context(|| format!("reading {json_path}"))?;
    let rows: Vec<Row> = serde_json::from_slice(&bytes).context("parsing Facilities JSON")?;
    let proj = EnuProjection::default();
    let want_boro = boro_filter.map(|b| b.trim().to_ascii_uppercase());
    let want_code = want_boro.as_deref().and_then(borough_char);

    // Schools + libraries (points, verbatim from the Facilities Database).
    let mut facilities = Vec::new();
    let mut dropped_boro = 0usize;
    let mut dropped_other = 0usize;
    for r in &rows {
        let (Some(name), Some(la), Some(lo), Some(fg)) =
            (&r.facname, &r.latitude, &r.longitude, &r.facgroup)
        else {
            dropped_other += 1;
            continue;
        };
        let Some(kind) = classify(fg) else {
            dropped_other += 1;
            continue;
        };
        if let Some(want) = &want_boro {
            let boro = r.boro.as_deref().unwrap_or("").trim().to_ascii_uppercase();
            if &boro != want {
                dropped_boro += 1;
                continue;
            }
        }
        let (Ok(lat), Ok(lon)) = (la.parse::<f64>(), lo.parse::<f64>()) else {
            dropped_other += 1;
            continue;
        };
        let enu = proj.to_enu(lat, lon);
        facilities.push(Facility {
            x: enu.x,
            y: enu.y,
            name: name.trim().to_string(),
            kind,
            subtype: r.factype.as_deref().unwrap_or("").trim().to_string(),
        });
    }
    let schools = facilities.iter().filter(|f| f.kind == FacilityKind::School).count();
    let libraries = facilities.iter().filter(|f| f.kind == FacilityKind::Library).count();

    // Parks (optional): polygon centroids, borough + acreage filtered.
    let parks = if let Some(p) = parks_geojson {
        fold_polygons(
            p,
            FacilityKind::Park,
            "signname",
            &proj,
            |f| {
                let props = f.properties.as_ref();
                let boro_ok = want_code.is_none_or(|code| {
                    props
                        .and_then(|p| p.get("borough"))
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.chars().next())
                        == Some(code)
                });
                let acres = props
                    .and_then(|p| p.get("acres"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                boro_ok && acres >= MIN_PARK_ACRES
            },
            |f| {
                f.properties
                    .as_ref()
                    .and_then(|p| p.get("typecategory"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Park")
                    .trim()
                    .to_string()
            },
            &mut facilities,
        )?
    } else {
        0
    };

    // Plazas (optional): polygon centroids, borough filtered. `boroname` is a full name
    // and may be comma-joined (e.g. "Brooklyn,Queens"), so match by substring.
    let plazas = if let Some(p) = plazas_geojson {
        let want_upper = want_boro.clone();
        fold_polygons(
            p,
            FacilityKind::Plaza,
            "plazaname",
            &proj,
            move |f| {
                want_upper.as_deref().is_none_or(|want| {
                    f.properties
                        .as_ref()
                        .and_then(|p| p.get("boroname"))
                        .and_then(|v| v.as_str())
                        .map(|b| b.to_ascii_uppercase().contains(want))
                        .unwrap_or(false)
                })
            },
            |_| "Pedestrian Plaza".to_string(),
            &mut facilities,
        )?
    } else {
        0
    };

    anyhow::ensure!(!facilities.is_empty(), "no facilities parsed");

    let layer = FacilityLayer {
        origin: GeoOrigin::MANHATTAN,
        facilities,
        provenance: Provenance {
            source: "NYC Facilities Database (ji82-xba5) + Parks Properties (enfh-gkve) + \
                     Pedestrian Plazas (k5k6-6jex)"
                .into(),
            url: "https://data.cityofnewyork.us/City-Government/Facilities-Database/ji82-xba5"
                .into(),
            license: "NYC Open Data terms of use".into(),
            as_of: "2025".into(),
            notes: "Schools (K-12), libraries, parks (>=1 acre) + pedestrian plazas; \
                    parks/plazas as polygon centroids. Institutions are subjects of \
                    surveillance, not sensors — ranked by nearby cameras."
                .into(),
        },
    };
    let n = layer.facilities.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "Facilities: {n} ({schools} schools, {libraries} libraries, {parks} parks, \
         {plazas} plazas; {dropped_boro} other-borough, {dropped_other} skipped) -> {out_path}"
    );
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::assets::{FacilityKind, FacilityLayer};

    #[test]
    fn bakes_classifies_and_filters_by_borough() {
        let json = r#"[
            {"facname":"PS 1","latitude":"40.715","longitude":"-73.998","boro":"MANHATTAN","facgroup":"SCHOOLS (K-12)","factype":"PUBLIC ELEMENTARY SCHOOL"},
            {"facname":"Branch Library","latitude":"40.72","longitude":"-73.99","boro":"MANHATTAN","facgroup":"LIBRARIES","factype":"BRANCH LIBRARY"},
            {"facname":"Brooklyn School","latitude":"40.69","longitude":"-73.99","boro":"BROOKLYN","facgroup":"SCHOOLS (K-12)","factype":"PUBLIC HIGH SCHOOL"},
            {"facname":"A Hospital","latitude":"40.71","longitude":"-73.99","boro":"MANHATTAN","facgroup":"HEALTH CARE","factype":"HOSPITAL"},
            {"facgroup":"LIBRARIES"}
        ]"#;
        let dir = std::env::temp_dir();
        let inp = dir.join("facilities_bake_test_in.json");
        let out = dir.join("facilities_bake_test_out.osfac");
        std::fs::write(&inp, json).unwrap();
        let n = bake(inp.to_str().unwrap(), out.to_str().unwrap(), Some("MANHATTAN"), None, None)
            .unwrap();
        // Only the two Manhattan school/library rows survive the borough + class filter.
        assert_eq!(n, 2);
        let layer = FacilityLayer::from_bytes(&std::fs::read(&out).unwrap()).unwrap();
        assert_eq!(layer.facilities.iter().filter(|f| f.kind == FacilityKind::School).count(), 1);
        assert_eq!(layer.facilities.iter().filter(|f| f.kind == FacilityKind::Library).count(), 1);
    }

    #[test]
    fn folds_parks_and_plazas_as_centroids_with_filters() {
        // One school, plus a park GeoJSON (a big Manhattan park, a sub-acre sliver that's
        // dropped, and a Brooklyn park filtered out by borough) and a Manhattan plaza.
        let facs = r#"[
            {"facname":"PS 1","latitude":"40.715","longitude":"-73.998","boro":"MANHATTAN","facgroup":"SCHOOLS (K-12)","factype":"PUBLIC ELEMENTARY SCHOOL"}
        ]"#;
        // ~0.011° square ≈ 1.2 km ≈ 140+ acres (kept); tiny square (dropped); Brooklyn (filtered).
        let parks = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"signname":"Big Park","typecategory":"Community Park","borough":"M","acres":"140"},
             "geometry":{"type":"Polygon","coordinates":[[[-73.99,40.71],[-73.979,40.71],[-73.979,40.721],[-73.99,40.721],[-73.99,40.71]]]}},
            {"type":"Feature","properties":{"signname":"Tiny Sliver","typecategory":"Triangle/Plaza","borough":"M","acres":"0.2"},
             "geometry":{"type":"Polygon","coordinates":[[[-73.99,40.71],[-73.9899,40.71],[-73.9899,40.7101],[-73.99,40.7101],[-73.99,40.71]]]}},
            {"type":"Feature","properties":{"signname":"Brooklyn Green","typecategory":"Community Park","borough":"B","acres":"50"},
             "geometry":{"type":"Polygon","coordinates":[[[-73.95,40.68],[-73.94,40.68],[-73.94,40.69],[-73.95,40.69],[-73.95,40.68]]]}}
        ]}"#;
        let plazas = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"plazaname":"Corner Plaza","boroname":"Manhattan"},
             "geometry":{"type":"MultiPolygon","coordinates":[[[[-73.988,40.716],[-73.987,40.716],[-73.987,40.717],[-73.988,40.717],[-73.988,40.716]]]]}}
        ]}"#;
        let dir = std::env::temp_dir();
        let fp = dir.join("fac_fold_facs.json");
        let pp = dir.join("fac_fold_parks.geojson");
        let zp = dir.join("fac_fold_plazas.geojson");
        let out = dir.join("fac_fold_out.osfac");
        std::fs::write(&fp, facs).unwrap();
        std::fs::write(&pp, parks).unwrap();
        std::fs::write(&zp, plazas).unwrap();
        let n = bake(
            fp.to_str().unwrap(),
            out.to_str().unwrap(),
            Some("MANHATTAN"),
            Some(pp.to_str().unwrap()),
            Some(zp.to_str().unwrap()),
        )
        .unwrap();
        // 1 school + 1 park (Big Park; sliver dropped, Brooklyn filtered) + 1 plaza.
        assert_eq!(n, 3);
        let layer = FacilityLayer::from_bytes(&std::fs::read(&out).unwrap()).unwrap();
        let park = layer.facilities.iter().find(|f| f.kind == FacilityKind::Park).unwrap();
        assert_eq!(park.name, "Big Park");
        assert_eq!(park.subtype, "Community Park");
        // Centroid sits inside the park's lon/lat box (projected to ENU near the origin).
        assert!(park.x.is_finite() && park.y.is_finite());
        assert_eq!(layer.facilities.iter().filter(|f| f.kind == FacilityKind::Plaza).count(), 1);
        assert_eq!(layer.facilities.iter().filter(|f| f.kind == FacilityKind::Park).count(), 1);
    }
}
