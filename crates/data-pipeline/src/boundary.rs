//! A Manhattan-borough boundary, used to clip layers (the street graph, the ACE
//! corridors, the moving-bus shapes) to the island so nothing renders or routes
//! out into the Bronx / Queens.
//!
//! There is no single borough polygon in the snapshot set, but the Pedia-Cities
//! neighborhood layer tiles the borough, so the **union of its Manhattan
//! neighborhoods** is the boundary: a point is "in Manhattan" iff it falls inside
//! any Manhattan neighborhood ring. We test against the rings directly (no
//! geometric dissolve needed) with a per-ring bbox prefilter.
//!
//! Strict point-in-polygon, deliberately **no buffer**: the Harlem River between
//! Manhattan and the Bronx is only ~30 m wide at the Broadway Bridge, so any
//! outward buffer would leak Bronx streets back across it. Marble Hill (north of
//! the river but civically Manhattan) is kept because the neighborhood layer tags
//! it Manhattan — the boundary follows the borough, not the shoreline.

use anyhow::{Context, Result};
use sim_core::math::Vec2;
use sim_core::projection::EnuProjection;

/// Neighborhoods that are legally Manhattan / New York County but are **detached
/// islands**, not the main island — we drop them so clipped layers stay visually
/// contiguous (e.g. the M60-SBS corridor no longer trails onto Randall's Island
/// across the RFK Bridge). Names normalized (lowercased, alphanumerics only) so the
/// apostrophe in "Randall's Island" can't cause a miss.
const DETACHED_ISLANDS: &[&str] = &[
    "randallsisland",
    "wardsisland",
    "rooseveltisland",
    "governorsisland",
    "libertyisland",
    "ellisisland",
    "marblehill", // the Bronx-attached exclave; also off the main island
];

/// Lowercase + keep only ASCII alphanumerics (apostrophe/space/punct-insensitive).
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// One Manhattan neighborhood ring in ENU meters, with its bbox for prefiltering.
struct Ring {
    pts: Vec<[f64; 2]>,
    bbox: [f64; 4], // [min_x, min_y, max_x, max_y]
}

/// The Manhattan boundary as a set of neighborhood rings (logical union).
pub struct ManhattanBoundary {
    rings: Vec<Ring>,
}

