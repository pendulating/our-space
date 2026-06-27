//! Bake NYC parks (Parks Properties, enfh-gkve) into flat ENU polygon rings — a
//! green context layer rendered under the street network, like the building fabric.
//!
//! Reuses the [`BuildingFootprints`] payload (a bag of flat exterior rings); the app
//! loads it under its own `ParksRes`/`.ospark` extension and tints it green. An
//! optional borough filter (`M`/`B`/`Q`/`X`/`R`) keeps the Manhattan build clean
//! while the citywide build bakes all five.
//!
//! Input: the Parks Properties GeoJSON (enfh-gkve) — `multipolygon` geometry with a
//! `borough` property.

use anyhow::{Context, Result};
use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon, Simplify};
use sim_core::assets::{BuildingFootprints, Provenance};
use sim_core::projection::{EnuProjection, GeoOrigin};

use crate::footprints::{rdp, ring_area_m2};

/// Shoreline simplification tolerance (degrees ≈ 5 m). The clip only has to keep
/// parks off open water; meter-exact coastline is invisible under a soft green wash,
/// and decimating the dense DCP boundary (3 MB) is what makes the citywide clip — one
/// boolean op per park against the borough land — finish in seconds instead of minutes.
const LAND_SIMPLIFY_DEG: f64 = 0.00005;

/// Axis-aligned bbox `[minx, miny, maxx, maxy]` of a lon/lat polygon's exterior.
fn poly_bbox(p: &Polygon<f64>) -> [f64; 4] {
    let (mut a, mut b, mut c, mut d) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for co in p.exterior().coords() {
        a = a.min(co.x);
        b = b.min(co.y);
        c = c.max(co.x);
        d = d.max(co.y);
    }
    [a, b, c, d]
}

fn bbox_overlap(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1]
}

/// The land clip mask: each borough/island as a simplified polygon with its bbox, so a
/// park only runs the (expensive) boolean op against the land it actually overlaps.
type LandMask = Vec<([f64; 4], Polygon<f64>)>;

/// Load all borough land (the shoreline-clipped NYC DCP boundaries) as the clip mask
/// in lon/lat — so no park spills into open water (the Parks Properties extents
/// overshoot the bank on e.g. Randall's/Ward's Island).
fn load_land(geojson_path: &str) -> Result<LandMask> {
    let bytes = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(&bytes).context("parsing borough-boundary GeoJSON")?;
    let ring_ls = |r: &Vec<Vec<f64>>| {
        LineString::from(r.iter().map(|p| Coord { x: p[0], y: p[1] }).collect::<Vec<_>>())
    };
    let mut polys: Vec<Polygon<f64>> = Vec::new();
    for f in fc.features {
        let Some(geom) = f.geometry else { continue };
        let mut push = |rings: &Vec<Vec<Vec<f64>>>| {
            if let Some(ext) = rings.first() {
                let holes = rings.iter().skip(1).map(&ring_ls).collect();
                polys.push(Polygon::new(ring_ls(ext), holes));
            }
        };
        match geom.value {
            geojson::Value::Polygon(rings) => push(&rings),
            geojson::Value::MultiPolygon(mp) => mp.iter().for_each(|rings| push(rings)),
            _ => {}
        }
    }
    anyhow::ensure!(!polys.is_empty(), "no borough land polygons in {geojson_path}");
    Ok(polys
        .into_iter()
        .map(|p| {
            let s = p.simplify(&LAND_SIMPLIFY_DEG);
            (poly_bbox(&s), s)
        })
        .collect())
}

