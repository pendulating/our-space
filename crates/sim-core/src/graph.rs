//! The routable pedestrian graph and A* routing, plus the position-over-time
//! sampling that decouples routing from exposure (route once, then expose).

use crate::assets::GraphAsset;
use crate::math::Vec2;
use crate::projection::GeoOrigin;
use crate::rng::RngLike;
use petgraph::algo::{astar, dijkstra};
use petgraph::graph::{NodeIndex, UnGraph};
use petgraph::visit::EdgeRef;
use rstar::{PointDistance, RTree, RTreeObject, AABB};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RouteError {
    #[error("no walkable path exists between the chosen points")]
    NoPath,
    #[error("graph is empty")]
    Empty,
}

/// Default pedestrian walking speed (m/s) — ~4.8 km/h.
pub const DEFAULT_WALK_SPEED_MPS: f64 = 1.34;

/// mph → m/s.
const MPH: f64 = 0.44704;

/// Heuristic ceiling (m/s) for the time-based A*: the straight-line ÷ max-speed
/// heuristic is admissible only if `max_speed` is ≥ every edge's speed. NYC's
/// fastest posted limit in CSCL is 50 mph; 60 gives headroom, and `class_speed_mps`
/// is capped here so no edge can ever exceed it.
pub const MAX_DRIVE_SPEED_MPS: f64 = 60.0 * MPH; // 26.82 m/s

/// Decode the CSCL drive-graph `EdgeData.segment_id` into `(rw_type, posted_mph)`.
/// To avoid a graph-asset format change, the drive bake packs both the road class
/// and the posted speed limit into the one `i64`: `segment_id = rw_type * 100 +
/// posted_mph` (posted_mph 0 = unknown). The ranges don't overlap, so we can still
/// read a **legacy** graph that stored the bare `rw_type` (1..=14): values ≤ 14 are
/// taken as a raw class with unknown speed. Anything else (None, or the raw OSM way
/// id the *walk* graph stores here) decodes to `(0, 0)` → the 25 mph default. Only
/// the drive graph's timed router and coverage call this, so the walk-graph way-id
/// case is never actually hit.
pub fn unpack_class(segment_id: Option<i64>) -> (i64, i64) {
    match segment_id {
        Some(s) if (0..=14).contains(&s) => (s, 0), // legacy: bare rw_type
        Some(s) if s >= 100 => (s.div_euclid(100), s.rem_euclid(100)), // packed rw*100+mph
        _ => (0, 0),
    }
}

/// Free-flow speed (m/s) for a CSCL drive-graph edge, from its packed `segment_id`.
/// Prefers the segment's **posted speed limit** when the bake recorded one; else
/// falls back to a per-class default — surface streets get NYC's 25 mph, highways /
/// ramps / bridges / tunnels are faster, so time-based routing prefers them for fast
/// trips (revealing FDR / Henry Hudson / West Side Hwy use). Unknown class/speed
/// (`None`) → 25 mph, making the time router equivalent to shortest-distance on a
/// graph without road classes. Always capped at [`MAX_DRIVE_SPEED_MPS`].
pub fn class_speed_mps(segment_id: Option<i64>) -> f64 {
    let (rw, posted_mph) = unpack_class(segment_id);
    // Trust the packed posted limit only in a plausible NYC range. The OSM builder path
    // historically stored raw way-ids in `segment_id`, which this decoder would otherwise
    // read as a garbage 1–99 mph "posted limit" on every edge; out-of-range values fall
    // back to the per-class default instead.
    let mph = if (15..=65).contains(&posted_mph) {
        posted_mph as f64 // segment's own posted limit
    } else {
        match rw {
            2 => 40.0, // Highway (FDR, Henry Hudson, West Side Hwy)
            9 => 30.0, // Ramp
            3 => 30.0, // Bridge
            4 => 35.0, // Tunnel
            _ => 25.0, // Street / Alley / unknown — NYC default limit
        }
    };
    (mph * MPH).min(MAX_DRIVE_SPEED_MPS)
}

/// A routable pedestrian graph in local ENU meters.
pub struct StreetGraph {
    origin: GeoOrigin,
    positions: Vec<Vec2>,
    asset: GraphAsset,
    /// Undirected graph; edge weight = index into `asset.edges`.
    g: UnGraph<(), usize>,
    /// R-tree over node positions for O(log n) nearest-node queries.
    node_tree: RTree<NodePoint2D>,
    /// CSR graph for fast distance-based A* routing (5–10× faster than petgraph).
    fast_graph: fast_paths::FastGraph,
}

struct NodePoint2D {
    pos: [f64; 2],
    id: u32,
}

impl RTreeObject for NodePoint2D {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.pos)
    }
}

