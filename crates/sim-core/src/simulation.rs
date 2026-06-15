//! The render-agnostic simulation loop: walk a route on a discrete clock and
//! accumulate exposure. This is the shared analytical core called by both the
//! interactive app and the headless batch.
//!
//! Routing is decoupled from exposure: the caller has already computed the
//! [`Route`]; here we sample position(t) at constant speed and, at each tick,
//! test the walker against every sensor's capture geometry.

use crate::assets::DashcamFieldLayer;
use crate::exposure::{ExposureTally, SourceKind};
use crate::geometry::{captures, FrustumWedge, OccluderEdge};
use crate::graph::Route;
use crate::math::{point_segment_distance, Vec2};
use crate::mobile::{bus_headway_minutes, pedestrian_multiplier, traffic_multiplier, MobileScenario};

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

/// Simulate fixed-sensor exposure only (mobile classes disabled).
///
/// `occluders` should be pre-filtered to walls near the route (e.g. via an
/// R-tree) by the caller; passing the full set is fine for small scenes.
pub fn simulate_fixed(
    route: &Route,
    sensors: &[SensorInstance],
    occluders: &[OccluderEdge],
    params: SimParams,
) -> ExposureTally {
    simulate_full(route, sensors, occluders, &MobileScenario::default(), params, 12.0, None)
}

/// Full exposure simulation: fixed cameras + the enabled mobile/ambient classes,
/// evaluated against the walker's position **and** the clock time at each tick.
///
/// Routing is already done; here we sample position(t) at constant speed and, at
/// each tick, accumulate fixed captures (in-FOV + line-of-sight) and mobile
/// encounter intensities (ACE corridor proximity, dashcam/glasses fields), with
/// the hour-of-day scaling bus headways, traffic, and foot traffic.
#[allow(clippy::too_many_arguments)]
pub fn simulate_full(
    route: &Route,
    fixed: &[SensorInstance],
    occluders: &[OccluderEdge],
    mobile: &MobileScenario,
    params: SimParams,
    departure_hour: f64,
    dashcam_field: Option<&DashcamFieldLayer>,
) -> ExposureTally {
    let mut tally = ExposureTally::new();
    tally.recall_factor = params.recall_factor;

    let samples = route.sample_over_time(params.speed_mps, params.dt);
    let mut prev: Option<Vec2> = None;

    for (i, (t_elapsed, pos)) in samples.iter().enumerate() {
        // Weight the final (possibly partial) tick by its true duration so the
        // capture integral isn't over-counted at the route end.
        let tick_dt = if i + 1 == samples.len() && samples.len() >= 2 {
            (samples[i].0 - samples[i - 1].0).min(params.dt)
        } else {
            params.dt
        };
        let hour = departure_hour + t_elapsed / 3600.0;

        // --- fixed cameras ---
        let mut covered_here = false;
        for s in fixed {
            if captures(&s.wedge, *pos, occluders) {
                tally.record_fixed_capture(s.kind, s.id, s.frame_rate, tick_dt);
                covered_here = true;
            }
        }

        // --- ACE buses: encounter only while within a corridor's curb reach ---
        if let Some(ace) = &mobile.ace {
            let nearest = ace
                .segments
                .iter()
                .map(|[a, b]| point_segment_distance(*pos, *a, *b))
                .fold(f64::INFINITY, f64::min);
            if nearest <= ace.capture_range_m {
                let headway_s = (bus_headway_minutes(hour) * 60.0 * ace.headway_scale).max(1.0);
                let encounters = (ace.directions / headway_s) * tick_dt;
                tally.record_mobile(SourceKind::AceBus, encounters, encounters * ace.frames_per_pass);
            }
        }

        // --- dashcams (rideshare cameras): rate follows local rideshare density
        //     (real TLC trip field) × diurnal traffic ---
        if let Some(d) = &mobile.dashcam {
            let zone = dashcam_field.map_or(1.0, |f| f.intensity_at(*pos));
            let veh_per_s = (d.vehicles_per_min_peak / 60.0) * traffic_multiplier(hour) * zone;
            let encounters = veh_per_s * d.penetration * d.capture_prob * tick_dt;
            tally.record_mobile(SourceKind::Dashcam, encounters, encounters * d.frames_per_pass);
        }

        // --- smart glasses: ambient field, scaled by diurnal foot traffic ---
        if let Some(g) = &mobile.glasses {
            let peds_per_s = (g.peds_per_min_peak / 60.0) * pedestrian_multiplier(hour);
            let encounters =
                peds_per_s * (g.per_1000_pedestrians / 1000.0) * g.p_recording * g.capture_prob * tick_dt;
            tally.record_mobile(SourceKind::SmartGlasses, encounters, encounters * g.frames_per_pass);
        }

        if let Some(p) = prev {
            tally.record_progress(p.distance(*pos), covered_here);
        }
        prev = Some(*pos);
    }

    tally
}

/// Per-class expected devices that would capture you **per minute of presence**
/// at a point (the citywide-heatmap intensity, kept per class so a uniform field
/// like dashcams doesn't wash out the spatial signal of fixed cameras / ACE).
#[derive(Debug, Clone, Copy, Default)]
pub struct ExposureRates {
    pub fixed: f64,
    pub ace: f64,
    pub dashcam: f64,
    pub glasses: f64,
}

impl ExposureRates {
    pub fn total(&self) -> f64 {
        self.fixed + self.ace + self.dashcam + self.glasses
    }
}

