//! Build a routable pedestrian graph from an Overpass API JSON dump of the
//! Manhattan walk network (an all-Rust alternative to OSMnx).
//!
//! Pipeline: parse nodes+ways -> identify graph nodes (way endpoints + nodes
//! shared by >1 way) -> split ways into edges between graph nodes, accumulating
//! intermediate vertices into the edge polyline -> keep the largest connected
//! component (so A* always succeeds within it) -> project to ENU.

use std::collections::HashMap;

use anyhow::Context;
use serde::Deserialize;
use sim_core::assets::{EdgeData, GraphAsset, NodePoint, Provenance};
use sim_core::math::Vec2;
use sim_core::projection::{EnuProjection, GeoOrigin};

#[derive(Deserialize)]
struct OverpassResponse {
    elements: Vec<RawElement>,
}

#[derive(Deserialize)]
struct RawElement {
    #[serde(rename = "type")]
    kind: String,
    id: i64,
    lat: Option<f64>,
    lon: Option<f64>,
    #[serde(default)]
    nodes: Vec<i64>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

/// Highway types kept as the **street-centerline** network. OSM separately maps
/// each sidewalk/crossing as a `footway`, so the raw walk dump has ~3 parallel
/// lines per street; keeping only carriageway centerlines + pedestrian plazas
/// collapses that to one line per street (~75% fewer segments) — far less visual
/// clutter and a much smaller render mesh, while A* still routes (roads meet at
/// shared intersection nodes). Pedestrian fidelity (individual sidewalks, mid-
/// block crossings) is intentionally traded away — the v1 design is centerline.
fn is_kept_highway(hw: &str) -> bool {
    matches!(
        hw,
        "residential"
            | "primary"
            | "secondary"
            | "tertiary"
            | "unclassified"
            | "living_street"
            | "pedestrian"
            | "road"
            | "trunk"
            | "primary_link"
            | "secondary_link"
            | "tertiary_link"
            | "trunk_link"
    )
}

/// Highway types kept for the **drive** network (vehicle/taxi routing): the walk
/// keep-set minus `pedestrian` (plazas like Broadway at Union Square are
/// `highway=pedestrian` — legal to walk, not to drive). `living_street` stays (a
/// shared zone vehicles may use at walking pace). `service`/`motorway` aren't in the
/// walk-network OSM dump, so the set is otherwise identical.
fn is_drivable_highway(hw: &str) -> bool {
    is_kept_highway(hw) && hw != "pedestrian"
}

/// Whether motor vehicles are legally allowed on a way, from its access tags —
/// catches pedestrian zones tagged on a drivable street type (e.g. `motor_vehicle=no`).
fn drive_allowed(tags: &HashMap<String, String>) -> bool {
    let restricted = |v: Option<&String>| matches!(v.map(String::as_str), Some("no") | Some("private"));
    if restricted(tags.get("motor_vehicle")) || restricted(tags.get("vehicle")) {
        return false;
    }
    // `access=no/private` blocks unless `motor_vehicle` explicitly re-permits it.
    if restricted(tags.get("access")) {
        let mv = tags.get("motor_vehicle").map(String::as_str);
        if !matches!(mv, Some("yes") | Some("permissive") | Some("destination") | Some("designated")) {
            return false;
        }
    }
    true
}

/// Intern an OSM node id into a compact graph-node index, recording its ENU
/// position the first time it is seen.
fn intern(
    osm: i64,
    coords: &HashMap<i64, Vec2>,
    id_to_index: &mut HashMap<i64, u32>,
    points: &mut Vec<NodePoint>,
) -> Option<u32> {
    if let Some(&i) = id_to_index.get(&osm) {
        return Some(i);
    }
    let p = coords.get(&osm)?;
    let i = points.len() as u32;
    points.push(NodePoint { x: p.x, y: p.y });
    id_to_index.insert(osm, i);
    Some(i)
}

/// Union-Find for largest-connected-component extraction.
struct UnionFind {
    parent: Vec<u32>,
    size: Vec<u32>,
}
impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n as u32).collect(),
            size: vec![1; n],
        }
    }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            self.parent[x as usize] = self.parent[self.parent[x as usize] as usize];
            x = self.parent[x as usize];
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let (big, small) = if self.size[ra as usize] >= self.size[rb as usize] {
            (ra, rb)
        } else {
            (rb, ra)
        };
        self.parent[small as usize] = big;
        self.size[big as usize] += self.size[small as usize];
    }
}