/// Clip one park ring (lon/lat) to the land mask, returning the exterior ring(s) of
/// the intersection. Only the land polygons whose bbox overlaps the park are tested.
/// Falls back to the original ring if the boolean op panics on degenerate input (so a
/// fragile polygon is kept whole rather than dropped).
fn clip_ring_to_land(ring: &[[f64; 2]], land: &LandMask) -> Vec<Vec<[f64; 2]>> {
    let subj = Polygon::new(
        LineString::from(ring.iter().map(|p| Coord { x: p[0], y: p[1] }).collect::<Vec<_>>()),
        vec![],
    );
    let pb = poly_bbox(&subj);
    let near = MultiPolygon(
        land.iter()
            .filter(|(b, _)| bbox_overlap(&pb, b))
            .map(|(_, p)| p.clone())
            .collect::<Vec<_>>(),
    );
    if near.0.is_empty() {
        return Vec::new(); // park far from any land → entirely in water
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| subj.intersection(&near))) {
        Ok(inter) => inter
            .0
            .iter()
            .map(|poly| poly.exterior().coords().map(|c| [c.x, c.y]).collect())
            .collect(),
        Err(_) => vec![ring.to_vec()],
    }
}

/// Parks are large and only a soft context wash, so a coarser simplification than
/// the buildings' 1 m is invisible and trims the citywide vertex count hard.
const SIMPLIFY_EPS_M: f64 = 2.5;
/// Drop park parts below this area (m²) — the dataset's slivers (street triangles,
/// medians, strips) read as green confetti otherwise. Keeps real pocket parks.
const MIN_AREA_M2: f64 = 400.0;

/// Single-letter NYC Parks borough code (`M`/`B`/`Q`/`X`/`R`) for a borough name or
/// code; `None` = keep every borough (citywide).
fn borough_code(borough: &str) -> Option<char> {
    match borough
        .to_ascii_lowercase()
        .replace([' ', '_', '-'], "")
        .as_str()
    {
        "manhattan" | "m" => Some('M'),
        "brooklyn" | "b" => Some('B'),
        "queens" | "q" => Some('Q'),
        "bronx" | "x" => Some('X'),
        "statenisland" | "si" | "r" => Some('R'),
        _ => None,
    }
}

