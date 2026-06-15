//! The render-agnostic simulation loop: walk a route on a discrete clock and
//! accumulate exposure. This is the shared analytical core called by both the
//! interactive app and the headless batch.
//!
//! Routing is decoupled from exposure: the caller has already computed the
//! [`Route`]; here we sample position(t) at constant speed and, at each tick,
//! test the walker against every sensor's capture geometry.

use crate::exposure::{ExposureTally, SourceKind};
use crate::geometry::{captures, FrustumWedge, OccluderEdge};
use crate::graph::Route;
use crate::math::Vec2;

/// A placed fixed sensor ready for capture testing.
#[derive(Debug, Clone, Copy)]
pub struct SensorInstance {
    pub wedge: FrustumWedge,
    /// Capture rate in frames/sec (or generic "captures per second of view").
    pub frame_rate: f64,
    /// Stable id for distinct-device counting.
    pub id: u64,
    pub kind: SourceKind,
}

/// Parameters for a simulation run.
#[derive(Debug, Clone, Copy)]
pub struct SimParams {
    pub speed_mps: f64,
    pub dt: f64,
    /// Multiplier applied to fixed distinct-device counts in the headline
    /// (e.g. 1/0.63 to undo Dahir detector recall). 1.0 = no correction.
    pub recall_factor: f64,
}

impl Default for SimParams {
    fn default() -> Self {
        SimParams {
            speed_mps: crate::graph::DEFAULT_WALK_SPEED_MPS,
            dt: 1.0,
            recall_factor: 1.0,
        }
    }
}

/// Simulate fixed-sensor exposure along a route.
///
/// `occluders` should be pre-filtered to walls near the route (e.g. via an
/// R-tree) by the caller; passing the full set is fine for small scenes.
pub fn simulate_fixed(
    route: &Route,
    sensors: &[SensorInstance],
    occluders: &[OccluderEdge],
    params: SimParams,
) -> ExposureTally {
    let mut tally = ExposureTally::new();
    tally.recall_factor = params.recall_factor;

    let samples = route.sample_over_time(params.speed_mps, params.dt);
    let mut prev: Option<Vec2> = None;

    for (i, (_t, pos)) in samples.iter().enumerate() {
        // Weight the final (possibly partial) tick by its true duration so the
        // capture-event integral isn't over-counted at the route end.
        let tick_dt = if i + 1 == samples.len() && samples.len() >= 2 {
            (samples[i].0 - samples[i - 1].0).min(params.dt)
        } else {
            params.dt
        };

        let mut covered_here = false;
        for s in sensors {
            if captures(&s.wedge, *pos, occluders) {
                tally.record_fixed_capture(s.kind, s.id, s.frame_rate, tick_dt);
                covered_here = true;
            }
        }

        if let Some(p) = prev {
            tally.record_progress(p.distance(*pos), covered_here);
        }
        prev = Some(*pos);
    }

    tally
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exposure::DAHIR_RECALL;
    use crate::geometry::OccluderEdge;
    use crate::graph::Route;

    fn straight_route() -> Route {
        // 0..20 m along +x.
        Route::from_points(vec![Vec2::new(0.0, 0.0), Vec2::new(20.0, 0.0)])
    }

    fn cam_at(x: f64, y: f64, heading_deg: f64) -> SensorInstance {
        SensorInstance {
            wedge: FrustumWedge::from_degrees(Vec2::new(x, y), Some(heading_deg), 90.0, 10.0),
            frame_rate: 15.0,
            id: 1,
            kind: SourceKind::FixedCctv,
        }
    }

    #[test]
    fn camera_beside_route_captures_a_stretch() {
        let route = straight_route();
        // Camera 5 m north of the path at x=10, looking south (heading 180).
        let cam = cam_at(10.0, 5.0, 180.0);
        let t = simulate_fixed(&route, &[cam], &[], SimParams::default());
        let s = t.source(SourceKind::FixedCctv);
        assert_eq!(s.distinct_devices, 1);
        assert!(s.expected_captures > 0.0);
        // Some but not all of the 20 m walk is under coverage.
        let f = t.fraction_surveilled();
        assert!(f > 0.0 && f < 1.0, "fraction was {f}");
    }

    #[test]
    fn occluding_wall_eliminates_capture() {
        let route = straight_route();
        let cam = cam_at(10.0, 5.0, 180.0);
        // A wall flush along y=2.5 between the camera (y=5) and the path (y=0).
        let wall = [OccluderEdge {
            a: Vec2::new(0.0, 2.5),
            b: Vec2::new(20.0, 2.5),
        }];
        let t = simulate_fixed(&route, &[cam], &wall, SimParams::default());
        let s = t.source(SourceKind::FixedCctv);
        assert_eq!(s.distinct_devices, 0);
        assert_eq!(s.expected_captures, 0.0);
        assert_eq!(t.fraction_surveilled(), 0.0);
    }

    #[test]
    fn recall_correction_flows_into_headline() {
        let route = straight_route();
        // Two cameras covering the route from opposite sides.
        let cams = [
            SensorInstance { id: 1, ..cam_at(7.0, 5.0, 180.0) },
            SensorInstance { id: 2, ..cam_at(13.0, -5.0, 0.0) },
        ];
        let params = SimParams { recall_factor: 1.0 / DAHIR_RECALL, ..SimParams::default() };
        let t = simulate_fixed(&route, &cams, &[], params);
        assert_eq!(t.source(SourceKind::FixedCctv).distinct_devices, 2);
        // 2 detected -> ~3 with recall correction (2 * 1.587 = 3.17 -> 3).
        assert_eq!(t.headline_device_count(), 3);
    }
}