impl PointDistance for NodePoint2D {
    fn distance_2(&self, point: &[f64; 2]) -> f64 {
        let dx = self.pos[0] - point[0];
        let dy = self.pos[1] - point[1];
        dx * dx + dy * dy
    }
}

impl StreetGraph {
    pub fn from_asset(asset: GraphAsset) -> Self {
        let positions: Vec<Vec2> = asset.nodes.iter().map(|n| Vec2::new(n.x, n.y)).collect();
        let mut g: UnGraph<(), usize> = UnGraph::with_capacity(positions.len(), asset.edges.len());
        for _ in 0..positions.len() {
            g.add_node(());
        }
        for (i, e) in asset.edges.iter().enumerate() {
            g.add_edge(NodeIndex::new(e.from as usize), NodeIndex::new(e.to as usize), i);
        }
        let node_tree = RTree::bulk_load(
            positions
                .iter()
                .enumerate()
                .map(|(i, p)| NodePoint2D { pos: [p.x, p.y], id: i as u32 })
                .collect(),
        );
        let mut input_graph = fast_paths::InputGraph::new();
        for e in &asset.edges {
            let w = ((e.length_m * 1000.0).round() as usize).max(1);
            input_graph.add_edge(e.from as usize, e.to as usize, w);
            input_graph.add_edge(e.to as usize, e.from as usize, w);
        }
        input_graph.freeze();
        let fast_graph = fast_paths::prepare(&input_graph);
        StreetGraph {
            origin: asset.origin,
            positions,
            asset,
            g,
            node_tree,
            fast_graph,
        }
    }

    pub fn origin(&self) -> GeoOrigin {
        self.origin
    }
    /// The underlying baked asset (node positions + edge polylines), e.g. for
    /// rendering the street network.
    pub fn asset(&self) -> &GraphAsset {
        &self.asset
    }
    pub fn node_count(&self) -> usize {
        self.positions.len()
    }
    pub fn edge_count(&self) -> usize {
        self.asset.edges.len()
    }
    pub fn node_pos(&self, id: u32) -> Vec2 {
        self.positions[id as usize]
    }

    /// Nearest graph node to a point (R-tree nearest-neighbor, O(log n)).
    pub fn snap_nearest(&self, p: Vec2) -> Option<u32> {
        self.node_tree
            .nearest_neighbor(&[p.x, p.y])
            .map(|n| n.id)
    }

    /// Route between two graph node ids using fast_paths CSR A* (distance-weighted).
    pub fn route(&self, start: u32, goal: u32) -> Result<Route, RouteError> {
        if self.positions.is_empty() {
            return Err(RouteError::Empty);
        }
        if start as usize >= self.positions.len() || goal as usize >= self.positions.len() {
            return Err(RouteError::NoPath);
        }
        if start == goal {
            return Ok(self.build_route(&[NodeIndex::new(start as usize)]));
        }
        let si = NodeIndex::new(start as usize);
        let gi = NodeIndex::new(goal as usize);
        if self.g.edges(si).next().is_none() || self.g.edges(gi).next().is_none() {
            return Err(RouteError::NoPath);
        }
        let sp = fast_paths::calc_path(&self.fast_graph, start as usize, goal as usize)
            .ok_or(RouteError::NoPath)?;
        let nodes: Vec<NodeIndex> = sp.get_nodes().iter().map(|&n| NodeIndex::new(n)).collect();
        Ok(self.build_route(&nodes))
    }

    /// Convenience: snap two ENU points to the nearest nodes, validate they are
    /// connected, and route. Mirrors the OSMnx snap+validate workflow.
    pub fn route_points(&self, from: Vec2, to: Vec2) -> Result<Route, RouteError> {
        let a = self.snap_nearest(from).ok_or(RouteError::Empty)?;
        let b = self.snap_nearest(to).ok_or(RouteError::Empty)?;
        self.route(a, b)
    }

    /// Route by **free-flow time** (edge cost = `length / class_speed`) instead of
    /// distance, so fast/long trips take the highways. Used for the dashcam-vehicle
    /// (taxi) routes. On a graph without road classes (`segment_id` = None) every
    /// edge gets the 25 mph default, making this equivalent to shortest-distance.
    pub fn route_timed(&self, start: u32, goal: u32) -> Result<Route, RouteError> {
        if self.positions.is_empty() {
            return Err(RouteError::Empty);
        }
        // Guard against out-of-range node ids so a bad caller gets an error, not a panic
        // on the `positions[goal]` index / petgraph lookups below.
        if start as usize >= self.positions.len() || goal as usize >= self.positions.len() {
            return Err(RouteError::NoPath);
        }
        let start_ix = NodeIndex::new(start as usize);
        let goal_ix = NodeIndex::new(goal as usize);
        let goal_pos = self.positions[goal as usize];
        // Admissible heuristic: straight-line distance ÷ the fastest possible edge
        // speed never overestimates remaining travel time.
        let result = astar(
            &self.g,
            start_ix,
            |n| n == goal_ix,
            |e| {
                let edge = &self.asset.edges[*e.weight()];
                edge.length_m / class_speed_mps(edge.segment_id)
            },
            |n| self.positions[n.index()].distance(goal_pos) / MAX_DRIVE_SPEED_MPS,
        );
        let (_cost, path) = result.ok_or(RouteError::NoPath)?;
        Ok(self.build_route(&path))
    }

