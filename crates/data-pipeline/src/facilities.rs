//! Bake the Institutions layer — schools + libraries as fixed map points (ENU meters).
//!
//! Input: the NYC Facilities Database (Socrata `ji82-xba5`) as a JSON array of rows
//! with `facname` / `latitude` / `longitude` / `boro` / `facgroup` / `factype`
//! strings, pre-filtered to `facgroup in ('SCHOOLS (K-12)','LIBRARIES')`. An optional
//! borough name (e.g. `MANHATTAN`) keeps only that borough's facilities, matching the
//! clipped street network in the default build.
//!
//! An institution is a *subject* of surveillance, not a sensor: it carries no FOV and
//! never enters the exposure model. The app ranks each by how many cameras sit nearby.

use anyhow::{Context, Result};
use serde::Deserialize;
use sim_core::assets::{Facility, FacilityKind, FacilityLayer, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

#[derive(Deserialize)]
struct Row {
    facname: Option<String>,
    latitude: Option<String>,
    longitude: Option<String>,
    boro: Option<String>,
    facgroup: Option<String>,
    factype: Option<String>,
}

/// Map the dataset's `facgroup` to our two classes. Anything else is skipped.
fn classify(facgroup: &str) -> Option<FacilityKind> {
    match facgroup.trim().to_ascii_uppercase().as_str() {
        "SCHOOLS (K-12)" => Some(FacilityKind::School),
        "LIBRARIES" => Some(FacilityKind::Library),
        _ => None,
    }
}

pub fn bake(json_path: &str, out_path: &str, boro_filter: Option<&str>) -> Result<usize> {
    let bytes = std::fs::read(json_path).with_context(|| format!("reading {json_path}"))?;
    let rows: Vec<Row> = serde_json::from_slice(&bytes).context("parsing Facilities JSON")?;
    let proj = EnuProjection::default();
    let want_boro = boro_filter.map(|b| b.trim().to_ascii_uppercase());

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
    anyhow::ensure!(!facilities.is_empty(), "no facilities parsed");

    let schools = facilities.iter().filter(|f| f.kind == FacilityKind::School).count();
    let libraries = facilities.len() - schools;
    let layer = FacilityLayer {
        origin: GeoOrigin::MANHATTAN,
        facilities,
        provenance: Provenance {
            source: "NYC Facilities Database (ji82-xba5)".into(),
            url: "https://data.cityofnewyork.us/City-Government/Facilities-Database/ji82-xba5"
                .into(),
            license: "NYC Open Data terms of use".into(),
            as_of: "2025".into(),
            notes: "Schools (K-12) and libraries; positions only. Institutions are \
                    subjects of surveillance, not sensors — ranked by nearby cameras."
                .into(),
        },
    };
    let n = layer.facilities.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "Facilities: {n} ({schools} schools, {libraries} libraries; \
         {dropped_boro} other-borough, {dropped_other} skipped) -> {out_path}"
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
        let n = bake(inp.to_str().unwrap(), out.to_str().unwrap(), Some("MANHATTAN")).unwrap();
        // Only the two Manhattan school/library rows survive the borough + class filter.
        assert_eq!(n, 2);
        let layer = FacilityLayer::from_bytes(&std::fs::read(&out).unwrap()).unwrap();
        assert_eq!(layer.facilities.iter().filter(|f| f.kind == FacilityKind::School).count(), 1);
        assert_eq!(layer.facilities.iter().filter(|f| f.kind == FacilityKind::Library).count(), 1);
    }
}
