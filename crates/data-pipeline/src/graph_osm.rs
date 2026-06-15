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

pub fn bake(json_path: &str, out_path: &str) -> anyhow::Result<(usize, usize)> {
    let data = std::fs::read(json_path).with_context(|| format!("reading {json_path}"))?;
    let resp: OverpassResponse =
        serde_json::from_slice(&data).context("parsing Overpass JSON")?;

    let proj = EnuProjection::default();

    // 1. Node coords (projected) + ways.
    let mut coords: HashMap<i64, Vec2> = HashMap::new();
    let mut ways: Vec<(i64, Vec<i64>)> = Vec::new();
    for el in &resp.elements {
        match el.kind.as_str() {
            "node" => {
                if let (Some(la), Some(lo)) = (el.lat, el.lon) {
                    coords.insert(el.id, proj.to_enu(la, lo));
                }
            }
            "way" if el.nodes.len() >= 2 => ways.push((el.id, el.nodes.clone())),
            _ => {}
        }
    }

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
                        let a = intern(ns[start], &coords, &mut id_to_index, &mut points);
                        let b = intern(ns[pos], &coords, &mut id_to_index, &mut points);
                        if let (Some(a), Some(b)) = (a, b) {
                            if a != b && len > 0.0 {
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

    // 4. Largest connected component.
    let mut uf = UnionFind::new(points.len());
    for e in &edges {
        uf.union(e.from, e.to);
    }
    let mut comp_size: HashMap<u32, u32> = HashMap::new();
    for i in 0..points.len() as u32 {
        let r = uf.find(i);
        *comp_size.entry(r).or_default() += 1;
    }
    let largest_root = comp_size
        .iter()
        .max_by_key(|(_, &s)| s)
        .map(|(&r, _)| r)
        .context("no components")?;

    // 5. Remap kept nodes to compact indices.
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

    let (n, m) = (new_nodes.len(), new_edges.len());
    let asset = GraphAsset {
        origin: GeoOrigin::MANHATTAN,
        nodes: new_nodes,
        edges: new_edges,
        provenance: Provenance {
            source: "OpenStreetMap via Overpass API (walk network)".into(),
            url: "https://overpass-api.de/".into(),
            license: "ODbL 1.0".into(),
            as_of: "2026-06-14".into(),
            notes: "Manhattan pedestrian-usable highways; largest connected component; \
                    street-centerline (not sidewalk-accurate)."
                .into(),
        },
    };
    std::fs::write(out_path, asset.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!("OSM walk graph: {n} nodes, {m} edges (largest component) -> {out_path}");
    Ok((n, m))
}
