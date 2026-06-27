//! Bake a borough's boundary outline (ENU rings) from the NYC Borough Boundaries
//! dataset, for the visual frame drawn around the street network.
//!
//! Input: the city's Borough Boundaries GeoJSON (data.cityofnewyork.us `gthc-hcne`),
//! a FeatureCollection of five `MultiPolygon`s keyed by a `boroname` property.
//!
//! For each borough we keep its **main landmass** plus any detached part at least
//! [`MIN_PART_AREA_FRAC`] of the main's area — so Manhattan's Roosevelt, Randalls/Wards
//! and Governors islands draw, the Bronx's City Island, Queens' Rockaway peninsula,
//! etc., while harbor specks are dropped. Rings are emitted **main landmasses first**
//! (one per borough, in borough order) and detached parts after, so the app's
//! offshore-label anchor (`rings[0]` = Manhattan) and the citywide footprint loader's
//! ring-`i` ↔ borough-`i` mapping both still hold.

use anyhow::{Context, Result};
use sim_core::assets::{BoroughOutline, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

/// Shoelace area of a lon/lat ring (relative magnitude only — for picking the
/// largest part; absolute units don't matter at this latitude).
fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    let mut j = n - 1;
    for i in 0..n {
        s += (ring[j][0] + ring[i][0]) * (ring[j][1] - ring[i][1]);
        j = i;
    }
    (s * 0.5).abs()
}

/// The five NYC borough names, for the citywide (`nyc`/`all`/`city`) mode.
const ALL_BOROUGHS: &[&str] = &["Manhattan", "Bronx", "Brooklyn", "Queens", "Staten Island"];

/// Keep a detached polygon part if its area is at least this fraction of the
/// borough's main-landmass area. 0.3% comfortably includes Manhattan's Roosevelt
/// (~1%), Randalls/Wards (~4.6%) and Governors (~1.2%) islands while dropping harbor
/// specks (U Thant / Mill Rock, well under 0.1%).
const MIN_PART_AREA_FRAC: f64 = 0.003;

