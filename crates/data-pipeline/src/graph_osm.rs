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

/// Max gap (m) a connectivity stitch will bridge. Sits deliberately between the real
/// interchange/ramp-stub gaps that fragment the CSCL drive graph (≤~75 m, measured) and
/// the open-water spans separating the outer islands (≥200 m, measured — Staten Island,
/// Rikers, Governors), so a severed bridge on/off-ramp reconnects to the surface grid
/// while no synthetic edge is ever drawn across open water. Env-overridable for tuning.
const STITCH_MAX_M: f64 = 75.0;

/// Synthetic class for a stitch connector edge: surface-street (rw 1), speed-unknown (0),
/// so the time router applies the street-class fallback speed over the short gap.
const CONNECTOR_SEGMENT_ID: i64 = 100;

/// Reconnect near-miss fragments before the keep-largest prune. CSCL centrelines are
/// planar (no elevation) and occasionally leave a metre-to-tens-of-metres gap where a
/// grade-separated **bridge on/off-ramp** meets the surface street at an interchange; that
/// gap fragmented the citywide graph into ~900 components, and keeping only the largest
/// then silently deleted the severed ramps (the "bridge ramps not used" bug). For every
/// component other than the largest, if one of its nodes lies within `stitch_max` of a node
/// in the growing main network, we add one straight connector edge across the closest such
/// pair. Iterated so ramp→deck→grid chains reconnect in stages. The distance ceiling keeps
/// the water-separated islands out — they rejoin only via a real, in-data bridge.
fn stitch_components(
    points: Vec<NodePoint>,
    mut edges: Vec<EdgeData>,
    stitch_max: f64,
) -> (Vec<NodePoint>, Vec<EdgeData>) {
    if points.is_empty() {
        return (points, edges);
    }
    let cell = |v: f64| (v / stitch_max).floor() as i64;
    let (mut total, mut max_len) = (0u32, 0.0f64);
    for _pass in 0..8 {
        // (Re)compute components over the current edge set (includes prior connectors).
        let mut uf = UnionFind::new(points.len());
        for e in &edges {
            uf.union(e.from, e.to);
        }
        let mut comp_size: HashMap<u32, u32> = HashMap::new();
        for i in 0..points.len() as u32 {
            *comp_size.entry(uf.find(i)).or_default() += 1;
        }
        let Some(largest_root) = comp_size.iter().max_by_key(|(_, &s)| s).map(|(&r, _)| r) else {
            break;
        };
        let roots: Vec<u32> = (0..points.len() as u32).map(|i| uf.find(i)).collect();
        // Grid over the largest component's nodes (cell = stitch_max ⇒ any node within reach
        // is in the 3×3 neighbourhood).
        let mut grid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for i in 0..points.len() {
            if roots[i] == largest_root {
                grid.entry((cell(points[i].x), cell(points[i].y))).or_default().push(i as u32);
            }
        }
        // Closest (fragment-node, main-node) pair per non-largest component.
        let mut best: HashMap<u32, (f64, u32, u32)> = HashMap::new();
        for i in 0..points.len() as u32 {
            let r = roots[i as usize];
            if r == largest_root {
                continue;
            }
            let q = &points[i as usize];
            let (cx, cy) = (cell(q.x), cell(q.y));
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if let Some(v) = grid.get(&(cx + dx, cy + dy)) {
                        for &j in v {
                            let p2 = &points[j as usize];
                            let d = ((p2.x - q.x).powi(2) + (p2.y - q.y).powi(2)).sqrt();
                            if d <= stitch_max && best.get(&r).map_or(true, |&(bd, ..)| d < bd) {
                                best.insert(r, (d, i, j));
                            }
                        }
                    }
                }
            }
        }
        if best.is_empty() {
            break;
        }
        for (_, (d, i, j)) in &best {
            edges.push(EdgeData {
                from: *i,
                to: *j,
                length_m: *d,
                polyline: vec![
                    [points[*i as usize].x, points[*i as usize].y],
                    [points[*j as usize].x, points[*j as usize].y],
                ],
                segment_id: Some(CONNECTOR_SEGMENT_ID),
            });
            total += 1;
            max_len = max_len.max(*d);
        }
    }
    eprintln!(
        "CSCL connectivity: stitched {total} fragment(s) into the main network (≤{stitch_max:.0} m gaps; longest connector {max_len:.0} m)"
    );
    (points, edges)
}