    /// Snap two ENU points to the nearest nodes and route by free-flow time.
    pub fn route_points_timed(&self, from: Vec2, to: Vec2) -> Result<Route, RouteError> {
        let a = self.snap_nearest(from).ok_or(RouteError::Empty)?;
        let b = self.snap_nearest(to).ok_or(RouteError::Empty)?;
        self.route_timed(a, b)
    }

    /// Route by free-flow time, multiplying the cost of highway/ramp edges (CSCL
    /// `rw_type` 2 / 9, carried in `segment_id`) by `hw_penalty`. `hw_penalty = 1.0`
    /// is the plain fastest path; a large penalty forces the surface-street
    /// alternative. Returns the route, its true (un-penalized) free-flow time in
    /// seconds (the `t_r` the route-inference likelihood scores against), and the
    /// list of **edge indices** it traverses (for the sensing-power metric).
    pub fn route_timed_pen(
        &self,
        start: u32,
        goal: u32,
        hw_penalty: f64,
    ) -> Result<(Route, f64, Vec<u32>), RouteError> {
        if self.positions.is_empty() {
            return Err(RouteError::Empty);
        }
        // Guard against out-of-range node ids so a bad caller gets an error, not a panic
        // on the `positions[goal]` index / petgraph lookups below.
        if start as usize >= self.positions.len() || goal as usize >= self.positions.len() {
            return Err(RouteError::NoPath);
        }
        let start_ix = NodeIndex::new(start as usize);
        let goal_ix = NodeIndex::new(goal as usize);
        let goal_pos = self.positions[goal as usize];
        // Highway/ramp = CSCL rw_type 2 / 9 (decoded from the packed segment_id).
        let is_hw = |sid: Option<i64>| matches!(unpack_class(sid).0, 2 | 9);
        let result = astar(
            &self.g,
            start_ix,
            |n| n == goal_ix,
            |e| {
                let edge = &self.asset.edges[*e.weight()];
                let t = edge.length_m / class_speed_mps(edge.segment_id);
                if is_hw(edge.segment_id) {
                    t * hw_penalty
                } else {
                    t
                }
            },
            |n| self.positions[n.index()].distance(goal_pos) / MAX_DRIVE_SPEED_MPS,
        );
        let (_cost, path) = result.ok_or(RouteError::NoPath)?;
        // True free-flow seconds + traversed edge indices along the chosen path.
        let mut t_r = 0.0;
        let mut edges = Vec::with_capacity(path.len().saturating_sub(1));
        for pair in path.windows(2) {
            if let Some(ei) = self.g.find_edge(pair[0], pair[1]) {
                let e = self.g[ei];
                t_r += self.asset.edges[e].length_m / class_speed_mps(self.asset.edges[e].segment_id);
                edges.push(e as u32);
            }
        }
        Ok((self.build_route(&path), t_r, edges))
    }

    /// All street reachable on foot within `max_seconds` of `start` (a walkshed
    /// / isochrone), via Dijkstra with time-weighted edges.
    pub fn walkshed(&self, start: u32, max_seconds: f64, speed_mps: f64) -> Walkshed {
        let costs = dijkstra(
            &self.g,
            NodeIndex::new(start as usize),
            None,
            |e| self.asset.edges[*e.weight()].length_m / speed_mps,
        );
        let node_time: HashMap<u32, f64> = costs
            .iter()
            .filter(|(_, c)| **c <= max_seconds)
            .map(|(n, c)| (n.index() as u32, *c))
            .collect();
        let mut dense = vec![f64::INFINITY; self.positions.len()];
        for (&nid, &t) in &node_time {
            dense[nid as usize] = t;
        }
        let mut edges: Vec<u32> = Vec::new();
        let mut partial: Vec<(u32, u32, f64)> = Vec::new();
        for (i, e) in self.asset.edges.iter().enumerate() {
            let (tf, tt) = (dense[e.from as usize], dense[e.to as usize]);
            match (tf.is_finite(), tt.is_finite()) {
                (true, true) => edges.push(i as u32),
                (true, false) | (false, true) if e.length_m > 0.0 => {
                    let t = tf.min(tt);
                    let frac = ((max_seconds - t) * speed_mps / e.length_m).clamp(0.0, 1.0);
                    if frac > 0.0 {
                        let entry = if tf.is_finite() { e.from } else { e.to };
                        partial.push((i as u32, entry, frac));
                    }
                }
                _ => {}
            }
        }
        Walkshed {
            start,
            max_seconds,
            node_time,
            edges,
            partial,
        }
    }