pub fn bake(
    json_path: &str,
    out_path: &str,
    boundary_geojson: Option<&str>,
    drive: bool,
) -> anyhow::Result<(usize, usize)> {
    let data = std::fs::read(json_path).with_context(|| format!("reading {json_path}"))?;
    let resp: OverpassResponse =
        serde_json::from_slice(&data).context("parsing Overpass JSON")?;

    let proj = EnuProjection::default();

    // Optional Manhattan clip: an Overpass bbox query pulls in Bronx streets that,
    // joined to the island by bridges, survive the largest-component step. Drop any
    // edge with an endpoint outside the borough so the kept network — and every
    // graph-bound agent that walks it — stays on Manhattan.
    let boundary = boundary_geojson
        .map(crate::boundary::ManhattanBoundary::load)
        .transpose()?;

    // 1. Node coords (projected) + ways, filtered to the street-centerline
    //    keep-set (drops separately-mapped sidewalks/footways/steps/etc.).
    let mut coords: HashMap<i64, Vec2> = HashMap::new();
    let mut ways: Vec<(i64, Vec<i64>)> = Vec::new();
    let (mut total_ways, mut kept_ways) = (0usize, 0usize);
    for el in &resp.elements {
        match el.kind.as_str() {
            "node" => {
                if let (Some(la), Some(lo)) = (el.lat, el.lon) {
                    coords.insert(el.id, proj.to_enu(la, lo));
                }
            }
            "way" if el.nodes.len() >= 2 => {
                total_ways += 1;
                // Keep a way if it carries a kept highway tag. For the drive network,
                // exclude pedestrian plazas + access-restricted ways. Untagged ways
                // (geometry-only dumps) are kept only for the walk network.
                let keep = match el.tags.get("highway") {
                    Some(hw) if drive => is_drivable_highway(hw) && drive_allowed(&el.tags),
                    Some(hw) => is_kept_highway(hw),
                    None => !drive && el.tags.is_empty(),
                };
                if keep {
                    kept_ways += 1;
                    ways.push((el.id, el.nodes.clone()));
                }
            }
            _ => {}
        }
    }
    eprintln!(
        "OSM ways: {kept_ways}/{total_ways} kept ({})",
        if drive { "drivable centerlines" } else { "walk centerlines" }
    );

    // 2. Usage counts -> which nodes are graph (split) nodes.
    let mut usage: HashMap<i64, u32> = HashMap::new();
    for (_, ns) in &ways {
        for &n in ns {
            *usage.entry(n).or_default() += 1;
        }
    }
    let is_split = |ns: &[i64], pos: usize| -> bool {
        pos == 0 || pos == ns.len() - 1 || usage.get(&ns[pos]).copied().unwrap_or(0) >= 2
    };

    // 3. Split ways into edges between graph nodes.
    let mut id_to_index: HashMap<i64, u32> = HashMap::new();
    let mut points: Vec<NodePoint> = Vec::new();
    let mut edges: Vec<EdgeData> = Vec::new();

    for (wid, ns) in &ways {
        // Positions in this way that have known coords.
        let mut seg_start: Option<usize> = None;
        for pos in 0..ns.len() {
            if !coords.contains_key(&ns[pos]) {
                continue;
            }
            match seg_start {
                None => seg_start = Some(pos),
                Some(start) => {
                    if is_split(ns, pos) {
                        let mut poly: Vec<[f64; 2]> = Vec::new();
                        let mut len = 0.0;
                        let mut prev: Option<Vec2> = None;
                        for k in start..=pos {
                            if let Some(p) = coords.get(&ns[k]) {
                                poly.push([p.x, p.y]);
                                if let Some(pp) = prev {
                                    len += pp.distance(*p);
                                }
                                prev = Some(*p);
                            }
                        }
                        // Manhattan clip: keep the edge only if both endpoints are
                        // inside the borough (drops Bronx streets + the bridge spans
                        // that connect them, which otherwise ride the largest
                        // component). No clip → keep everything (legacy behavior).
                        let in_bounds = match &boundary {
                            Some(b) => {
                                poly.first().is_some_and(|&p| b.contains(p))
                                    && poly.last().is_some_and(|&p| b.contains(p))
                            }
                            None => true,
                        };
                        let a = intern(ns[start], &coords, &mut id_to_index, &mut points);
                        let b = intern(ns[pos], &coords, &mut id_to_index, &mut points);
                        if let (Some(a), Some(b)) = (a, b) {
                            if a != b && len > 0.0 && in_bounds {
                                edges.push(EdgeData {
                                    from: a,
                                    to: b,
                                    length_m: len,
                                    polyline: poly,
                                    segment_id: Some(*wid),
                                });
                            }
                        }
                        seg_start = Some(pos);
                    }
                }
            }
        }
    }

    anyhow::ensure!(!points.is_empty(), "no graph nodes parsed from Overpass dump");

    // 4. Keep the largest connected component (so A* always succeeds within it).
    let (new_nodes, new_edges) = largest_component(points, edges);
    let (n, m) = (new_nodes.len(), new_edges.len());
    let asset = GraphAsset {
        origin: GeoOrigin::MANHATTAN,
        nodes: new_nodes,
        edges: new_edges,
        provenance: Provenance {
            source: format!(
                "OpenStreetMap via Overpass API ({} network)",
                if drive { "drive" } else { "walk" }
            ),
            url: "https://overpass-api.de/".into(),
            license: "ODbL 1.0".into(),
            as_of: "2026-06-14".into(),
            notes: format!(
                "Manhattan {}-usable highways; largest connected component; \
                 street-centerline (not sidewalk-accurate){}.",
                if drive { "motor-vehicle" } else { "pedestrian" },
                if boundary.is_some() {
                    "; clipped to the Manhattan borough boundary"
                } else {
                    ""
                }
            ),
        },
    };
    std::fs::write(out_path, asset.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "OSM {} graph: {n} nodes, {m} edges (largest component) -> {out_path}",
        if drive { "drive" } else { "walk" }
    );
    Ok((n, m))
}

