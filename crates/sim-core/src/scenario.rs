//! High-level orchestration: turn baked assets into a placed sensor set and run
//! a route end-to-end into an exposure summary. Used by the app, the batch host,
//! and the headless `route_demo` example.

use std::collections::HashSet;

use crate::assets::{DashcamFieldLayer, FixedSensorLayer, RobotabilityField, TeslaField};
use crate::exposure::{ConfidenceTier, ExposureTally, SourceKind};
use crate::geometry::{captures, FrustumWedge, OccluderEdge};
use crate::graph::{Route, RouteError, StreetGraph, Walkshed};
use crate::math::Vec2;
use crate::mobile::{MobileScenario, RealDayRates};
use crate::simulation::{simulate_full, SensorInstance, SimParams};

/// Default model assumptions for a fixed camera (the source data has only point
/// locations). User-tunable in the app.
#[derive(Debug, Clone, Copy)]
pub struct FixedCameraDefaults {
    pub full_fov_deg: f64,
    pub range_m: f64,
    pub frame_rate: f64,
}

impl Default for FixedCameraDefaults {
    fn default() -> Self {
        FixedCameraDefaults {
            full_fov_deg: 70.0,
            range_m: 15.0,
            frame_rate: 15.0,
        }
    }
}

impl FixedCameraDefaults {
    /// Model assumptions for NYC DOT traffic cameras: PTZ units mounted high at
    /// intersections, covering the roadway from any bearing. Wider reach than a
    /// storefront CCTV, but treated as a *monitoring* class — a live public feed
    /// refreshing roughly once a second, not a high-frame-rate recorder — so its
    /// per-pass frame contribution stays modest. The feed publishes no bearing,
    /// so these are placed omnidirectional (`heading_deg = None`).
    pub fn dot_monitoring() -> Self {
        FixedCameraDefaults {
            full_fov_deg: 360.0,
            range_m: 30.0,
            frame_rate: 1.0,
        }
    }
}

/// Convert a baked fixed-sensor layer into capture-testable instances. The
/// vector index becomes the device id for distinct-device counting.
pub fn sensors_from_layer(layer: &FixedSensorLayer, d: FixedCameraDefaults) -> Vec<SensorInstance> {
    layer
        .sensors
        .iter()
        .enumerate()
        .map(|(i, s)| SensorInstance {
            wedge: FrustumWedge::from_degrees(
                Vec2::new(s.x, s.y),
                s.heading_deg,
                d.full_fov_deg,
                d.range_m,
            ),
            frame_rate: d.frame_rate,
            id: i as u64,
            kind: s.kind,
            // Default: each sensor its own group, confirmed iff it's a surveyed
            // (non-recall-corrected) kind. `group_sensors` overwrites both once the
            // layers are combined and clustered.
            group: i as u32,
            confirmed: !s.kind.recall_corrected(),
        })
        .collect()
}

