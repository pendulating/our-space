//! High-level orchestration: turn baked assets into a placed sensor set and run
//! a route end-to-end into an exposure summary. Used by the app, the batch host,
//! and the headless `route_demo` example.

use crate::assets::FixedSensorLayer;
use crate::exposure::ExposureTally;
use crate::geometry::{FrustumWedge, OccluderEdge};
use crate::graph::{Route, RouteError, StreetGraph};
use crate::math::Vec2;
use crate::simulation::{simulate_fixed, SensorInstance, SimParams};

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

/// A compact, display-ready summary of a routed walk's exposure.
#[derive(Debug, Clone)]
pub struct RouteSummary {
    pub route_len_m: f64,
    pub duration_s: f64,
    /// Headline: "~N cameras could have captured you" (recall-corrected).
    pub headline_devices: u32,
    pub total_expected_captures: f64,
    pub fraction_surveilled: f64,
    pub tally: ExposureTally,
}

/// Route between two ENU points, simulate fixed-sensor exposure, and summarize.
pub fn run_route(
    graph: &StreetGraph,
    sensors: &[SensorInstance],
    occluders: &[OccluderEdge],
    from: Vec2,
    to: Vec2,
    params: SimParams,
) -> Result<(Route, RouteSummary), RouteError> {
    let route = graph.route_points(from, to)?;
    let tally = simulate_fixed(&route, sensors, occluders, params);
    let summary = RouteSummary {
        route_len_m: route.total_m,
        duration_s: route.total_m / params.speed_mps,
        headline_devices: tally.headline_device_count(),
        total_expected_captures: tally.total_expected_captures(),
        fraction_surveilled: tally.fraction_surveilled(),
        tally,
    };
    Ok((route, summary))
}