/// Read a borough feature's significant landmass parts — the main landmass plus any
/// detached part ≥ [`MIN_PART_AREA_FRAC`] of the main's area — in lon/lat, **sorted
/// largest-first** (so element 0 is always the main landmass).
fn significant_parts(fc: &geojson::FeatureCollection, borough: &str) -> Result<Vec<Vec<[f64; 2]>>> {
    let want = borough.to_ascii_lowercase();
    let feature = fc
        .features
        .iter()
        .find(|f| {
            f.properties
                .as_ref()
                .and_then(|p| p.get("boroname").or_else(|| p.get("boro_name")))
                .and_then(|v| v.as_str())
                .map(|s| s.to_ascii_lowercase() == want)
                .unwrap_or(false)
        })
        .with_context(|| format!("borough {borough:?} not found"))?;

    let geom = feature
        .geometry
        .as_ref()
        .context("borough feature has no geometry")?;
    // Collect every polygon part's exterior ring (lon/lat).
    let mut parts: Vec<Vec<[f64; 2]>> = match &geom.value {
        geojson::Value::MultiPolygon(polys) => polys
            .iter()
            .filter_map(|poly| poly.first()) // exterior ring of each part
            .map(|ring| ring.iter().map(|p| [p[0], p[1]]).collect())
            .collect(),
        geojson::Value::Polygon(rings) => rings
            .iter()
            .take(1)
            .map(|ring| ring.iter().map(|p| [p[0], p[1]]).collect())
            .collect(),
        _ => anyhow::bail!("borough geometry is not a (Multi)Polygon"),
    };
    anyhow::ensure!(!parts.is_empty(), "no polygon parts for {borough}");

    // Largest part first.
    parts.sort_by(|a, b| {
        ring_area(b)
            .partial_cmp(&ring_area(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    anyhow::ensure!(parts[0].len() >= 4, "degenerate main-island ring for {borough}");

    // Keep the main landmass + detached parts above the area threshold.
    let main_a = ring_area(&parts[0]);
    let threshold = main_a * MIN_PART_AREA_FRAC;
    // deg² → approx km² at NYC's latitude (1° lat ≈ 111 km, 1° lon ≈ 84.4 km).
    let km2 = |a: f64| a * 111.0 * 84.4;
    let kept: Vec<Vec<[f64; 2]>> = parts
        .into_iter()
        .filter(|r| r.len() >= 4 && ring_area(r) >= threshold)
        .collect();
    eprintln!(
        "  {borough}: kept {} part(s) — {}",
        kept.len(),
        kept.iter()
            .map(|r| format!("{:.2}km²", km2(ring_area(r))))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(kept)
}

pub fn bake(geojson_path: &str, borough: &str, out_path: &str) -> Result<usize> {
    let bytes = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(&bytes).context("parsing borough GeoJSON")?;
    let proj = EnuProjection::default();

    // `nyc`/`all`/`city` → one outline holding all five boroughs' main-landmass
    // rings (the app draws every ring and clips sensors to "inside any ring", so a
    // multi-ring outline is the citywide frame + the citywide camera clip in one).
    let citywide = matches!(borough.to_ascii_lowercase().as_str(), "nyc" | "all" | "city");
    let (names, display): (Vec<&str>, String) = if citywide {
        (ALL_BOROUGHS.to_vec(), "New York City".to_string())
    } else {
        (vec![borough], borough.to_string())
    };

    // Each borough's significant parts (main landmass first, then detached islands).
    let mut per_boro: Vec<Vec<Vec<[f64; 2]>>> = Vec::with_capacity(names.len());
    for name in &names {
        per_boro.push(
            significant_parts(&fc, name).with_context(|| format!("reading {geojson_path}"))?,
        );
    }

    // Emit every borough's MAIN landmass first (in borough order), then all detached
    // parts. This keeps `rings[0]` = Manhattan (the offshore-label anchor) and the
    // first N rings = the N boroughs' main landmasses in order (the citywide footprint
    // loader maps ring `i` → borough region `i`); islands ride along after that.
    let mut rings_ll: Vec<Vec<[f64; 2]>> = Vec::new();
    for parts in &per_boro {
        rings_ll.push(parts[0].clone());
    }
    for parts in &per_boro {
        rings_ll.extend(parts.iter().skip(1).cloned());
    }

    // Project lon/lat → ENU meters (canonical origin; matches every other layer).
    let rings: Vec<Vec<[f64; 2]>> = rings_ll
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|p| {
                    let e = proj.to_enu(p[1], p[0]); // GeoJSON is [lon, lat]
                    [e.x, e.y]
                })
                .collect()
        })
        .collect();

    let n_detached = rings.len() - names.len();
    let notes = if citywide {
        format!(
            "Five-borough outlines (Manhattan, Bronx, Brooklyn, Queens, Staten Island), \
             main landmasses first then {n_detached} detached parts ≥{:.1}% of their borough \
             (Roosevelt, Randalls/Wards, Governors, City Island, Rockaway, …).",
            MIN_PART_AREA_FRAC * 100.0
        )
    } else {
        format!(
            "{display} outline: main landmass + {n_detached} detached part(s) \
             ≥{:.1}% of the main.",
            MIN_PART_AREA_FRAC * 100.0
        )
    };
    let layer = BoroughOutline {
        origin: GeoOrigin::MANHATTAN,
        name: display.clone(),
        rings,
        provenance: Provenance {
            source: "NYC Borough Boundaries (Department of City Planning)".into(),
            url: "https://data.cityofnewyork.us/City-Government/Borough-Boundaries/gthc-hcne".into(),
            license: "NYC Open Data — public domain".into(),
            as_of: "2024".into(),
            notes,
        },
    };
    let pts = layer.rings.iter().map(|r| r.len()).sum::<usize>();
    let n_rings = layer.rings.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("borough outline: {display}, {n_rings} ring(s) / {pts} points -> {out_path}");
    Ok(pts)
}