/// Per-minute exposure rate at a point, at `hour`, split by class.
///
/// `nearby_fixed` should already be spatially culled to candidate cameras by the
/// caller (e.g. an R-tree query); `near_ace` indicates the point is within an
/// ACE corridor's capture range. Fixed cameras covering the point contribute one
/// device each (recall-corrected); mobile classes contribute their per-minute
/// encounter rate at the given hour.
#[allow(clippy::too_many_arguments)]
pub fn exposure_rates_per_minute(
    point: Vec2,
    hour: f64,
    nearby_fixed: &[SensorInstance],
    occluders: &[OccluderEdge],
    near_ace: bool,
    mobile: &MobileScenario,
    recall_factor: f64,
    dashcam_field: Option<&DashcamFieldLayer>,
) -> ExposureRates {
    let mut r = ExposureRates::default();

    r.fixed = nearby_fixed
        .iter()
        .filter(|s| captures(&s.wedge, point, occluders))
        .count() as f64
        * recall_factor;

    if near_ace {
        if let Some(ace) = &mobile.ace {
            let headway_s = (bus_headway_minutes(hour) * 60.0 * ace.headway_scale).max(1.0);
            r.ace = (ace.directions / headway_s) * 60.0;
        }
    }
    if let Some(d) = &mobile.dashcam {
        let zone = dashcam_field.map_or(1.0, |f| f.intensity_at(point));
        let veh_per_s = (d.vehicles_per_min_peak / 60.0) * traffic_multiplier(hour) * zone;
        r.dashcam = veh_per_s * d.penetration * d.capture_prob * 60.0;
    }
    if let Some(g) = &mobile.glasses {
        let peds_per_s = (g.peds_per_min_peak / 60.0) * pedestrian_multiplier(hour);
        r.glasses = peds_per_s * (g.per_1000_pedestrians / 1000.0) * g.p_recording * g.capture_prob * 60.0;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exposure::DAHIR_RECALL;
    use crate::geometry::OccluderEdge;
    use crate::graph::Route;
    use crate::mobile::{AceConfig, DashcamConfig, MobileScenario};

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
        assert_eq!(s.devices, 1.0);
        assert!(s.frames > 0.0);
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
        assert_eq!(s.devices, 0.0);
        assert_eq!(s.frames, 0.0);
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
        assert_eq!(t.source(SourceKind::FixedCctv).devices, 2.0);
        // 2 detected -> ~3 with recall correction (2 * 1.587 = 3.17 -> 3).
        assert_eq!(t.headline_device_count(), 3);
    }

    #[test]
    fn ace_corridor_encounters_scale_with_headway() {
        let route = straight_route();
        // Corridor coincident with the walked route.
        let ace = AceConfig::new(vec![[Vec2::new(0.0, 0.0), Vec2::new(20.0, 0.0)]]);
        let mobile = MobileScenario { ace: Some(ace), dashcam: None, glasses: None };
        let dev = |hour| {
            simulate_full(&route, &[], &[], &mobile, SimParams::default(), hour, None)
                .source(SourceKind::AceBus)
                .devices
        };
        let (rush, midday, night) = (dev(8.0), dev(14.0), dev(3.0));
        assert!(rush > 0.0);
        assert!(rush > midday, "rush {rush} should exceed midday {midday}");
        assert!(midday > night, "midday {midday} should exceed night {night}");
    }

    #[test]
    fn ace_off_corridor_no_encounters() {
        let route = straight_route();
        // Corridor 100 m away — beyond the 20 m capture range.
        let ace = AceConfig::new(vec![[Vec2::new(0.0, 100.0), Vec2::new(20.0, 100.0)]]);
        let mobile = MobileScenario { ace: Some(ace), dashcam: None, glasses: None };
        let t = simulate_full(&route, &[], &[], &mobile, SimParams::default(), 8.0, None);
        assert_eq!(t.source(SourceKind::AceBus).devices, 0.0);
    }

    #[test]
    fn dashcam_field_scales_with_traffic() {
        let route = straight_route();
        let mobile = MobileScenario {
            ace: None,
            dashcam: Some(DashcamConfig::default()),
            glasses: None,
        };
        let peak = simulate_full(&route, &[], &[], &mobile, SimParams::default(), 8.5, None)
            .source(SourceKind::Dashcam)
            .devices;
        let night = simulate_full(&route, &[], &[], &mobile, SimParams::default(), 3.0, None)
            .source(SourceKind::Dashcam)
            .devices;
        assert!(night > 0.0);
        assert!(peak > night, "peak {peak} should exceed night {night}");
    }

    #[test]
    fn per_minute_rate_adds_classes() {
        // Camera at (0,5) looking south covers the origin (5 m away, in range).
        let cam = cam_at(0.0, 5.0, 180.0);
        let fixed_only =
            exposure_rates_per_minute(Vec2::ZERO, 12.0, &[cam], &[], false, &MobileScenario::default(), 1.0, None);
        assert!((fixed_only.fixed - 1.0).abs() < 1e-9, "one covering camera, got {}", fixed_only.fixed);
        assert_eq!(fixed_only.ace, 0.0);

        let mobile = MobileScenario {
            ace: Some(AceConfig::new(vec![])),
            dashcam: Some(DashcamConfig::default()),
            glasses: None,
        };
        let with_mobile =
            exposure_rates_per_minute(Vec2::ZERO, 8.0, &[cam], &[], true, &mobile, 1.0, None);
        assert!(with_mobile.ace > 0.0 && with_mobile.dashcam > 0.0);
        assert!(with_mobile.total() > fixed_only.total());
    }
}