/// Keep only the largest connected component and remap its nodes to compact indices.
/// Shared by the Overpass and CSCL bakes so A* always succeeds within the kept graph.
fn largest_component(
    points: Vec<NodePoint>,
    edges: Vec<EdgeData>,
) -> (Vec<NodePoint>, Vec<EdgeData>) {
    if points.is_empty() {
        return (points, edges);
    }
    let mut uf = UnionFind::new(points.len());
    for e in &edges {
        uf.union(e.from, e.to);
    }
    let mut comp_size: HashMap<u32, u32> = HashMap::new();
    for i in 0..points.len() as u32 {
        *comp_size.entry(uf.find(i)).or_default() += 1;
    }
    let Some(largest_root) = comp_size.iter().max_by_key(|(_, &s)| s).map(|(&r, _)| r) else {
        return (Vec::new(), Vec::new());
    };
    let mut old_to_new: HashMap<u32, u32> = HashMap::new();
    let mut new_nodes: Vec<NodePoint> = Vec::new();
    for i in 0..points.len() as u32 {
        if uf.find(i) == largest_root {
            old_to_new.insert(i, new_nodes.len() as u32);
            new_nodes.push(points[i as usize]);
        }
    }
    let mut new_edges: Vec<EdgeData> = Vec::new();
    for e in edges {
        if let (Some(&from), Some(&to)) = (old_to_new.get(&e.from), old_to_new.get(&e.to)) {
            new_edges.push(EdgeData { from, to, ..e });
        }
    }
    (new_nodes, new_edges)
}