    /// Incident edges of `node` as `(other_node_id, edge_index)` pairs. Used by
    /// the ambient pedestrian agents' random walk (no routing).
    pub fn neighbors(&self, node: u32) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        self.neighbors_into(node, &mut out);
        out
    }

    /// Fill `out` with the incident edges of `node` (clear-and-reuse pattern).
    pub fn neighbors_into(&self, node: u32, out: &mut Vec<(u32, u32)>) {
        out.clear();
        let ni = NodeIndex::new(node as usize);
        for e in self.g.edges(ni) {
            let other = if e.source() == ni { e.target() } else { e.source() };
            out.push((other.index() as u32, *e.weight() as u32));
        }
    }

    /// One random-walk step from `node`: pick a uniform random incident edge,
    /// avoiding an immediate U-turn back to `prev` when another option exists.
    /// Returns `(next_node, edge_index)`, or `None` at a dead end. O(degree).
    pub fn random_walk_step(
        &self,
        node: u32,
        prev: Option<u32>,
        rng: &mut impl RngLike,
    ) -> Option<(u32, u32)> {
        let mut opts: Vec<(u32, u32)> = Vec::with_capacity(6);
        self.neighbors_into(node, &mut opts);
        if opts.is_empty() {
            return None;
        }
        if opts.len() > 1 {
            if let Some(p) = prev {
                let filtered: Vec<(u32, u32)> = opts.iter().copied().filter(|&(n, _)| n != p).collect();
                if !filtered.is_empty() {
                    opts = filtered;
                }
            }
        }
        Some(opts[rng.below(opts.len())])
    }

    /// Build a `Route` by random-walking up to `max_edges` steps from `start`,
    /// reusing the same polyline stitching as routed paths. Cheap, O(max_edges),
    /// and naturally organic — used for the wandering smart-glasses pedestrians.
    pub fn random_walk_route(&self, start: u32, max_edges: usize, rng: &mut impl RngLike) -> Route {
        let mut path = vec![NodeIndex::new(start as usize)];
        let mut prev: Option<u32> = None;
        let mut cur = start;
        for _ in 0..max_edges {
            match self.random_walk_step(cur, prev, rng) {
                Some((next, _edge)) => {
                    path.push(NodeIndex::new(next as usize));
                    prev = Some(cur);
                    cur = next;
                }
                None => break,
            }
        }
        self.build_route(&path)
    }

    /// Stitch the node path into a continuous ENU polyline, orienting each
    /// edge's stored geometry in the direction of travel.
    fn build_route(&self, path: &[NodeIndex]) -> Route {
        let mut points: Vec<Vec2> = Vec::new();
        for pair in path.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let edge_geom = self
                .g
                .find_edge(a, b)
                .map(|ei| &self.asset.edges[self.g[ei]]);
            match edge_geom {
                Some(edge) => {
                    // Stored polyline runs from `edge.from` to `edge.to`; reverse
                    // if we are traversing it the other way.
                    let forward = edge.from as usize == a.index();
                    let mut seg: Vec<Vec2> =
                        edge.polyline.iter().map(|p| Vec2::new(p[0], p[1])).collect();
                    if !forward {
                        seg.reverse();
                    }
                    push_polyline(&mut points, &seg);
                }
                None => {
                    // Fallback: straight line between node centers.
                    push_polyline(&mut points, &[self.positions[a.index()], self.positions[b.index()]]);
                }
            }
        }
        if points.is_empty() {
            if let Some(&single) = path.first() {
                points.push(self.positions[single.index()]);
            }
        }
        Route::from_points(points)
    }
}

/// Append `seg` to `out`, skipping a duplicated joint vertex.
fn push_polyline(out: &mut Vec<Vec2>, seg: &[Vec2]) {
    if seg.is_empty() {
        return;
    }
    let start = if out
        .last()
        .map(|l| l.distance(seg[0]) < 1e-6)
        .unwrap_or(false)
    {
        1
    } else {
        0
    };
    out.extend_from_slice(&seg[start..]);
}

/// A routed path as a continuous ENU polyline with cumulative arc length, used
/// to sample position(t) at constant walking speed.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub points: Vec<Vec2>,
    pub cumulative_m: Vec<f64>,
    pub total_m: f64,
}

