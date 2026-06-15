//! High-level orchestration: turn baked assets into a placed sensor set and run
//! a route end-to-end into an exposure summary. Used by the app, the batch host,
//! and the headless `route_demo` example.

use crate::assets::FixedSensorLayer;
use crate::exposure::{ConfidenceTier, ExposureTally, SourceKind};
use crate::geometry::{FrustumWedge, OccluderEdge};
use crate::graph::{Route, RouteError, StreetGraph};
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
pub fn run_route(
    graph: &StreetGraph,
    sensors: &[SensorInstance],
    occluders: &[OccluderEdge],
    mobile: &MobileScenario,
    from: Vec2,
    to: Vec2,
    params: SimParams,
    departure_hour: f64,
) -> Result<(Route, RouteSummary), RouteError> {
    let route = graph.route_points(from, to)?;
    let summary = summarize(&route, sensors, occluders, mobile, params, departure_hour);
    Ok((route, summary))
}

/// Simulate exposure for an already-computed route and summarize. Lets the app
/// re-evaluate when scenario sliders / departure hour change without re-routing.
pub fn summarize(
    route: &Route,
    sensors: &[SensorInstance],
    occluders: &[OccluderEdge],
    mobile: &MobileScenario,
    params: SimParams,
    departure_hour: f64,
) -> RouteSummary {
    let tally = simulate_full(route, sensors, occluders, mobile, params, departure_hour);

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
