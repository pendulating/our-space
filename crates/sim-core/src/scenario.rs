//! High-level orchestration: turn baked assets into a placed sensor set and run
//! a route end-to-end into an exposure summary. Used by the app, the batch host,
//! and the headless `route_demo` example.

use std::collections::HashSet;

use crate::assets::{DashcamFieldLayer, FixedSensorLayer};
use crate::exposure::{ConfidenceTier, ExposureTally, SourceKind};
use crate::geometry::{captures, FrustumWedge, OccluderEdge};
use crate::graph::{Route, RouteError, StreetGraph, Walkshed};
use crate::math::Vec2;
use crate::mobile::MobileScenario;
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
        })
        .collect()
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
) -> Result<(Route, RouteSummary), RouteError> {
    let route = graph.route_points(from, to)?;
    let summary = summarize(&route, sensors, occluders, mobile, params, departure_hour, dashcam_field);
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
) -> RouteSummary {
    let tally = simulate_full(route, sensors, occluders, mobile, params, departure_hour, dashcam_field);

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