impl Route {
    pub fn from_points(points: Vec<Vec2>) -> Self {
        let mut cumulative = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                total += points[i - 1].distance(*p);
            }
            cumulative.push(total);
        }
        Route {
            points,
            cumulative_m: cumulative,
            total_m: total,
        }
    }

    /// Position at arc-length `d` meters from the start (clamped to the route).
    pub fn position_at(&self, d: f64) -> Vec2 {
        if self.points.is_empty() {
            return Vec2::ZERO;
        }
        if self.points.len() == 1 || d <= 0.0 {
            return self.points[0];
        }
        if d >= self.total_m {
            return *self.points.last().unwrap();
        }
        // Find the segment containing `d`.
        let seg = match self
            .cumulative_m
            .binary_search_by(|c| c.partial_cmp(&d).unwrap_or(std::cmp::Ordering::Equal))
        {
            Ok(i) => i,
            Err(i) => i, // first cumulative strictly greater than d
        };
        let i1 = seg.min(self.points.len() - 1);
        let i0 = i1 - 1;
        let seg_len = self.cumulative_m[i1] - self.cumulative_m[i0];
        let t = if seg_len > 0.0 {
            (d - self.cumulative_m[i0]) / seg_len
        } else {
            0.0
        };
        self.points[i0].lerp(self.points[i1], t)
    }

    /// Unit travel direction at arc-length `d` (for orienting a moving sprite).
    /// Finite difference of `position_at`; falls back to +x for a degenerate
    /// route. O(log n).
    pub fn heading_at(&self, d: f64) -> Vec2 {
        if self.total_m <= 0.0 {
            return Vec2::new(1.0, 0.0);
        }
        let eps = 0.5_f64.min(self.total_m * 0.5);
        let a = self.position_at((d - eps).max(0.0));
        let b = self.position_at((d + eps).min(self.total_m));
        b.sub(a).normalize()
    }

    /// Position **and** unit heading at arc-length `d`, walking forward from a
    /// caller-owned segment cursor (`hint`). Amortized O(1) per call when `d`
    /// advances monotonically (the agent case). The heading is the current
    /// segment's direction — exact, no finite-difference second lookup.
    pub fn position_and_heading_at(&self, d: f64, hint: &mut usize) -> (Vec2, Vec2) {
        if self.points.is_empty() {
            return (Vec2::ZERO, Vec2::new(1.0, 0.0));
        }
        if self.points.len() == 1 || d <= 0.0 {
            let h = if self.points.len() >= 2 {
                self.points[1].sub(self.points[0]).normalize()
            } else {
                Vec2::new(1.0, 0.0)
            };
            return (self.points[0], h);
        }
        if d >= self.total_m {
            let n = self.points.len();
            let h = if n >= 2 {
                self.points[n - 1].sub(self.points[n - 2]).normalize()
            } else {
                Vec2::new(1.0, 0.0)
            };
            return (*self.points.last().unwrap(), h);
        }
        while *hint + 2 < self.points.len() && self.cumulative_m[*hint + 1] < d {
            *hint += 1;
        }
        let i1 = (*hint + 1).min(self.points.len() - 1);
        let i0 = i1.saturating_sub(1);
        let seg_len = self.cumulative_m[i1] - self.cumulative_m[i0];
        let t = if seg_len > 0.0 { (d - self.cumulative_m[i0]) / seg_len } else { 0.0 };
        let pos = self.points[i0].lerp(self.points[i1], t);
        let dir = self.points[i1].sub(self.points[i0]);
        let heading = if dir.length() > 1e-12 { dir.normalize() } else { Vec2::new(1.0, 0.0) };
        (pos, heading)
    }

    /// Sample (elapsed_seconds, position) at a fixed time step `dt` while
    /// walking at `speed` m/s. The final sample lands exactly on the endpoint.
    /// Uses a single linear pass over segments (O(n) total) instead of a
    /// binary search per sample.
    pub fn sample_over_time(&self, speed: f64, dt: f64) -> Vec<(f64, Vec2)> {
        let mut out = Vec::new();
        if self.points.is_empty() || speed <= 0.0 || dt <= 0.0 {
            return out;
        }
        let duration = self.total_m / speed;
        let n_pts = self.points.len();
        let mut seg = 0usize;
        let mut t = 0.0;
        while t < duration {
            let d = t * speed;
            while seg + 2 < n_pts && self.cumulative_m[seg + 1] < d {
                seg += 1;
            }
            let i1 = (seg + 1).min(n_pts - 1);
            let i0 = i1.saturating_sub(1);
            let seg_len = self.cumulative_m[i1] - self.cumulative_m[i0];
            let frac = if seg_len > 0.0 { (d - self.cumulative_m[i0]) / seg_len } else { 0.0 };
            out.push((t, self.points[i0].lerp(self.points[i1], frac)));
            t += dt;
        }
        out.push((duration, *self.points.last().unwrap()));
        out
    }
}