/// Ceiling (m) for the **bridge-aware** reconnection. A real, in-data bridge to an outer
/// island is sometimes split into two chains because CSCL omits the centreline of its long
/// centre span — most importantly the **Verrazzano-Narrows** (measured ≈587 m gap between its
/// Staten-Island-side and Brooklyn-side decks), which leaves all of Staten Island a separate
/// component. This ceiling sits above that span but below any distance at which two *unrelated*
/// bridges would be joined, and the reconnection only ever fires between two bridge-class
/// (`rw_type` 3) nodes — so a ferry-only island (Governors) with no in-data bridge is never
/// joined. Env-overridable via `BRIDGE_STITCH_MAX_M`.
const BRIDGE_STITCH_MAX_M: f64 = 700.0;

/// Reconnect an outer-island component to the mainland across a **real bridge** whose deck
/// chain is broken by a centre-span digitization gap wider than the ramp-stitch ceiling. Only
/// bridge-class (`rw_type` 3) nodes are eligible on *both* ends, and only within `max_gap`, so
/// this repairs the Verrazzano (→ Staten Island) and any similarly-split in-data vehicular
/// bridge, while never drawing a synthetic span to a ferry-only island (no bridge nodes) or
/// between two unrelated bridges (beyond `max_gap`). The connector is classed as a bridge so
/// the time router treats it as bridge pavement. Runs after `stitch_components`, before the
/// component prune. Iterated so a multi-gap deck reconnects in stages.
fn stitch_bridges(
    points: Vec<NodePoint>,
    mut edges: Vec<EdgeData>,
    max_gap: f64,
) -> (Vec<NodePoint>, Vec<EdgeData>) {
    if points.is_empty() {
        return (points, edges);
    }
    // Bridge nodes: endpoints of any rw_class==3 edge (segment_id = rw_class*100 + posted_mph).
    let mut is_bridge = vec![false; points.len()];
    for e in &edges {
        if e.segment_id.unwrap_or(0).div_euclid(100) == 3 {
            is_bridge[e.from as usize] = true;
            is_bridge[e.to as usize] = true;
        }
    }
    let cell = |v: f64| (v / max_gap).floor() as i64;
    let (mut total, mut max_len) = (0u32, 0.0f64);
    for _pass in 0..4 {
        let mut uf = UnionFind::new(points.len());
        for e in &edges {
            uf.union(e.from, e.to);
        }
        let mut comp_size: HashMap<u32, u32> = HashMap::new();
        for i in 0..points.len() as u32 {
            *comp_size.entry(uf.find(i)).or_default() += 1;
        }
        let Some(largest_root) = comp_size.iter().max_by_key(|(_, &s)| s).map(|(&r, _)| r) else {
            break;
        };
        let roots: Vec<u32> = (0..points.len() as u32).map(|i| uf.find(i)).collect();
        // Grid of the largest component's *bridge* nodes (cell = max_gap ⇒ 3×3 neighbourhood
        // covers every in-reach node).
        let mut grid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for i in 0..points.len() {
            if roots[i] == largest_root && is_bridge[i] {
                grid.entry((cell(points[i].x), cell(points[i].y))).or_default().push(i as u32);
            }
        }
        // Closest (fragment bridge-node, main bridge-node) pair per non-largest component.
        let mut best: HashMap<u32, (f64, u32, u32)> = HashMap::new();
        for i in 0..points.len() as u32 {
            let r = roots[i as usize];
            if r == largest_root || !is_bridge[i as usize] {
                continue;
            }
            let q = &points[i as usize];
            let (cx, cy) = (cell(q.x), cell(q.y));
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if let Some(v) = grid.get(&(cx + dx, cy + dy)) {
                        for &j in v {
                            let p2 = &points[j as usize];
                            let d = ((p2.x - q.x).powi(2) + (p2.y - q.y).powi(2)).sqrt();
                            if d <= max_gap && best.get(&r).map_or(true, |&(bd, ..)| d < bd) {
                                best.insert(r, (d, i, j));
                            }
                        }
                    }
                }
            }
        }
        if best.is_empty() {
            break;
        }
        for (_, (d, i, j)) in &best {
            edges.push(EdgeData {
                from: *i,
                to: *j,
                length_m: *d,
                polyline: vec![
                    [points[*i as usize].x, points[*i as usize].y],
                    [points[*j as usize].x, points[*j as usize].y],
                ],
                segment_id: Some(300), // rw_class 3 (Bridge), posted speed unknown → class fallback
            });
            total += 1;
            max_len = max_len.max(*d);
        }
    }
    eprintln!(
        "CSCL bridge repair: reconnected {total} island bridge(s) across ≤{max_gap:.0} m centre-span gap(s); longest connector {max_len:.0} m"
    );
    (points, edges)
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
    // Surface the remaining fragmentation instead of hiding it: keeping only the largest
    // component silently discards the rest, so report how many nodes are dropped and flag
    // any *large* dropped component (>500 nodes) with its centroid — after the connectivity
    // stitch these are the genuine water-separated islands the drive graph can't reach
    // (Staten Island, Rikers, Governors), a known + intended limitation, not a silent loss.
    {
        let dropped = points.len() as u32 - comp_size.get(&largest_root).copied().unwrap_or(0);
        eprintln!(
            "CSCL components: {} total; keeping the largest, dropping {dropped} nodes in {} smaller component(s)",
            comp_size.len(),
            comp_size.len().saturating_sub(1),
        );
        let mut by_root: HashMap<u32, (f64, f64, u32)> = HashMap::new();
        for i in 0..points.len() as u32 {
            let e = by_root.entry(uf.find(i)).or_insert((0.0, 0.0, 0));
            e.0 += points[i as usize].x;
            e.1 += points[i as usize].y;
            e.2 += 1;
        }
        let dproj = EnuProjection::default();
        let mut big: Vec<(u32, f64, f64)> = by_root
            .iter()
            .filter(|(r, (.., c))| **r != largest_root && *c > 500)
            .map(|(_, (sx, sy, c))| {
                let (lat, lon) = dproj.to_wgs84(Vec2::new(sx / *c as f64, sy / *c as f64));
                (*c, lat, lon)
            })
            .collect();
        big.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        for (c, lat, lon) in big {
            eprintln!("  note: unreachable island component ~{c} nodes @ lat {lat:.3}, lon {lon:.3} (no drivable in-data bridge within reach)");
        }
    }
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