// ---- CSCL (NYC Street Centerline) bake --------------------------------------
//
// The OSM/Overpass path is the most accurate for Manhattan, but a citywide Overpass
// query overwhelms the public instances. NYC's own LION/CSCL centerline (Socrata
// `inkn-q76z`) is the authoritative five-borough street network — already split at
// intersections and clean of out-of-city spillover — so the citywide graph is built
// from it instead. Each segment is one edge; shared intersection endpoints snap to a
// common node, then we keep the largest connected component (bridges + tunnels span
// the boroughs, so all five land in one component — incl. Staten Island via the
// Verrazzano, which the walk network would strand).

#[derive(Deserialize)]
struct CsclFc {
    features: Vec<CsclFeature>,
}
#[derive(Deserialize)]
struct CsclFeature {
    geometry: Option<CsclGeom>,
    properties: CsclProps,
}
#[derive(Deserialize)]
struct CsclGeom {
    coordinates: Vec<Vec<[f64; 2]>>, // MultiLineString: parts of [lon, lat] vertices
}
#[derive(Deserialize)]
struct CsclProps {
    rw_type: Option<String>,
    /// Traffic direction: `TW`/`FT`/`TF` (vehicular) or `NV` (**non-vehicular** —
    /// CSCL's own "not for cars" flag: pedestrian malls, promenades, bike paths).
    trafdir: Option<String>,
    /// Non-pedestrian indicator — a DOE *school-walk-route* exclusion, not a vehicle
    /// field. `V` ≈ vehicular-only (highways); `D` ≈ park / off-grid. We use `D`
    /// **only inside a park polygon** as the residual signal for car-free park drives.
    nonped: Option<String>,
    /// Posted speed limit (mph, as a string); absent on most ramps/alleys.
    posted_speed: Option<String>,
    /// Full street name — used to whitelist the Central Park **transverses** (open
    /// crosstown car routes) back in, since they share the loop drives' `nonped=D`.
    full_street_name: Option<String>,
}

/// CSCL `rw_type` codes kept as the drivable street network: 1 Street, 2 Highway,
/// 3 Bridge, 4 Tunnel, 9 Ramp, 10 Alley. Ramps (9, per the NYC LION coding) are the
/// grade-separated connectors between highways and surface streets — without them the
/// FDR / Henry Hudson would be an isolated component and get dropped. Excluded: 6
/// Path/Trail, 12 non-physical/paper, 14 Ferry, 5 Boardwalk, 7 Step, 8 Driveway, 13 U-turn.
const CSCL_KEEP_RW: &[&str] = &["1", "2", "3", "4", "9", "10"];

/// Intern a point into a node index, snapping to a 1 m grid so segments meeting at a
/// shared intersection collapse onto one node (CSCL endpoints coincide to <0.1 m).
fn intern_snapped(
    p: Vec2,
    keys: &mut HashMap<(i64, i64), u32>,
    points: &mut Vec<NodePoint>,
) -> u32 {
    let key = (p.x.round() as i64, p.y.round() as i64);
    if let Some(&i) = keys.get(&key) {
        return i;
    }
    let i = points.len() as u32;
    points.push(NodePoint { x: p.x, y: p.y });
    keys.insert(key, i);
    i
}

/// Weekday (as it appears in Open Streets `apprdayswe`) the trip model represents.
/// Both the Manhattan (2024-06-25) and citywide (2024-06-25) baked days are Tuesdays,
/// so Open-Streets closures active on a Tuesday are dropped from the drive graph while
/// weekend-only closures stay drivable. See `docs/TRIP_MODEL.md` (one real day).
const SIM_WEEKDAY: &str = "Tue";