/// Cluster fixed sensors into physical-camera groups by proximity (≤ `radius_m`),
/// **across sources**, so a camera attested by several layers (e.g. the CCTV census
/// + a DOT survey + an enforcement sign) is one node in the headline. Assigns each
/// sensor a compact `group` id and `confirmed` = whether the group has any surveyed
/// (non-CCTV-census) attestation. Returns the number of distinct groups.
pub fn group_sensors(sensors: &mut [SensorInstance], radius_m: f64) -> usize {
    let n = sensors.len();
    if n == 0 {
        return 0;
    }
    let apex: Vec<Vec2> = sensors.iter().map(|s| s.wedge.apex).collect();
    let surveyed: Vec<bool> = sensors.iter().map(|s| !s.kind.recall_corrected()).collect();
    let kinds: Vec<SourceKind> = sensors.iter().map(|s| s.kind).collect();

    // Union-find with a grid bucket (cell = radius) so only nearby pairs are tested.
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let cell = radius_m.max(1e-6);
    let r2 = radius_m * radius_m;
    let key = |p: Vec2| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = std::collections::HashMap::new();
    for (i, p) in apex.iter().enumerate() {
        grid.entry(key(*p)).or_default().push(i);
    }
    for i in 0..n {
        let (cx, cy) = key(apex[i]);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(bucket) = grid.get(&(cx + dx, cy + dy)) {
                    for &j in bucket {
                        if j <= i {
                            continue;
                        }
                        // Only merge ACROSS sources: a camera the CCTV census and a
                        // DOT/ALPR/enforcement survey both record is one node, but two
                        // distinct same-source detections at one intersection stay two
                        // (each source is already internally de-duplicated).
                        if kinds[i] != kinds[j] {
                            let (ddx, ddy) = (apex[i].x - apex[j].x, apex[i].y - apex[j].y);
                            if ddx * ddx + ddy * ddy <= r2 {
                                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                                if a != b {
                                    parent[a] = b;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Compact group ids; confirmed = any surveyed member.
    let mut group_of: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
    let mut confirmed: Vec<bool> = Vec::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        let g = *group_of.entry(root).or_insert_with(|| {
            confirmed.push(false);
            (confirmed.len() - 1) as u32
        });
        if surveyed[i] {
            confirmed[g as usize] = true;
        }
    }
    for i in 0..n {
        let root = find(&mut parent, i);
        let g = group_of[&root];
        sensors[i].group = g;
        sensors[i].confirmed = confirmed[g as usize];
    }
    group_of.len()
}

/// Result of a one-point walkshed exposure query.
#[derive(Debug, Clone)]
pub struct WalkshedSummary {
    pub max_minutes: f64,
    pub reachable_edges: usize,
    /// Distinct fixed cameras whose coverage touches the walkshed (as detected).
    pub cameras_raw: u32,
    /// Recall-corrected estimate (the headline).
    pub cameras_corrected: f64,
    /// ENU positions of those cameras (for highlighting on the map).
    pub camera_points: Vec<Vec2>,
    /// The source layer of each highlighted camera (aligned 1:1 with
    /// `camera_points`), so the map can style the highlight per layer.
    pub camera_kinds: Vec<SourceKind>,
}

/// Count the distinct fixed cameras that could capture you anywhere within a
/// walkshed (their FOV covers any reachable street point).
pub fn walkshed_exposure(
    graph: &StreetGraph,
    ws: &Walkshed,
    sensors: &[SensorInstance],
    occluders: &[OccluderEdge],
    recall_factor: f64,
) -> WalkshedSummary {
    let edges = &graph.asset().edges;
    let mut seen: HashSet<u64> = HashSet::new();
    let mut camera_points: Vec<Vec2> = Vec::new();
    let mut camera_kinds: Vec<SourceKind> = Vec::new();

    for &ei in &ws.edges {
        let poly = &edges[ei as usize].polyline;
        // Sample first / middle / last vertices of each (short) edge.
        let idxs = [0, poly.len() / 2, poly.len().saturating_sub(1)];
        for &k in &idxs {
            let p = poly[k];
            let pt = Vec2::new(p[0], p[1]);
            for s in sensors {
                if !seen.contains(&s.id) && captures(&s.wedge, pt, occluders) {
                    seen.insert(s.id);
                    camera_points.push(s.wedge.apex);
                    camera_kinds.push(s.kind);
                }
            }
        }
    }

    let raw = seen.len() as u32;
    WalkshedSummary {
        max_minutes: ws.max_seconds / 60.0,
        reachable_edges: ws.edges.len(),
        cameras_raw: raw,
        cameras_corrected: raw as f64 * recall_factor,
        camera_points,
        camera_kinds,
    }
}

/// Per-class exposure for the breakdown panel.
#[derive(Debug, Clone, Copy)]
pub struct SourceBreakdown {
    pub kind: SourceKind,
    pub tier: ConfidenceTier,
    /// Expected distinct devices that captured you (recall-corrected for fixed).
    pub devices: f64,
    /// Poisson probability of at least one capture from this class.
    pub p_at_least_one: f64,
}

/// A compact, display-ready summary of a routed walk's exposure.
#[derive(Debug, Clone)]
pub struct RouteSummary {
    pub route_len_m: f64,
    pub duration_s: f64,
    /// Headline: "~N cameras could have captured you" (recall-corrected).
    pub headline_devices: u32,
    pub total_expected_frames: f64,
    pub fraction_surveilled: f64,
    /// Per-class detail (only classes that contributed any exposure).
    pub breakdown: Vec<SourceBreakdown>,
    pub tally: ExposureTally,
}

/// Route between two ENU points, simulate full exposure (fixed + mobile), and
/// summarize. `departure_hour` (0–24) scales the time-dependent mobile classes.
#[allow(clippy::too_many_arguments)]
pub fn run_route(
    graph: &StreetGraph,
    sensors: &[SensorInstance],
    occluders: &[OccluderEdge],
    mobile: &MobileScenario,
    from: Vec2,
    to: Vec2,
    params: SimParams,
    departure_hour: f64,
    dashcam_field: Option<&DashcamFieldLayer>,
    robot_field: Option<&RobotabilityField>,
    tesla_field: Option<&TeslaField>,
    real: Option<&RealDayRates>,
) -> Result<(Route, RouteSummary), RouteError> {
    let route = graph.route_points(from, to)?;
    let summary = summarize(
        &route, sensors, occluders, mobile, params, departure_hour, dashcam_field, robot_field,
        tesla_field, real,
    );
    Ok((route, summary))
}

/// Simulate exposure for an already-computed route and summarize. Lets the app
/// re-evaluate when scenario sliders / departure hour change without re-routing.
#[allow(clippy::too_many_arguments)]
pub fn summarize(
    route: &Route,
    sensors: &[SensorInstance],
    occluders: &[OccluderEdge],
    mobile: &MobileScenario,
    params: SimParams,
    departure_hour: f64,
    dashcam_field: Option<&DashcamFieldLayer>,
    robot_field: Option<&RobotabilityField>,
    tesla_field: Option<&TeslaField>,
    real: Option<&RealDayRates>,
) -> RouteSummary {
    let tally = simulate_full(
        route, sensors, occluders, mobile, params, departure_hour, dashcam_field, robot_field,
        tesla_field, real,
    );

    let breakdown: Vec<SourceBreakdown> = SourceKind::ALL
        .iter()
        .filter(|&&k| tally.adjusted_devices(k) > 1e-6)
        .map(|&k| SourceBreakdown {
            kind: k,
            tier: k.tier(),
            devices: tally.adjusted_devices(k),
            p_at_least_one: tally.p_capture(k),
        })
        .collect();

    RouteSummary {
        route_len_m: route.total_m,
        duration_s: route.total_m / params.speed_mps,
        headline_devices: tally.headline_device_count(),
        total_expected_frames: tally.total_expected_frames(),
        fraction_surveilled: tally.fraction_surveilled(),
        breakdown,
        tally,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sensor(x: f64, y: f64, kind: SourceKind, id: u64) -> SensorInstance {
        SensorInstance {
            wedge: FrustumWedge::from_degrees(Vec2::new(x, y), None, 360.0, 10.0),
            frame_rate: 1.0,
            id,
            kind,
            group: id as u32,
            confirmed: false,
        }
    }

    #[test]
    fn group_sensors_merges_across_sources_only() {
        // s0+s1+s2 cluster at the origin (CCTV+DOT+CCTV → one node via the DOT bridge);
        // s3 and s4 are two same-source CCTV cameras 3 m apart far away (stay distinct).
        let mut s = vec![
            sensor(0.0, 0.0, SourceKind::FixedCctv, 0),
            sensor(5.0, 0.0, SourceKind::DotLiveView, 1),
            sensor(8.0, 0.0, SourceKind::FixedCctv, 2),
            sensor(100.0, 0.0, SourceKind::FixedCctv, 3),
            sensor(103.0, 0.0, SourceKind::FixedCctv, 4),
        ];
        let n = group_sensors(&mut s, 15.0);
        assert_eq!(n, 3, "{{cctv+dot+cctv}}, {{cctv}}, {{cctv}}");
        assert_eq!(s[0].group, s[1].group);
        assert_eq!(s[1].group, s[2].group);
        assert!(s[0].confirmed, "a surveyed (DOT) attestation confirms the group");
        assert_ne!(s[3].group, s[4].group, "two same-source CCTV stay distinct cameras");
        assert!(!s[3].confirmed && !s[4].confirmed);
    }

    #[test]
    fn walkshed_reports_camera_kinds_aligned_with_points() {
        use crate::assets::{EdgeData, GraphAsset, NodePoint, Provenance};
        use crate::projection::GeoOrigin;
        // A single 50 m street; one ALPR at one end, one DOT cam at the other.
        let asset = GraphAsset {
            origin: GeoOrigin::MANHATTAN,
            nodes: vec![NodePoint { x: 0.0, y: 0.0 }, NodePoint { x: 50.0, y: 0.0 }],
            edges: vec![EdgeData {
                from: 0,
                to: 1,
                length_m: 50.0,
                polyline: vec![[0.0, 0.0], [50.0, 0.0]],
                segment_id: None,
            }],
            provenance: Provenance {
                source: String::new(),
                url: String::new(),
                license: String::new(),
                as_of: String::new(),
                notes: String::new(),
            },
        };
        let graph = StreetGraph::from_asset(asset);
        let ws = graph.walkshed(0, 600.0, 1.34);
        let sensors = vec![
            sensor(0.0, 0.0, SourceKind::Alpr, 0),
            sensor(50.0, 0.0, SourceKind::DotLiveView, 1),
        ];
        let sum = walkshed_exposure(&graph, &ws, &sensors, &[], 1.0);
        // The per-layer highlight depends on kinds staying aligned 1:1 with points.
        assert_eq!(sum.camera_points.len(), sum.camera_kinds.len());
        assert_eq!(sum.cameras_raw, 2, "both cameras cover the street");
        assert!(sum.camera_kinds.contains(&SourceKind::Alpr));
        assert!(sum.camera_kinds.contains(&SourceKind::DotLiveView));
    }
}