/// Minimum node count for a connected component to survive the citywide prune. Separates the
/// genuine street-network landmasses of the five boroughs from digitization noise. Measured
/// on the CSCL bake, the real components are the four-borough mainland (~60k nodes), **Staten
/// Island** (~12k), and the small islands that carry their own street grid but reach the
/// mainland only by a bridge/ferry CSCL doesn't fully digitize — City Island (~285), Rikers
/// (~177), Governors (~94); the long tail is ~870 components under 50 nodes (severed ramp
/// stubs + driveway fragments, ~3k nodes / ≈4% of the graph) that carry no neighbourhood.
/// Keeping every component ≥ this many nodes retains all real landmasses — so Staten Island's
/// ~500k residents are in the graph — while dropping the stub noise. Env-overridable via
/// `MIN_COMPONENT_NODES`.
const MIN_COMPONENT_NODES: usize = 50;

/// Keep every connected component with ≥ `min_nodes` nodes (not only the single largest) and
/// remap survivors to compact indices. Unlike `largest_component`, this retains the
/// water-separated boroughs/islands that have their own street grid but no drivable in-data
/// bridge to the mainland — chiefly **Staten Island**, whose Verrazzano deck chain is severed
/// mid-span because CSCL omits the bridge's ~590 m centre suspension span. Routing stays
/// correct: A* succeeds within a component and returns `NoPath` across the harbour, matching
/// the physical drive network (ferry/transit crossings aren't in the drive graph). Reports
/// what is kept (with centroids) and how much stub noise is dropped.
fn keep_components_above(
    points: Vec<NodePoint>,
    edges: Vec<EdgeData>,
    min_nodes: usize,
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
    // Roots of components large enough to be a real landmass grid (not stub noise).
    let keep_roots: std::collections::HashSet<u32> = comp_size
        .iter()
        .filter(|(_, &s)| s as usize >= min_nodes)
        .map(|(&r, _)| r)
        .collect();

    // Surface exactly what survives and what is dropped instead of hiding the prune: list the
    // kept landmass components with their centroids, and report the dropped fragment/node
    // totals so a regression (e.g. a borough silently falling below threshold) is visible.
    {
        let dproj = EnuProjection::default();
        let mut centroid: HashMap<u32, (f64, f64, u32)> = HashMap::new();
        for i in 0..points.len() as u32 {
            let e = centroid.entry(uf.find(i)).or_insert((0.0, 0.0, 0));
            e.0 += points[i as usize].x;
            e.1 += points[i as usize].y;
            e.2 += 1;
        }
        let mut kept: Vec<(u32, f64, f64)> = keep_roots
            .iter()
            .map(|r| {
                let (sx, sy, c) = centroid[r];
                let (lat, lon) = dproj.to_wgs84(Vec2::new(sx / c as f64, sy / c as f64));
                (c, lat, lon)
            })
            .collect();
        kept.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        let kept_nodes: u32 = kept.iter().map(|(c, ..)| *c).sum();
        let dropped_comps = comp_size.len().saturating_sub(keep_roots.len());
        let dropped_nodes = points.len() as u32 - kept_nodes;
        eprintln!(
            "CSCL components: {} total; keeping {} landmass component(s) with ≥{min_nodes} nodes \
             ({kept_nodes} nodes), dropping {dropped_nodes} nodes in {dropped_comps} \
             sub-{min_nodes}-node fragment(s)",
            comp_size.len(),
            keep_roots.len(),
        );
        for (c, lat, lon) in &kept {
            eprintln!("  keep: component ~{c} nodes @ lat {lat:.3}, lon {lon:.3}");
        }
    }

    let mut old_to_new: HashMap<u32, u32> = HashMap::new();
    let mut new_nodes: Vec<NodePoint> = Vec::new();
    for i in 0..points.len() as u32 {
        if keep_roots.contains(&uf.find(i)) {
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
// from it instead. Each segment is one edge; shared intersection endpoints weld to a
// common node (`intern_snapped`). CSCL is planar (no elevation), so grade-separated
// bridge on/off-ramps often leave a small gap where they meet the surface street, which
// fragments the raw graph into ~900 components; `stitch_components` bridges those sub-75 m
// gaps *before* the component prune, so we no longer silently delete severed ramps. Then
// `stitch_bridges` repairs real vehicular bridges whose deck chain CSCL splits at a long
// centre span — chiefly the **Verrazzano** (≈587 m gap), which otherwise strands all of
// **Staten Island** as its own component — reconnecting the island across its actual bridge so
// SI→mainland trips route (SI is NYC's most car-dependent borough; ~62% of its commutes cross
// to other boroughs). Finally we keep every component ≥50 nodes (`keep_components_above`)
// rather than only the largest, so an island that genuinely has no drivable in-data bridge
// (Governors — ferry only) still stays in for local residential exposure: A* routes within it
// and returns no-path across the water, and its block groups snap to their own local streets.
// Only the ~870 sub-50-node stub fragments are dropped as noise.

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

/// Which network to extract from the CSCL centerline. The two are **not** complements —
/// most streets carry both — but each drops what the other's traveller may not use, and
/// CSCL happens to carry a clean flag for each direction:
///
/// | | keeps | drops via | example |
/// |---|---|---|---|
/// | [`Drive`](CsclNetwork::Drive) | roadway | `trafdir == NV` ("not for cars") | Brooklyn Bridge **deck** |
/// | [`Walk`](CsclNetwork::Walk) | footway | `nonped == V` ("not for pedestrians") | Brooklyn Bridge **promenade** |
///
/// The Brooklyn Bridge is the clean demonstration: its roadway is `(trafdir=FT, nonped=V)`
/// and its promenade is `(trafdir=NV, nonped=D)`, so each network keeps exactly one of them.
/// The Verrazzano, which has no pedestrian path at all, is `nonped=V` on 120 of 121 segments
/// and so simply does not exist on foot — which is correct, and is why the walk graph leaves
/// Staten Island as its own component instead of stitching a phantom footway across the Narrows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CsclNetwork {
    /// Drivable street network (taxi/commute routing, roving-camera coverage).
    Drive,
    /// Pedestrian network — **what `R_i` should be flooded over.** A walkshed on the drive
    /// graph silently excludes 7,171 segments a person may legally walk (park paths, bridge
    /// promenades, pedestrian malls, boardwalks, steps) plus every park interior and Open
    /// Street, which are *precisely* pedestrian space.
    Walk,
}

/// CSCL `rw_type` codes kept as the drivable street network: 1 Street, 2 Highway,
/// 3 Bridge, 4 Tunnel, 9 Ramp, 10 Alley. Ramps (9, per the NYC LION coding) are the
/// grade-separated connectors between highways and surface streets — without them the
/// FDR / Henry Hudson would be an isolated component and get dropped. Excluded: 6
/// Path/Trail, 12 non-physical/paper, 14 Ferry, 5 Boardwalk, 7 Step, 8 Driveway, 13 U-turn.
const CSCL_KEEP_RW: &[&str] = &["1", "2", "3", "4", "9", "10"];

/// CSCL `rw_type` codes kept as the **pedestrian** network: 1 Street, 3 Bridge, 4 Tunnel,
/// 5 Boardwalk, 6 Path/Trail, 7 Step, 10 Alley.
///
/// Mirror-image of [`CSCL_KEEP_RW`]. Adds the classes a person walks and a car cannot
/// (Path/Trail — 5,036 segments, chiefly park paths; Boardwalk — Coney Island, the
/// Rockaways; Step). Drops 2 Highway and 9 Ramp: you cannot walk the FDR, and the 114
/// highway segments that lack a `nonped=V` flag would otherwise slip through on a coding gap.
/// Still excluded: 8 Driveway, 12 paper, 13 U-turn, 14 Ferry.
const CSCL_KEEP_RW_WALK: &[&str] = &["1", "3", "4", "5", "6", "7", "10"];

/// Default endpoint-merge tolerance (m) for welding CSCL segment endpoints into shared
/// nodes. CSCL endpoints that meet at an intersection are coincident to <0.1 m, but a
/// plain 1 m *grid bucket* (round-to-int key) fails to merge two coincident points that
/// straddle a cell boundary — e.g. x=100.49→100 vs x=100.51→101, 2 cm apart yet different
/// keys. That artifact (plus the odd sub-few-metre CSCL gap at grade-separated bridge/ramp
/// junctions) shattered the citywide graph into ~900 components and stranded Staten Island
/// + every island's bridge/ramp links in discarded components. So we weld by a true
/// *radius* search instead. 2 m stays well under the spacing of distinct NYC intersections,
/// so it never merges two genuine junctions. Overridable via `WELD_TOL_M` env for tuning.
const WELD_TOL_M: f64 = 1.0;

/// Intern a point into a node index, merging it with any existing node within `tol` metres
/// (a radius search over the 3×3 neighbouring grid cells — cell size = `tol` guarantees
/// every in-tolerance node is a neighbour). This welds segments meeting at a shared
/// intersection — or across a small CSCL gap at a bridge/ramp junction — onto one node, so
/// the drivable network stays connected across bridges instead of fragmenting.
fn intern_snapped(
    p: Vec2,
    tol: f64,
    keys: &mut HashMap<(i64, i64), Vec<u32>>,
    points: &mut Vec<NodePoint>,
) -> u32 {
    let cell = |v: f64| (v / tol).floor() as i64;
    let (cx, cy) = (cell(p.x), cell(p.y));
    let mut best: Option<(u32, f64)> = None;
    for dx in -1..=1 {
        for dy in -1..=1 {
            if let Some(v) = keys.get(&(cx + dx, cy + dy)) {
                for &i in v {
                    let q = &points[i as usize];
                    let d = ((q.x - p.x).powi(2) + (q.y - p.y).powi(2)).sqrt();
                    if d <= tol && best.map_or(true, |(_, bd)| d < bd) {
                        best = Some((i, d));
                    }
                }
            }
        }
    }
    if let Some((i, _)) = best {
        return i;
    }
    let i = points.len() as u32;
    points.push(NodePoint { x: p.x, y: p.y });
    keys.entry((cx, cy)).or_default().push(i);
    i
}

/// Weekday (as it appears in Open Streets `apprdayswe`) the trip model represents.
/// Both the Manhattan (2024-06-25) and citywide (2024-06-25) baked days are Tuesdays,
/// so Open-Streets closures active on a Tuesday are dropped from the drive graph while
/// weekend-only closures stay drivable. See `docs/TRIP_MODEL.md` (one real day).
const SIM_WEEKDAY: &str = "Tue";

/// Build the citywide street graph from the NYC CSCL centerline GeoJSON, for either the
/// drivable or the pedestrian network (see [`CsclNetwork`]).
///
/// `parks_geojson` / `open_streets_geojson` are the drivability blacklist's layers 3 and 4 and
/// are **ignored for [`CsclNetwork::Walk`]** — a park interior and a car-free Open Street are
/// exactly where pedestrians *are*, so masking them out of a walk graph would delete the
/// pedestrian network's best streets.
pub fn bake_cscl(
    geojson_path: &str,
    out_path: &str,
    net: CsclNetwork,
    boundary_geojson: Option<&str>,
    parks_geojson: Option<&str>,
    open_streets_geojson: Option<&str>,
) -> anyhow::Result<(usize, usize)> {
    let walk = net == CsclNetwork::Walk;
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
    // Layers 3 and 4 of the *drivability* blacklist. A pedestrian is not subject to either —
    // park interiors and car-free Open Streets are prime walking surface — so the walk graph
    // ignores both masks even when the caller passes them.
    let parks = (!walk)
        .then_some(parks_geojson)
        .flatten()
        .map(crate::boundary::ParkMask::load)
        .transpose()?;
    // Optional Open Streets mask: NYC DOT car-free streets active on the simulated
    // weekday (CSCL still codes them vehicular). Layer 4 of the drivability blacklist.
    let open = (!walk)
        .then_some(open_streets_geojson)
        .flatten()
        .map(|p| crate::boundary::OpenStreetMask::load(p, SIM_WEEKDAY))
        .transpose()?;

    // Endpoint-weld tolerance (see `WELD_TOL_M`); env-overridable for tuning re-bakes.
    let weld_tol: f64 = std::env::var("WELD_TOL_M")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|t: &f64| *t > 0.0)
        .unwrap_or(WELD_TOL_M);
    let mut keys: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    let mut points: Vec<NodePoint> = Vec::new();
    let mut edges: Vec<EdgeData> = Vec::new();
    let (mut total, mut kept, mut dropped_nv, mut dropped_park, mut dropped_open) =
        (0usize, 0usize, 0usize, 0usize, 0usize);

    for f in &fc.features {
        total += 1;
        let Some(geom) = &f.geometry else { continue };
        let p = &f.properties;
        let rw = p.rw_type.as_deref().unwrap_or("");
        // Layer 1 — physical roadway type. Keep only the classes this traveller can use:
        // drive drops paths/steps/boardwalks/driveways/ferries; walk drops highways/ramps.
        let keep_rw = if walk { CSCL_KEEP_RW_WALK } else { CSCL_KEEP_RW };
        if !keep_rw.contains(&rw) {
            continue;
        }
        let rw_class: i64 = rw.parse().unwrap_or(0);
        let trafdir = p.trafdir.as_deref().unwrap_or("");
        let nonped_flag = p.nonped.as_deref().unwrap_or("");
        if walk {
            // Layer 2 (walk) — authoritative **non-pedestrian** flag, the mirror of the drive
            // graph's `trafdir=NV`. `nonped == "V"` is CSCL's "vehicular only": limited-access
            // roadways and the car decks of bridges that carry no footway. It is what removes
            // the Verrazzano (120/121 segments flagged V — you genuinely cannot walk to Staten
            // Island) and the Brooklyn Bridge *roadway*, while leaving the Brooklyn Bridge
            // *promenade* (`trafdir=NV, nonped=D`) in — the one segment pair that proves the
            // two classifiers are duals rather than complements.
            //
            // `nonped == "D"` is explicitly KEPT: it marks park drives and off-grid paths,
            // which the drive graph drops (layer 3) precisely because people, not cars, use them.
            if nonped_flag == "V" {
                dropped_nv += 1;
                continue;
            }
        } else {
            // Layer 2 (drive) — authoritative non-vehicular flag. `trafdir == "NV"` is CSCL's own
            // "not for cars" designation, and catches the cases Layer 1 can't: pedestrian
            // malls / promenades / bike paths that are mis-typed as `rw_type` 1 "Street".
            // Exempt highways/ramps (rw 2/9): an NV flag there is a coding error, not a
            // pedestrianization, and dropping it could sever the FDR / a parkway ramp.
            // Bridges (3) are NOT exempt — a bridge's NV segments are its pedestrian
            // promenade (e.g. the Brooklyn Bridge walkway), which must stay out of the drive
            // graph; the vehicular deck carries trafdir FT/TF and is kept.
            if trafdir == "NV" && !matches!(rw_class, 2 | 9) {
                dropped_nv += 1;
                continue;
            }
        }
        // Posted speed limit (mph) — packed into `segment_id` alongside the class so
        // the time router can use real limits; 0 = unknown → per-class fallback.
        let posted_mph: i64 = p
            .posted_speed
            .as_deref()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0)
            .clamp(0, 60);
        let nonped = nonped_flag;
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
            let a = intern_snapped(verts[0], weld_tol, &mut keys, &mut points);
            let b = intern_snapped(*verts.last().unwrap(), weld_tol, &mut keys, &mut points);
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
    if walk {
        eprintln!(
            "CSCL WALK network: {kept}/{total} segments kept (rw_type {CSCL_KEEP_RW_WALK:?}); \
             dropped {dropped_nv} non-pedestrian (nonped=V — highways, car-only bridge decks). \
             Park interiors and Open Streets are KEPT (they are pedestrian space)."
        );
    } else {
        eprintln!(
            "CSCL DRIVE network: {kept}/{total} segments kept (rw_type {CSCL_KEEP_RW:?}); dropped \
             {dropped_nv} non-vehicular (trafdir=NV) + {dropped_park} closed park drives \
             (nonped=D in a park) + {dropped_open} Open-Streets blocks (car-free on {SIM_WEEKDAY})"
        );
    }

    // Reconnect severed interchange/ramp fragments (within `stitch_max`) before pruning, so
    // the keep-largest step no longer silently deletes on/off-ramps that meet the surface
    // grid across a small planar gap. Env-overridable ceiling stays below open-water spans.
    let stitch_max: f64 = std::env::var("STITCH_MAX_M")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|t: &f64| *t > 0.0)
        .unwrap_or(STITCH_MAX_M);
    let (points, edges) = stitch_components(points, edges, stitch_max);
    // Repair real vehicular bridges whose deck chain is split by a centre-span digitization
    // gap (chiefly the Verrazzano → Staten Island), so the island joins the routable network
    // across its actual bridge. Bridge-class nodes on both ends only — ferry-only islands stay
    // out. Env-overridable ceiling stays below any unrelated cross-bridge distance.
    let bridge_gap: f64 = std::env::var("BRIDGE_STITCH_MAX_M")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|t: &f64| *t > 0.0)
        .unwrap_or(BRIDGE_STITCH_MAX_M);
    let (points, edges) = stitch_bridges(points, edges, bridge_gap);
    // Keep every real landmass component (not just the largest) so any island still lacking a
    // drivable in-data bridge (Governors — ferry only) stays in the graph for local residential
    // exposure. Its block groups then snap to their own local streets instead of across the
    // harbour, and A* still returns no-path across the water.
    let min_comp: usize = std::env::var("MIN_COMPONENT_NODES")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|t: &usize| *t > 0)
        .unwrap_or(MIN_COMPONENT_NODES);
    let (nodes, edges) = keep_components_above(points, edges, min_comp);
    let (n, m) = (nodes.len(), edges.len());
    // Sanity: the ENU node bbox should span the whole city (~40.49–40.92 N,
    // −74.26–−73.70 W) — Staten Island now pulls the south-west corner out to ~40.50/−74.26.
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
                    grade-separated bridge/ramp fragments stitched across <=75 m gaps; all \
                    street-network components >=50 nodes retained (Staten Island + City Island / \
                    Rikers / Governors kept as their own components — no fully-digitized drivable \
                    bridge to the mainland in CSCL; A* returns no-path across the harbour)."
                .into(),
        },
    };
    std::fs::write(out_path, asset.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("CSCL graph: {n} nodes, {m} edges (all components >={min_comp} nodes) -> {out_path}");
    Ok((n, m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(x: f64, y: f64) -> NodePoint {
        NodePoint { x, y }
    }
    fn edge(from: u32, to: u32) -> EdgeData {
        EdgeData { from, to, length_m: 1.0, polyline: vec![], segment_id: Some(100) }
    }

    #[test]
    fn stitch_reconnects_near_fragment_but_not_across_water() {
        // Main component: a chain of 3 nodes near the origin.
        // Fragment A: 2 nodes a 40 m gap away (a severed ramp) — should stitch in.
        // Fragment B: 2 nodes 500 m away (an island across water) — must stay out.
        let points = vec![
            node(0.0, 0.0),   // 0 main
            node(10.0, 0.0),  // 1 main
            node(20.0, 0.0),  // 2 main
            node(60.0, 0.0),  // 3 fragment A (40 m from node 2)
            node(70.0, 0.0),  // 4 fragment A
            node(520.0, 0.0), // 5 fragment B (island)
            node(530.0, 0.0), // 6 fragment B
        ];
        let edges = vec![edge(0, 1), edge(1, 2), edge(3, 4), edge(5, 6)];
        let (_pts, out) = stitch_components(points, edges, 75.0);
        // A connector must now bridge fragment A (node 3 or 4) to the main chain (node 2)…
        let a_linked = out.iter().any(|e| {
            let s = [e.from, e.to];
            (s.contains(&2) && (s.contains(&3) || s.contains(&4)))
                && e.segment_id == Some(CONNECTOR_SEGMENT_ID)
        });
        assert!(a_linked, "near 40 m fragment should be stitched to the main network");
        // …but nothing may connect the 500 m island (nodes 5/6) to anything else.
        let b_linked = out.iter().any(|e| {
            let s = [e.from, e.to];
            (s.contains(&5) || s.contains(&6)) && !(s.contains(&5) && s.contains(&6))
        });
        assert!(!b_linked, "island across 500 m of water must not be stitched");
    }

    #[test]
    fn stitch_then_largest_component_keeps_the_reconnected_fragment() {
        // After stitching, keep-largest must retain fragment A but drop island B.
        let points = vec![
            node(0.0, 0.0),
            node(10.0, 0.0),
            node(20.0, 0.0),
            node(60.0, 0.0),
            node(70.0, 0.0),
            node(520.0, 0.0),
            node(530.0, 0.0),
        ];
        let edges = vec![edge(0, 1), edge(1, 2), edge(3, 4), edge(5, 6)];
        let (pts, es) = stitch_components(points, edges, 75.0);
        let (nodes, _) = largest_component(pts, es);
        // Main (3) + fragment A (2) = 5 kept; island B (2) dropped.
        assert_eq!(nodes.len(), 5, "reconnected fragment kept, water island dropped");
    }

    #[test]
    fn keep_components_above_retains_every_landmass_over_threshold() {
        // Two disconnected landmasses over threshold — a 5-node "mainland" and a 4-node
        // Staten-Island-like component with no edge between them (the Verrazzano gap) — plus a
        // 2-node stub fragment. keep_components_above must retain BOTH landmasses (unlike
        // largest_component, which would drop the island) and drop only the sub-threshold stub.
        let points = vec![
            node(0.0, 0.0), node(10.0, 0.0), node(20.0, 0.0), node(30.0, 0.0), node(40.0, 0.0),
            node(0.0, 5000.0), node(10.0, 5000.0), node(20.0, 5000.0), node(30.0, 5000.0),
            node(0.0, -5000.0), node(10.0, -5000.0),
        ];
        let edges = vec![
            edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 4), // mainland (5 nodes)
            edge(5, 6), edge(6, 7), edge(7, 8), // island (4 nodes)
            edge(9, 10), // stub (2 nodes) — below threshold
        ];
        let (nodes, out) = keep_components_above(points, edges, 3);
        assert_eq!(nodes.len(), 9, "both components >=3 nodes kept; sub-3 stub dropped");
        assert_eq!(out.len(), 7, "stub's edge dropped, all landmass edges kept");
        // Every surviving edge must reference a valid compact node index.
        assert!(
            out.iter().all(|e| (e.from as usize) < nodes.len() && (e.to as usize) < nodes.len()),
            "edge endpoints remapped into the compact kept-node range"
        );
        // With a threshold above the island size, only the largest survives (largest_component parity).
        let points2 = vec![
            node(0.0, 0.0), node(10.0, 0.0), node(20.0, 0.0), node(30.0, 0.0), node(40.0, 0.0),
            node(0.0, 5000.0), node(10.0, 5000.0), node(20.0, 5000.0), node(30.0, 5000.0),
            node(0.0, -5000.0), node(10.0, -5000.0),
        ];
        let edges2 = vec![
            edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 4),
            edge(5, 6), edge(6, 7), edge(7, 8),
            edge(9, 10),
        ];
        let (nodes2, _) = keep_components_above(points2, edges2, 5);
        assert_eq!(nodes2.len(), 5, "threshold above island size keeps only the mainland");
    }

    #[test]
    fn stitch_bridges_reconnects_split_bridge_but_not_ferry_or_far_island() {
        // Bridge-class edge (rw 3, segment_id 300) helper.
        let bridge = |a: u32, b: u32| EdgeData {
            from: a,
            to: b,
            length_m: 1.0,
            polyline: vec![],
            segment_id: Some(300),
        };
        // Main {0,1,2} with a bridge stub at node 2. Island A {3,4}: a bridge deck 500 m away
        // (both ends bridge-class → reconnect). Island B {5,6}: a *surface* fragment 520 m away
        // (ferry-island analogue → must NOT reconnect). Island C {7,8}: a bridge >700 m from
        // BOTH the mainland and island A (so no direct link and no A→C chain hop → must NOT
        // reconnect).
        let points = vec![
            node(0.0, 0.0), node(10.0, 0.0), node(20.0, 0.0),
            node(520.0, 0.0), node(530.0, 0.0),
            node(20.0, 520.0), node(20.0, 530.0),
            node(1300.0, 0.0), node(1310.0, 0.0),
        ];
        let edges = vec![edge(0, 1), bridge(1, 2), bridge(3, 4), edge(5, 6), bridge(7, 8)];
        let (_pts, out) = stitch_bridges(points, edges, 700.0);
        let linked = |a: u32, grp: [u32; 2]| {
            out.iter().any(|e| {
                let s = [e.from, e.to];
                s.contains(&a) && (s.contains(&grp[0]) || s.contains(&grp[1])) && e.segment_id == Some(300)
            })
        };
        assert!(linked(2, [3, 4]), "split bridge (both ends bridge-class, <=700 m) must reconnect");
        let b_linked = out.iter().any(|e| {
            let s = [e.from, e.to];
            (s.contains(&5) || s.contains(&6)) && !(s.contains(&5) && s.contains(&6))
        });
        assert!(!b_linked, "surface (non-bridge) island must not be bridge-stitched");
        let c_linked = out.iter().any(|e| {
            let s = [e.from, e.to];
            (s.contains(&7) || s.contains(&8)) && !(s.contains(&7) && s.contains(&8))
        });
        assert!(!c_linked, "bridge island beyond the ceiling must not be reconnected");
    }
}