/// Build the citywide street graph from the NYC CSCL centerline GeoJSON.
pub fn bake_cscl(
    geojson_path: &str,
    out_path: &str,
    boundary_geojson: Option<&str>,
    parks_geojson: Option<&str>,
    open_streets_geojson: Option<&str>,
) -> anyhow::Result<(usize, usize)> {
    let data = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let fc: CsclFc = serde_json::from_slice(&data).context("parsing CSCL GeoJSON")?;
    let proj = EnuProjection::default();
    // Optional borough clip (e.g. a Manhattan-only drive graph for taxi routing +
    // coverage): drop any segment with an endpoint off the island.
    let boundary = boundary_geojson
        .map(crate::boundary::ManhattanBoundary::load)
        .transpose()?;
    // Optional park mask: CSCL codes car-free park interiors (Central Park's
    // drives/paths) as `rw_type` 1 "Street", so the router would shortcut cars
    // through them. Drop surface segments whose midpoint falls inside a park.
    let parks = parks_geojson.map(crate::boundary::ParkMask::load).transpose()?;
    // Optional Open Streets mask: NYC DOT car-free streets active on the simulated
    // weekday (CSCL still codes them vehicular). Layer 4 of the drivability blacklist.
    let open = open_streets_geojson
        .map(|p| crate::boundary::OpenStreetMask::load(p, SIM_WEEKDAY))
        .transpose()?;

    let mut keys: HashMap<(i64, i64), u32> = HashMap::new();
    let mut points: Vec<NodePoint> = Vec::new();
    let mut edges: Vec<EdgeData> = Vec::new();
    let (mut total, mut kept, mut dropped_nv, mut dropped_park, mut dropped_open) =
        (0usize, 0usize, 0usize, 0usize, 0usize);

    for f in &fc.features {
        total += 1;
        let Some(geom) = &f.geometry else { continue };
        let p = &f.properties;
        let rw = p.rw_type.as_deref().unwrap_or("");
        // Layer 1 — physical roadway type. Keep only the drivable classes (drops
        // paths, steps, boardwalks, driveways, ferries, paper/u-turn segments).
        if !CSCL_KEEP_RW.contains(&rw) {
            continue;
        }
        let rw_class: i64 = rw.parse().unwrap_or(0);
        // Layer 2 — authoritative non-vehicular flag. `trafdir == "NV"` is CSCL's own
        // "not for cars" designation, and catches the cases Layer 1 can't: pedestrian
        // malls / promenades / bike paths that are mis-typed as `rw_type` 1 "Street".
        // Exempt highways/ramps (rw 2/9): an NV flag there is a coding error, not a
        // pedestrianization, and dropping it could sever the FDR / a parkway ramp.
        let trafdir = p.trafdir.as_deref().unwrap_or("");
        if trafdir == "NV" && !matches!(rw_class, 2 | 9) {
            dropped_nv += 1;
            continue;
        }
        // Posted speed limit (mph) — packed into `segment_id` alongside the class so
        // the time router can use real limits; 0 = unknown → per-class fallback.
        let posted_mph: i64 = p
            .posted_speed
            .as_deref()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0)
            .clamp(0, 60);
        let nonped = p.nonped.as_deref().unwrap_or("");
        // Central Park's crosstown transverses (65/79/86/97 St) are open to cars but
        // share the closed loop drives' `nonped=D` coding (they're sunken cuts the
        // park bridges over, so peds can't walk them). Their *name* is the only clean
        // separator: "TRANSVERSE" appears on exactly those 42 segments citywide.
        let is_transverse = p
            .full_street_name
            .as_deref()
            .is_some_and(|n| n.to_ascii_uppercase().contains("TRANSVERSE"));
        kept += 1;
        for part in &geom.coordinates {
            if part.len() < 2 {
                continue;
            }
            let verts: Vec<Vec2> = part.iter().map(|c| proj.to_enu(c[1], c[0])).collect(); // [lon, lat]
            // Borough clip (when given): both endpoints must be on-island.
            if let Some(b) = &boundary {
                let on = |p: Vec2| b.contains([p.x, p.y]);
                if !(on(verts[0]) && on(*verts.last().unwrap())) {
                    continue;
                }
            }
            // Layer 3 — policy-closed park interiors. CSCL leaves Central Park's loop
            // drives coded *vehicular* (trafdir TW/FT/TF) though they've been closed to
            // cars since 2018, so `trafdir` alone misses them. Inside a park, the loop
            // drives (and footpaths/malls) carry `nonped == "D"` (a DOE school-walk-route
            // exclusion), so we drop `nonped=D` drivable surface classes (1/3/4/10) whose
            // midpoint sits in a park — **except** the named transverses, which are open
            // crosstown car routes that share the same `nonped=D` coding. Highways/ramps
            // (2/9) are already exempt above; only surface classes reach here.
            if let Some(pk) = &parks {
                if nonped == "D" && !is_transverse && matches!(rw_class, 1 | 3 | 4 | 10) {
                    let mid = verts[verts.len() / 2];
                    if pk.contains([mid.x, mid.y]) {
                        dropped_park += 1;
                        continue;
                    }
                }
            }
            // Layer 4 — NYC DOT Open Streets (car-free on the simulated weekday). CSCL
            // codes these as ordinary streets; drop a segment colinear with and close
            // to a closed Open-Streets run. Surface classes only (Open Streets are never
            // highways/ramps).
            if let Some(os) = &open {
                if matches!(rw_class, 1 | 3 | 4 | 10) {
                    let mid = verts[verts.len() / 2];
                    let dir = verts.last().unwrap().sub(verts[0]).normalize();
                    if os.blocks(mid, dir) {
                        dropped_open += 1;
                        continue;
                    }
                }
            }
            let a = intern_snapped(verts[0], &mut keys, &mut points);
            let b = intern_snapped(*verts.last().unwrap(), &mut keys, &mut points);
            if a == b {
                continue; // a closed loop / sub-metre stub
            }
            let len: f64 = verts.windows(2).map(|w| w[0].distance(w[1])).sum();
            if len <= 0.0 {
                continue;
            }
            edges.push(EdgeData {
                from: a,
                to: b,
                length_m: len,
                polyline: verts.iter().map(|p| [p.x, p.y]).collect(),
                // Pack road class + posted speed: rw_type * 100 + posted_mph (0 =
                // unknown). Decoded by `sim_core::graph::unpack_class`.
                segment_id: Some(rw_class * 100 + posted_mph),
            });
        }
    }
    anyhow::ensure!(!points.is_empty(), "no street segments parsed from CSCL");
    eprintln!(
        "CSCL segments: {kept}/{total} kept (rw_type {CSCL_KEEP_RW:?}); dropped {dropped_nv} \
         non-vehicular (trafdir=NV) + {dropped_park} closed park drives (nonped=D in a park) + \
         {dropped_open} Open-Streets blocks (car-free on {SIM_WEEKDAY})"
    );

    let (nodes, edges) = largest_component(points, edges);
    let (n, m) = (nodes.len(), edges.len());
    // Sanity: the ENU node bbox should span the whole city (~40.49–40.92 N,
    // −74.26–−73.70 W) if all five boroughs landed in one connected component.
    {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for nd in &nodes {
            x0 = x0.min(nd.x);
            y0 = y0.min(nd.y);
            x1 = x1.max(nd.x);
            y1 = y1.max(nd.y);
        }
        let (lat0, lon0) = proj.to_wgs84(Vec2::new(x0, y0));
        let (lat1, lon1) = proj.to_wgs84(Vec2::new(x1, y1));
        eprintln!("CSCL graph bbox: lat {lat0:.3}..{lat1:.3}, lon {lon0:.3}..{lon1:.3}");
    }
    let asset = GraphAsset {
        origin: GeoOrigin::MANHATTAN,
        nodes,
        edges,
        provenance: Provenance {
            source: "NYC Street Centerline (CSCL), NYC DCP / DoITT".into(),
            url: "https://data.cityofnewyork.us/City-Government/NYC-Street-Centerline-CSCL-/inkn-q76z"
                .into(),
            license: "NYC Open Data — public domain".into(),
            as_of: "2026".into(),
            notes: "Five-borough street centerline, drivable network: rw_type \
                    street/highway/bridge/tunnel/ramp/alley, minus trafdir=NV (non-vehicular), \
                    nonped=D park-interior drives, and DOT Open Streets car-free on the simulated \
                    weekday; posted speed packed in segment_id; intersections snapped at 1 m; \
                    largest connected component."
                .into(),
        },
    };
    std::fs::write(out_path, asset.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("CSCL graph: {n} nodes, {m} edges (largest component) -> {out_path}");
    Ok((n, m))
}