/// A turn-aware, speed-limit-capped pace for replaying a routed trip. Instead of
/// gliding the whole origin→destination route at one constant speed, a vehicle
/// **decelerates into turns and cruises on straights** (cruise capped at the posted
/// limit) while still completing the trip in its real duration. `time_frac[i]` is
/// the fraction of total trip time elapsed when the vehicle reaches route point `i`
/// — so mapping the trip's elapsed-time fraction through it yields a piecewise pace.
///
/// Honesty about the limit: the profile shapes the trip relative to a limit-paced
/// reference. When the trip's average speed is at/below the limit (the overwhelming
/// majority of Manhattan trips, which crawl in traffic), every instantaneous speed
/// stays at/below the limit. A trip whose data is *faster* than even a limit-paced
/// traversal is replayed faithfully (the record itself implies speeding).
#[derive(Debug, Clone)]
pub struct PaceProfile {
    /// Cumulative fraction of total trip time at each route point (monotonic,
    /// `[0]==0`, `last==1`); same length as the route's points.
    pub time_frac: Vec<f64>,
}

impl PaceProfile {
    /// Build the pace for `route`: `limit_mps` caps cruise speed; a vertex whose turn
    /// is sharp is taken at down to `turn_min_frac` of the limit (decelerate through
    /// corners). Degenerate routes fall back to a constant pace (linear time↔arc).
    pub fn for_route(route: &Route, limit_mps: f64, turn_min_frac: f64) -> Self {
        let n = route.points.len();
        if n < 2 || route.total_m <= 0.0 {
            return PaceProfile { time_frac: vec![0.0; n.max(1)] };
        }
        // Per-vertex speed factor in [turn_min_frac, 1]: straight = 1, hairpin → min.
        // Endpoints cruise. Interior vertices use the angle between the incoming and
        // outgoing directions (dot of unit dirs = cos θ; 1 straight, -1 reversal).
        let speed_factor = |i: usize| -> f64 {
            if i == 0 || i + 1 >= n {
                return 1.0;
            }
            let din = route.points[i].sub(route.points[i - 1]).normalize();
            let dout = route.points[i + 1].sub(route.points[i]).normalize();
            let straightness = ((din.dot(dout) + 1.0) * 0.5).clamp(0.0, 1.0); // 0..1
            turn_min_frac + (1.0 - turn_min_frac) * straightness
        };
        let limit = limit_mps.max(0.1);
        // Travel time of each segment i→i+1 at the slower of its two endpoints' speeds
        // (so a vehicle is already slow approaching a corner and accelerating away).
        let mut dt = Vec::with_capacity(n - 1);
        let mut total_t = 0.0;
        for i in 0..n - 1 {
            let seg_len = route.cumulative_m[i + 1] - route.cumulative_m[i];
            let v = limit * speed_factor(i).min(speed_factor(i + 1));
            let d = seg_len / v.max(1e-6);
            dt.push(d);
            total_t += d;
        }
        let mut time_frac = Vec::with_capacity(n);
        time_frac.push(0.0);
        let mut acc = 0.0;
        for d in &dt {
            acc += d;
            time_frac.push(if total_t > 0.0 { acc / total_t } else { 0.0 });
        }
        if let Some(last) = time_frac.last_mut() {
            *last = 1.0; // guard against rounding
        }
        PaceProfile { time_frac }
    }

    /// Arc length (m) reached at trip-time fraction `frac` (0..1 of elapsed/duration),
    /// inverting the pace. `route` supplies the arc lengths the `time_frac` align to.
    pub fn arc_at(&self, route: &Route, frac: f64) -> f64 {
        let frac = frac.clamp(0.0, 1.0);
        let tf = &self.time_frac;
        if tf.len() < 2 || route.total_m <= 0.0 {
            return frac * route.total_m;
        }
        // First index whose cumulative time fraction is >= frac.
        let j = match tf.binary_search_by(|c| c.partial_cmp(&frac).unwrap_or(std::cmp::Ordering::Equal)) {
            Ok(i) => i.max(1),
            Err(i) => i.clamp(1, tf.len() - 1),
        };
        let (t0, t1) = (tf[j - 1], tf[j]);
        let local = if t1 > t0 { (frac - t0) / (t1 - t0) } else { 0.0 };
        let (a0, a1) = (route.cumulative_m[j - 1], route.cumulative_m[j]);
        a0 + (a1 - a0) * local
    }
}