impl ManhattanBoundary {
    /// Load from the Pedia-Cities neighborhoods GeoJSON, keeping only the rings
    /// whose `borough` is `Manhattan`, projected with the canonical ENU origin
    /// (matches every other bake, so the boundary aligns with the layers it clips).
    pub fn load(geojson_path: &str) -> Result<Self> {
        let bytes =
            std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
        let fc: geojson::FeatureCollection =
            serde_json::from_slice(&bytes).context("parsing boundary GeoJSON")?;
        let proj = EnuProjection::default();
        let mut rings: Vec<Ring> = Vec::new();
        for f in fc.features {
            let props = f.properties.as_ref();
            let borough = props
                .and_then(|p| p.get("borough"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if borough != "Manhattan" {
                continue;
            }
            // Drop detached-island neighborhoods so the boundary is the main island.
            let name = props
                .and_then(|p| p.get("neighborhood").or_else(|| p.get("name")))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if DETACHED_ISLANDS.contains(&normalize(name).as_str()) {
                continue;
            }
            let Some(geom) = f.geometry else { continue };
            // Exterior ring(s): Polygon → first ring; MultiPolygon → each part's.
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
            for raw in raw_rings {
                if raw.len() < 4 {
                    continue;
                }
                let pts: Vec<[f64; 2]> = raw
                    .iter()
                    .map(|p| {
                        let e = proj.to_enu(p[1], p[0]); // GeoJSON is [lon, lat]
                        [e.x, e.y]
                    })
                    .collect();
                let (mut minx, mut miny, mut maxx, mut maxy) =
                    (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for p in &pts {
                    minx = minx.min(p[0]);
                    miny = miny.min(p[1]);
                    maxx = maxx.max(p[0]);
                    maxy = maxy.max(p[1]);
                }
                rings.push(Ring {
                    pts,
                    bbox: [minx, miny, maxx, maxy],
                });
            }
        }
        anyhow::ensure!(
            !rings.is_empty(),
            "no Manhattan neighborhood rings in {geojson_path}"
        );
        Ok(ManhattanBoundary { rings })
    }

    /// Is ENU point `p` inside Manhattan (inside any neighborhood ring)?
    pub fn contains(&self, p: [f64; 2]) -> bool {
        let pt = Vec2::new(p[0], p[1]);
        for r in &self.rings {
            if p[0] >= r.bbox[0]
                && p[0] <= r.bbox[2]
                && p[1] >= r.bbox[1]
                && p[1] <= r.bbox[3]
                && point_in_ring(pt, &r.pts)
            {
                return true;
            }
        }
        false
    }

    /// Find the boundary-crossing point on segment `a`→`b`, where exactly one of
    /// `a`, `b` is inside Manhattan, by bisection on the inside/outside predicate.
    /// Returns a point just inside the boundary so the clipped run stays valid.
    fn crossing(&self, a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
        let a_in = self.contains(a);
        // Bisect the parameter t∈[0,1] keeping `lo` on the inside, `hi` outside.
        let (mut lo, mut hi) = if a_in { (0.0, 1.0) } else { (1.0, 0.0) };
        let lerp = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if self.contains(lerp(mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lerp(lo) // the last point confirmed inside
    }

    /// Clip a polyline to Manhattan, returning each contiguous in-Manhattan run
    /// (≥2 points), with interpolated points inserted where it crosses the
    /// boundary so each run begins/ends exactly on the border.
    pub fn clip_polyline(&self, poly: &[[f64; 2]]) -> Vec<Vec<[f64; 2]>> {
        if poly.is_empty() {
            return Vec::new();
        }
        let inside: Vec<bool> = poly.iter().map(|&p| self.contains(p)).collect();
        let mut runs: Vec<Vec<[f64; 2]>> = Vec::new();
        let mut cur: Vec<[f64; 2]> = Vec::new();
        for i in 0..poly.len() {
            if inside[i] {
                if i > 0 && !inside[i - 1] {
                    // Entering: start the run on the boundary.
                    cur.push(self.crossing(poly[i - 1], poly[i]));
                }
                cur.push(poly[i]);
            } else {
                if i > 0 && inside[i - 1] {
                    // Leaving: end the run on the boundary, then flush.
                    cur.push(self.crossing(poly[i - 1], poly[i]));
                }
                if cur.len() >= 2 {
                    runs.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
        }
        if cur.len() >= 2 {
            runs.push(cur);
        }
        runs
    }

    /// Clip a polyline and return only its **longest** in-Manhattan run (by point
    /// count) — the right choice for a single bus shape, which has one main
    /// Manhattan trunk plus, at most, stub re-entries we don't want to keep.
    pub fn longest_run(&self, poly: &[[f64; 2]]) -> Vec<[f64; 2]> {
        self.clip_polyline(poly)
            .into_iter()
            .max_by_key(|r| r.len())
            .unwrap_or_default()
    }
}

/// A mask of park polygons (NYC Parks Properties): "is this point inside any
/// park?" Used to keep the drive graph out of **car-free park interiors** —
/// Central Park's drives/paths read as `rw_type` 1 ("Street") in CSCL but the
/// park has been closed to cars since 2018, so routing must not shortcut through
/// it. Reuses the same ring + bbox-prefilter machinery as `ManhattanBoundary`.
pub struct ParkMask {
    rings: Vec<Ring>,
}

impl ParkMask {
    /// Load every park polygon's exterior ring(s) from the Parks GeoJSON,
    /// projected with the canonical ENU origin (aligns with the graph it filters).
    pub fn load(geojson_path: &str) -> Result<Self> {
        let bytes =
            std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
        let fc: geojson::FeatureCollection =
            serde_json::from_slice(&bytes).context("parsing parks GeoJSON")?;
        let proj = EnuProjection::default();
        let mut rings: Vec<Ring> = Vec::new();
        for f in fc.features {
            let Some(geom) = f.geometry else { continue };
            let raw_rings: Vec<Vec<[f64; 2]>> = match geom.value {
                geojson::Value::Polygon(rs) => {
                    rs.into_iter().take(1).map(|r| r.iter().map(|p| [p[0], p[1]]).collect()).collect()
                }
                geojson::Value::MultiPolygon(polys) => polys
                    .into_iter()
                    .filter_map(|poly| poly.into_iter().next())
                    .map(|r| r.iter().map(|p| [p[0], p[1]]).collect())
                    .collect(),
                _ => continue,
            };
            for raw in raw_rings {
                if raw.len() < 4 {
                    continue;
                }
                let pts: Vec<[f64; 2]> = raw
                    .iter()
                    .map(|p| {
                        let e = proj.to_enu(p[1], p[0]);
                        [e.x, e.y]
                    })
                    .collect();
                let (mut minx, mut miny, mut maxx, mut maxy) =
                    (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for p in &pts {
                    minx = minx.min(p[0]);
                    miny = miny.min(p[1]);
                    maxx = maxx.max(p[0]);
                    maxy = maxy.max(p[1]);
                }
                rings.push(Ring { pts, bbox: [minx, miny, maxx, maxy] });
            }
        }
        anyhow::ensure!(!rings.is_empty(), "no park rings in {geojson_path}");
        Ok(ParkMask { rings })
    }

    /// Is ENU point `p` inside any park?
    pub fn contains(&self, p: [f64; 2]) -> bool {
        let pt = Vec2::new(p[0], p[1]);
        for r in &self.rings {
            if p[0] >= r.bbox[0]
                && p[0] <= r.bbox[2]
                && p[1] >= r.bbox[1]
                && p[1] <= r.bbox[3]
                && point_in_ring(pt, &r.pts)
            {
                return true;
            }
        }
        false
    }
}

/// A mask of **car-free streets** from the NYC DOT Open Streets program (Socrata
/// `uiay-nctu`): polyline geometry of streets closed to through traffic. CSCL still
/// codes these as ordinary vehicular streets (its LION vintage predates the closures),
/// so without this the router sends cars down pedestrianized blocks (Broadway near
/// Union Square, the West Village/LES permanent Open Streets, etc.).
///
/// Two filters at load: (1) `reviewstat` starts with "Full Closure" (full + school
/// closures are car-free; "Limited Local Access" still admits local cars, so it's
/// kept), and (2) the closure is active on the **simulated weekday** — the trip model
/// is one real Tuesday, so weekend-only closures (5 Ave Sundays, Columbus Ave) stay
/// drivable while permanent and weekday closures drop.
///
/// Match is **colinear proximity**: a street segment is car-free if its midpoint is
/// within `OPEN_ST_THRESH_M` of an Open-Streets sub-segment running roughly parallel
/// (|cos θ| > `OPEN_ST_COS`). The parallel test avoids dropping a cross street merely
/// because it meets a closed street at a corner.
pub struct OpenStreetMask {
    /// Closed sub-segments: (a, b, unit direction, bbox [minx,miny,maxx,maxy]).
    segs: Vec<(Vec2, Vec2, Vec2, [f64; 4])>,
}

const OPEN_ST_THRESH_M: f64 = 10.0;
const OPEN_ST_COS: f64 = 0.87; // ~30°

impl OpenStreetMask {
    /// Load the Open Streets GeoJSON, keeping only full/school closures active on
    /// `weekday` (a three-letter abbreviation as it appears in `apprdayswe`, e.g.
    /// "Tue"), projected with the canonical ENU origin.
    pub fn load(geojson_path: &str, weekday: &str) -> Result<Self> {
        let bytes =
            std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
        let fc: geojson::FeatureCollection =
            serde_json::from_slice(&bytes).context("parsing Open Streets GeoJSON")?;
        let proj = EnuProjection::default();
        let mut segs: Vec<(Vec2, Vec2, Vec2, [f64; 4])> = Vec::new();
        for f in fc.features {
            let props = f.properties.as_ref();
            let reviewstat = props
                .and_then(|p| p.get("reviewstat"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Full + school closures are car-free; "Limited Local Access" keeps cars.
            if !reviewstat.starts_with("Full Closure") {
                continue;
            }
            let days = props
                .and_then(|p| p.get("apprdayswe"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !days.contains(weekday) {
                continue;
            }
            let Some(geom) = f.geometry else { continue };
            let lines: Vec<Vec<[f64; 2]>> = match geom.value {
                geojson::Value::LineString(line) => {
                    vec![line.iter().map(|p| [p[0], p[1]]).collect()]
                }
                geojson::Value::MultiLineString(lines) => lines
                    .into_iter()
                    .map(|l| l.iter().map(|p| [p[0], p[1]]).collect())
                    .collect(),
                _ => continue,
            };
            for line in lines {
                let pts: Vec<Vec2> = line.iter().map(|p| proj.to_enu(p[1], p[0])).collect();
                for w in pts.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    let d = b.sub(a);
                    let len = d.length();
                    if len < 1e-6 {
                        continue;
                    }
                    let dir = Vec2::new(d.x / len, d.y / len);
                    let bbox = [a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)];
                    segs.push((a, b, dir, bbox));
                }
            }
        }
        anyhow::ensure!(!segs.is_empty(), "no active car-free segments in {geojson_path}");
        Ok(OpenStreetMask { segs })
    }

    /// Is a street segment with midpoint `mid` and unit travel direction `dir`
    /// car-free (colinear with, and close to, an Open-Streets sub-segment)?
    pub fn blocks(&self, mid: Vec2, dir: Vec2) -> bool {
        for (a, b, sdir, bbox) in &self.segs {
            if mid.x < bbox[0] - OPEN_ST_THRESH_M
                || mid.x > bbox[2] + OPEN_ST_THRESH_M
                || mid.y < bbox[1] - OPEN_ST_THRESH_M
                || mid.y > bbox[3] + OPEN_ST_THRESH_M
            {
                continue;
            }
            if (dir.x * sdir.x + dir.y * sdir.y).abs() > OPEN_ST_COS
                && point_seg_dist(mid, *a, *b) < OPEN_ST_THRESH_M
            {
                return true;
            }
        }
        false
    }
}

/// Distance from point `p` to segment `a`→`b` in ENU meters.
fn point_seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let ab = b.sub(a);
    let l2 = ab.x * ab.x + ab.y * ab.y;
    if l2 <= 1e-12 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / l2).clamp(0.0, 1.0);
    let proj = Vec2::new(a.x + t * ab.x, a.y + t * ab.y);
    p.distance(proj)
}

/// Ray-casting point-in-polygon on an ENU ring (mirrors sim_core's private one).
fn point_in_ring(p: Vec2, ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (ring[i][0], ring[i][1]);
        let (xj, yj) = (ring[j][0], ring[j][1]);
        if ((yi > p.y) != (yj > p.y)) && (p.x < (xj - xi) * (p.y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}
