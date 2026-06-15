//! The routable pedestrian graph and A* routing, plus the position-over-time
//! sampling that decouples routing from exposure (route once, then expose).

use crate::assets::GraphAsset;
use crate::math::Vec2;
use crate::projection::GeoOrigin;
use petgraph::algo::astar;
use petgraph::graph::{NodeIndex, UnGraph};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RouteError {
    #[error("no walkable path exists between the chosen points")]
    NoPath,
    #[error("graph is empty")]
    Empty,
}

/// Default pedestrian walking speed (m/s) — ~4.8 km/h.
pub const DEFAULT_WALK_SPEED_MPS: f64 = 1.34;

/// A routable pedestrian graph in local ENU meters.
pub struct StreetGraph {
    origin: GeoOrigin,
    positions: Vec<Vec2>,
    asset: GraphAsset,
    /// Undirected graph; edge weight = index into `asset.edges`.
    g: UnGraph<(), usize>,
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
        StreetGraph {
            origin: asset.origin,
            positions,
            asset,
            g,
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

    /// Nearest graph node to a point (linear scan; fine for one interactive
    /// query over a borough-scale graph).
    pub fn snap_nearest(&self, p: Vec2) -> Option<u32> {
        self.positions
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                a.distance(p)
                    .partial_cmp(&b.distance(p))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i as u32)
    }

    /// Route between two graph node ids using A* with a Euclidean heuristic
    /// (admissible: straight-line distance never overestimates path length).
    pub fn route(&self, start: u32, goal: u32) -> Result<Route, RouteError> {
        if self.positions.is_empty() {
            return Err(RouteError::Empty);
        }
        let start_ix = NodeIndex::new(start as usize);
        let goal_ix = NodeIndex::new(goal as usize);
        let goal_pos = self.positions[goal as usize];

        let result = astar(
            &self.g,
            start_ix,
            |n| n == goal_ix,
            |e| self.asset.edges[*e.weight()].length_m,
            |n| self.positions[n.index()].distance(goal_pos),
        );

        let (_cost, path) = result.ok_or(RouteError::NoPath)?;
        Ok(self.build_route(&path))
    }

    /// Convenience: snap two ENU points to the nearest nodes, validate they are
    /// connected, and route. Mirrors the OSMnx snap+validate workflow.
    pub fn route_points(&self, from: Vec2, to: Vec2) -> Result<Route, RouteError> {
        let a = self.snap_nearest(from).ok_or(RouteError::Empty)?;
        let b = self.snap_nearest(to).ok_or(RouteError::Empty)?;
        self.route(a, b)
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

    /// Sample (elapsed_seconds, position) at a fixed time step `dt` while
    /// walking at `speed` m/s. The final sample lands exactly on the endpoint.
    pub fn sample_over_time(&self, speed: f64, dt: f64) -> Vec<(f64, Vec2)> {
        let mut out = Vec::new();
        if self.points.is_empty() || speed <= 0.0 || dt <= 0.0 {
            return out;
        }
        let duration = self.total_m / speed;
        let mut t = 0.0;
        while t < duration {
            out.push((t, self.position_at(t * speed)));
            t += dt;
        }
        out.push((duration, *self.points.last().unwrap()));
        out
    }
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
    fn disconnected_returns_no_path() {
        let mut asset = square_graph();
        // Add an isolated node 4 with no edges.
        asset.nodes.push(NodePoint { x: 100.0, y: 100.0 });
        let g = StreetGraph::from_asset(asset);
        assert_eq!(g.route(0, 4), Err(RouteError::NoPath));
    }
}