/// A walkshed (isochrone): the street network reachable within a time budget on
/// foot from a start node.
#[derive(Debug, Clone)]
pub struct Walkshed {
    pub start: u32,
    pub max_seconds: f64,
    /// Reachable node id -> arrival time (seconds).
    pub node_time: HashMap<u32, f64>,
    /// Indices into the graph's edges for edges fully inside the walkshed.
    pub edges: Vec<u32>,
    /// Boundary edges reachable from exactly one endpoint, walkable up to the remaining
    /// time budget from that end: `(edge index, entry node id, walkable fraction of the
    /// edge's length)`. Dropping these truncated every isochrone short of its true rim —
    /// by up to a whole block, and by more where blocks are long — so exposure queries
    /// must sample them (up to the cut) alongside `edges`.
    pub partial: Vec<(u32, u32, f64)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{EdgeData, GraphAsset, NodePoint, Provenance};

    fn prov() -> Provenance {
        Provenance {
            source: "test".into(),
            url: String::new(),
            license: String::new(),
            as_of: "2026-06-14".into(),
            notes: String::new(),
        }
    }

    /// A simple square: 0=(0,0) 1=(10,0) 2=(10,10) 3=(0,10), edges 0-1,1-2,2-3,3-0.
    fn square_graph() -> GraphAsset {
        let nodes = vec![
            NodePoint { x: 0.0, y: 0.0 },
            NodePoint { x: 10.0, y: 0.0 },
            NodePoint { x: 10.0, y: 10.0 },
            NodePoint { x: 0.0, y: 10.0 },
        ];
        let mk = |from: u32, to: u32, a: [f64; 2], b: [f64; 2]| EdgeData {
            from,
            to,
            length_m: ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt(),
            polyline: vec![a, b],
            segment_id: None,
        };
        let edges = vec![
            mk(0, 1, [0.0, 0.0], [10.0, 0.0]),
            mk(1, 2, [10.0, 0.0], [10.0, 10.0]),
            mk(2, 3, [10.0, 10.0], [0.0, 10.0]),
            mk(3, 0, [0.0, 10.0], [0.0, 0.0]),
        ];
        GraphAsset {
            origin: crate::projection::GeoOrigin::MANHATTAN,
            nodes,
            edges,
            provenance: prov(),
        }
    }

    #[test]
    fn routes_shortest_around_square() {
        let g = StreetGraph::from_asset(square_graph());
        // 0 -> 3 directly is 10m via edge 3-0; the long way is 30m.
        let r = g.route(0, 3).unwrap();
        assert!((r.total_m - 10.0).abs() < 1e-6, "got {}", r.total_m);
    }

    #[test]
    fn snap_and_position_over_time() {
        let g = StreetGraph::from_asset(square_graph());
        let r = g.route_points(Vec2::new(0.1, -0.1), Vec2::new(9.9, 0.2)).unwrap();
        assert!((r.total_m - 10.0).abs() < 1e-6);
        // Halfway along, position should be ~ (5,0).
        let mid = r.position_at(5.0);
        assert!(mid.distance(Vec2::new(5.0, 0.0)) < 1e-6, "{mid:?}");
        // Sampling at 1 m/s, dt=1s yields ~11 samples ending on the endpoint.
        let samples = r.sample_over_time(1.0, 1.0);
        assert!(samples.len() >= 11);
        let (t_end, p_end) = *samples.last().unwrap();
        assert!((t_end - 10.0).abs() < 1e-6);
        assert!(p_end.distance(Vec2::new(10.0, 0.0)) < 1e-6);
    }

    #[test]
    fn walkshed_respects_time_budget() {
        // Square, 10 m edges, walk 1 m/s -> each edge = 10 s.
        let g = StreetGraph::from_asset(square_graph());
        // 15 s budget from node 0: reach 0, and neighbors 1 & 3 (10 s); node 2 is 20 s away.
        let ws = g.walkshed(0, 15.0, 1.0);
        assert!(ws.node_time.contains_key(&0));
        assert!(ws.node_time.contains_key(&1));
        assert!(ws.node_time.contains_key(&3));
        assert!(!ws.node_time.contains_key(&2));
        // Reachable edges: 0-1 and 3-0 (both endpoints reachable); not 1-2 or 2-3.
        assert_eq!(ws.edges.len(), 2);
        // Boundary edges 1-2 and 2-3 are each walkable 5 s past their reachable endpoint:
        // (15 - 10) s * 1 m/s / 10 m = fraction 0.5, entered from node 1 resp. node 3.
        assert_eq!(ws.partial.len(), 2);
        for &(ei, entry, frac) in &ws.partial {
            assert!(matches!((ei, entry), (1, 1) | (2, 3)), "unexpected partial ({ei},{entry})");
            assert!((frac - 0.5).abs() < 1e-9, "frac {frac}");
        }

        // A generous budget reaches everything, leaving nothing partial.
        let all = g.walkshed(0, 1000.0, 1.0);
        assert_eq!(all.node_time.len(), 4);
        assert_eq!(all.edges.len(), 4);
        assert!(all.partial.is_empty());
    }