pub fn bake(
    geojson_path: &str,
    out_path: &str,
    borough: Option<&str>,
    boundary_geojson: Option<&str>,
) -> Result<usize> {
    let bytes = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(&bytes).context("parsing parks GeoJSON")?;
    let proj = EnuProjection::default();
    let want_boro = borough.and_then(borough_code);
    // Optional shoreline clip mask (the borough-boundary land) so parks don't spill
    // into open water; lon/lat space, applied before projection.
    let land = boundary_geojson.map(load_land).transpose()?;

    let mut polygons: Vec<Vec<[f32; 2]>> = Vec::new();
    let (mut verts_in, mut verts_out) = (0usize, 0usize);
    let (mut dropped_small, mut dropped_boro) = (0usize, 0usize);
    let mut dropped_water = 0usize;
    for f in fc.features {
        if let Some(code) = want_boro {
            let b = f
                .properties
                .as_ref()
                .and_then(|p| p.get("borough"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.chars().next());
            if b != Some(code) {
                dropped_boro += 1;
                continue;
            }
        }
        let Some(geom) = f.geometry else { continue };
        // Each part's exterior ring (Polygon → ring; MultiPolygon → every part).
        let raw_rings: Vec<Vec<[f64; 2]>> = match geom.value {
            geojson::Value::Polygon(rings) => rings
                .into_iter()
                .take(1)
                .map(|r| r.iter().map(|p| [p[0], p[1]]).collect())
                .collect(),
            geojson::Value::MultiPolygon(polys) => polys
                .into_iter()
                .filter_map(|poly| poly.into_iter().next())
                .map(|r| r.iter().map(|p| [p[0], p[1]]).collect())
                .collect(),
            _ => continue,
        };
        for ring in raw_rings {
            if ring.len() < 4 {
                continue;
            }
            // Clip the park to the shoreline land mask (lon/lat) so it can't overshoot
            // the bank into open water; a park fully on land passes through unchanged,
            // one straddling the shore is trimmed, one entirely in water disappears.
            let parts: Vec<Vec<[f64; 2]>> = match &land {
                Some(l) => {
                    let clipped = clip_ring_to_land(&ring, l);
                    if clipped.is_empty() {
                        dropped_water += 1;
                    }
                    clipped
                }
                None => vec![ring],
            };
            for cring in parts {
                if cring.len() < 4 {
                    continue;
                }
                let enu64: Vec<[f64; 2]> = cring
                    .iter()
                    .map(|p| {
                        let e = proj.to_enu(p[1], p[0]);
                        [e.x, e.y]
                    })
                    .collect();
                if ring_area_m2(&enu64) < MIN_AREA_M2 {
                    dropped_small += 1;
                    continue;
                }
                verts_in += enu64.len();
                let simplified = rdp(&enu64, SIMPLIFY_EPS_M);
                if simplified.len() < 4 {
                    dropped_small += 1;
                    continue;
                }
                verts_out += simplified.len();
                polygons.push(simplified.iter().map(|p| [p[0] as f32, p[1] as f32]).collect());
            }
        }
    }
    anyhow::ensure!(!polygons.is_empty(), "no parks parsed");

    let layer = BuildingFootprints {
        origin: GeoOrigin::MANHATTAN,
        polygons,
        provenance: Provenance {
            source: "NYC Parks Properties (NYC Parks & Recreation)".into(),
            url: "https://data.cityofnewyork.us/Recreation/Parks-Properties/enfh-gkve".into(),
            license: "NYC Open Data — public domain".into(),
            as_of: "2024".into(),
            notes: format!(
                "Park property polygons, RDP-simplified @ {SIMPLIFY_EPS_M} m, ≥ {MIN_AREA_M2} m²; \
                 flat green context layer."
            ),
        },
    };
    let n = layer.polygons.len();
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "parks: {n} polygons ({dropped_boro} other-borough, {dropped_small} sub-min-area, \
         {dropped_water} fully-in-water dropped); vertices {verts_in} -> {verts_out} -> {out_path}"
    );
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::{borough_code, clip_ring_to_land, poly_bbox};
    use geo::{Coord, LineString, Polygon};

    /// A square land polygon `[lo,hi]²` as a one-entry mask (bbox + polygon).
    fn square_land(lo: f64, hi: f64) -> Vec<([f64; 4], Polygon<f64>)> {
        let p = Polygon::new(
            LineString::from(vec![
                Coord { x: lo, y: lo },
                Coord { x: hi, y: lo },
                Coord { x: hi, y: hi },
                Coord { x: lo, y: hi },
                Coord { x: lo, y: lo },
            ]),
            vec![],
        );
        vec![(poly_bbox(&p), p)]
    }

    #[test]
    fn clip_trims_a_park_straddling_the_shore() {
        // Land = the square [0,2]²; a park spanning [1,3]² straddles it. The clip keeps
        // only the [1,2]² overlap (area 1), so no vertex exceeds 2.
        let land = square_land(0.0, 2.0);
        let park = [[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0], [1.0, 1.0]];
        let parts = clip_ring_to_land(&park, &land);
        assert_eq!(parts.len(), 1, "one overlap region");
        for v in &parts[0] {
            assert!(v[0] <= 2.0 + 1e-9 && v[1] <= 2.0 + 1e-9, "clipped within land");
        }
    }

    #[test]
    fn clip_drops_a_park_entirely_in_water() {
        let land = square_land(0.0, 1.0);
        // A park far offshore → no bbox overlap → dropped.
        let park = [[5.0, 5.0], [6.0, 5.0], [6.0, 6.0], [5.0, 6.0], [5.0, 5.0]];
        assert!(clip_ring_to_land(&park, &land).is_empty());
    }

    #[test]
    fn borough_code_maps_names_codes_and_unknown() {
        // NYC Parks single-letter codes (note Bronx = X, Staten Island = R).
        assert_eq!(borough_code("Manhattan"), Some('M'));
        assert_eq!(borough_code("m"), Some('M'));
        assert_eq!(borough_code("brooklyn"), Some('B'));
        assert_eq!(borough_code("Queens"), Some('Q'));
        assert_eq!(borough_code("bronx"), Some('X'));
        assert_eq!(borough_code("Staten Island"), Some('R'));
        assert_eq!(borough_code("si"), Some('R'));
        // "all"/unknown → no filter.
        assert_eq!(borough_code("all"), None);
        assert_eq!(borough_code(""), None);
    }
}