    #[test]
    fn neighbors_lists_incident_edges() {
        let g = StreetGraph::from_asset(square_graph());
        // Node 0 connects to 1 (edge 0) and 3 (edge 3).
        let mut ns: Vec<u32> = g.neighbors(0).iter().map(|&(n, _)| n).collect();
        ns.sort();
        assert_eq!(ns, vec![1, 3]);
    }

    #[test]
    fn random_walk_avoids_immediate_backtrack() {
        use crate::rng::WyRand;
        let g = StreetGraph::from_asset(square_graph());
        let mut rng = WyRand::new(1);
        // From node 1 (neighbors 0 and 2), having come from 0, must go to 2.
        for _ in 0..50 {
            let (next, _) = g.random_walk_step(1, Some(0), &mut rng).unwrap();
            assert_eq!(next, 2, "should not U-turn back to prev when alternative exists");
        }
    }

    #[test]
    fn random_walk_route_is_connected_and_monotone() {
        use crate::rng::WyRand;
        let g = StreetGraph::from_asset(square_graph());
        let mut rng = WyRand::new(99);
        let r = g.random_walk_route(0, 6, &mut rng);
        assert!(r.points.len() >= 2);
        assert!(r.total_m > 0.0);
        // cumulative arc length is non-decreasing.
        for w in r.cumulative_m.windows(2) {
            assert!(w[1] >= w[0]);
        }
    }

    #[test]
    fn heading_points_along_segment() {
        let g = StreetGraph::from_asset(square_graph());
        let r = g.route(0, 1).unwrap(); // along +x from (0,0) to (10,0)
        let h = r.heading_at(5.0);
        assert!((h.x - 1.0).abs() < 1e-6 && h.y.abs() < 1e-6, "{h:?}");
    }

    #[test]
    fn disconnected_returns_no_path() {
        let mut asset = square_graph();
        // Add an isolated node 4 with no edges.
        asset.nodes.push(NodePoint { x: 100.0, y: 100.0 });
        let g = StreetGraph::from_asset(asset);
        assert_eq!(g.route(0, 4), Err(RouteError::NoPath));
    }

    #[test]
    fn pace_profile_endpoints_and_monotonic() {
        // An L-shaped route with a 90° turn at the corner.
        let r = Route::from_points(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 100.0),
        ]);
        let p = PaceProfile::for_route(&r, 11.176, 0.3);
        // time_frac is monotonic, spans 0..1, one per point.
        assert_eq!(p.time_frac.len(), r.points.len());
        assert_eq!(p.time_frac[0], 0.0);
        assert!((p.time_frac.last().unwrap() - 1.0).abs() < 1e-9);
        assert!(p.time_frac.windows(2).all(|w| w[1] >= w[0]));
        // Arc maps endpoints exactly.
        assert!(p.arc_at(&r, 0.0).abs() < 1e-9);
        assert!((p.arc_at(&r, 1.0) - r.total_m).abs() < 1e-6);
        // Monotonic arc in between.
        assert!(p.arc_at(&r, 0.25) < p.arc_at(&r, 0.75));
    }

    #[test]
    fn pace_profile_slows_through_turns() {
        // A pure straight covers exactly half its arc at the half-time mark.
        let straight = Route::from_points(vec![Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0)]);
        let ps = PaceProfile::for_route(&straight, 11.176, 0.3);
        assert!((ps.arc_at(&straight, 0.5) - 100.0).abs() < 1e-6);

        // A route that runs straight, then turns: A→B→C straight, C→D a 90° turn. The
        // vehicle banks distance early cruising the straight, then brakes for the turn,
        // so by the time-midpoint it has covered MORE than half the arc (≠ constant).
        let route = Route::from_points(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(200.0, 0.0),
            Vec2::new(200.0, 200.0),
        ]);
        let p = PaceProfile::for_route(&route, 11.176, 0.3);
        let half_arc = route.total_m * 0.5;
        let mid = p.arc_at(&route, 0.5);
        assert!(mid > half_arc + 1.0, "turn-aware pace should bank the straight early: {mid} vs {half_arc}");
    }

    #[test]
    fn pace_profile_degenerate_is_linear() {
        let r = Route::from_points(vec![Vec2::new(0.0, 0.0)]);
        let p = PaceProfile::for_route(&r, 11.176, 0.3);
        assert_eq!(p.arc_at(&r, 0.5), 0.0); // total_m == 0 → constant fallback
    }
}
