//! Time-of-day model and mobile/ambient sensing scenario configuration.
//!
//! Mobile classes (ACE buses, dashcams, smart glasses) are modeled as rare
//! independent encounters along the walked route: an intensity λ integrated over
//! route-time gives a Poisson mean of devices that captured you. λ scales with
//! the hour of day (bus headways, traffic, foot traffic). Dashcam and
//! smart-glasses intensities are **scenario assumptions**, not measurements —
//! their configs are user-tunable and labeled speculative in the UI.

use crate::assets::{BusDayLayer, TaxiDayLayer};
use crate::math::Vec2;

/// Bus headway in minutes (between buses, one direction) as a function of hour.
pub fn bus_headway_minutes(hour: f64) -> f64 {
    let h = hour.rem_euclid(24.0);
    if (7.0..9.0).contains(&h) || (16.0..19.0).contains(&h) {
        5.0 // rush
    } else if (6.0..22.0).contains(&h) {
        10.0 // daytime
    } else {
        25.0 // overnight
    }
}

/// Relative vehicle-traffic intensity by hour (≈1.0 at rush, low overnight).
pub fn traffic_multiplier(hour: f64) -> f64 {
    diurnal_two_peak(hour, 0.12, 8.5, 17.5)
}

/// Relative pedestrian-flow intensity by hour.
pub fn pedestrian_multiplier(hour: f64) -> f64 {
    diurnal_two_peak(hour, 0.10, 9.0, 18.0)
}

/// Relative sidewalk-delivery-robot activity by hour: food/grocery delivery peaks
/// at lunch and dinner, near-zero overnight (speculative operating profile).
pub fn robot_activity_multiplier(hour: f64) -> f64 {
    diurnal_two_peak(hour, 0.05, 12.0, 18.5)
}

/// Real per-minute mobile service levels for one baked day, replacing the synthetic
/// diurnal curves so the headline tracks the actual MTA ACE timetable + TLC trip
/// volume. Each array is indexed by minute-of-day (`0..1440`); empty → fall back to
/// the synthetic curve. Built by the app from the baked `BusDayLayer`/`TaxiDayLayer`.
#[derive(Debug, Clone, Default)]
pub struct RealDayRates {
    /// Effective ACE bus headway (minutes) per minute-of-day, from the real schedule.
    pub ace_headway_min: Vec<f64>,
    /// Rideshare volume multiplier per minute-of-day, normalized to the day's peak
    /// (real TLC pickups) — the temporal factor on local dashcam density.
    pub taxi_traffic_mult: Vec<f64>,
}

/// Minutes in a day; the per-minute arrays are this long.
const DAY_MIN: usize = 1440;
/// Typical midday ACE headway (min) that anchors the real-headway magnitude; the
/// real schedule sets only the *shape* (service now ÷ typical service).
const BASE_HEADWAY_MIN: f64 = 8.0;

impl RealDayRates {
    /// Derive the real per-minute service arrays from one baked day: the effective
    /// ACE headway (from the GTFS trip-activity histogram, anchored to a typical-
    /// service baseline) and the rideshare volume multiplier (real TLC pickups per
    /// minute, normalized to the day's peak). Empty arrays (no schedule / no trips)
    /// make the accessors fall back to the synthetic diurnal curves.
    pub fn from_day(bus: &BusDayLayer, taxi: &TaxiDayLayer) -> Self {
        // ACE: buses active per minute-of-day (wrapping after-midnight trips).
        let mut active = vec![0.0f64; DAY_MIN];
        for t in &bus.trips {
            let s = t.start_min.floor() as i64;
            let e = t.end_min.ceil() as i64;
            for mm in s..e.max(s + 1) {
                active[mm.rem_euclid(DAY_MIN as i64) as usize] += 1.0;
            }
        }
        let running: Vec<f64> = active.iter().copied().filter(|&a| a > 0.0).collect();
        let mean = if running.is_empty() {
            0.0
        } else {
            running.iter().sum::<f64>() / running.len() as f64
        };
        let ace_headway_min: Vec<f64> = if mean <= 0.0 {
            Vec::new()
        } else {
            active
                .iter()
                .map(|&a| if a <= 0.0 { 60.0 } else { (BASE_HEADWAY_MIN * mean / a).clamp(3.0, 60.0) })
                .collect()
        };

        // Taxis: real pickups per minute-of-day, normalized to the day's peak.
        let mut vol = vec![0.0f64; DAY_MIN];
        for od in &taxi.od_per_minute {
            vol[od.pu_min as usize % DAY_MIN] += od.trips as f64;
        }
        let peak = vol.iter().copied().fold(0.0, f64::max);
        let taxi_traffic_mult: Vec<f64> =
            if peak <= 0.0 { Vec::new() } else { vol.iter().map(|&v| v / peak).collect() };

        RealDayRates { ace_headway_min, taxi_traffic_mult }
    }

    fn sample(arr: &[f64], hour: f64) -> Option<f64> {
        if arr.is_empty() {
            return None;
        }
        let m = (hour.rem_euclid(24.0) * 60.0) as usize % arr.len();
        Some(arr[m])
    }
    /// Real ACE headway (min) at `hour`, else the synthetic curve.
    pub fn ace_headway_at(&self, hour: f64) -> f64 {
        Self::sample(&self.ace_headway_min, hour).unwrap_or_else(|| bus_headway_minutes(hour))
    }
    /// Real rideshare traffic multiplier at `hour`, else the synthetic curve.
    pub fn taxi_mult_at(&self, hour: f64) -> f64 {
        Self::sample(&self.taxi_traffic_mult, hour).unwrap_or_else(|| traffic_multiplier(hour))
    }
}

/// A smooth two-peak (AM/PM) diurnal curve in roughly `[floor, 1.0]`.
fn diurnal_two_peak(hour: f64, floor: f64, am_peak: f64, pm_peak: f64) -> f64 {
    let h = hour.rem_euclid(24.0);
    let am = (-((h - am_peak) / 2.6).powi(2)).exp();
    let pm = (-((h - pm_peak) / 2.8).powi(2)).exp();
    let midday = 0.55 * (-((h - 13.0) / 4.0).powi(2)).exp();
    let day = am.max(pm).max(midday);
    floor + (1.0 - floor) * day
}

/// ACE bus corridor exposure (Tier A: mapped public infrastructure).
#[derive(Debug, Clone)]
pub struct AceConfig {
    /// Corridor line segments in ENU meters (the routes ACE buses traverse).
    pub segments: Vec<[Vec2; 2]>,
    /// How close (m) the walker must be to a corridor to be in a bus camera's
    /// curb-side field of view as it passes.
    pub capture_range_m: f64,
    /// Passes per headway interval (≈2 for both directions).
    pub directions: f64,
    /// Image frames captured per passing bus.
    pub frames_per_pass: f64,
    /// Multiplier on headway (≥1 stretches headways, e.g. modeling fewer routes
    /// enrolled in an earlier coverage year).
    pub headway_scale: f64,
}

impl AceConfig {
    pub fn new(segments: Vec<[Vec2; 2]>) -> Self {
        AceConfig {
            segments,
            capture_range_m: 20.0,
            directions: 2.0,
            frames_per_pass: 30.0,
            headway_scale: 1.0,
        }
    }
}

/// Rideshare-camera (dashcam) exposure. NYC requires for-hire vehicles to carry
/// in-vehicle cameras, so this models the cameras riding in Uber/Lyft vehicles;
/// the spatial density comes from real TLC High-Volume FHV trip records (a
/// [`crate::assets::DashcamFieldLayer`]), scaled by the penetration assumption.
#[derive(Debug, Clone, Copy)]
pub struct DashcamConfig {
    /// Fraction of passing rideshare vehicles whose camera captures the street.
    pub penetration: f64,
    /// Baseline passing rideshare vehicles/min in a *typical-density* zone at peak.
    pub vehicles_per_min_peak: f64,
    /// P(walker falls in a passing vehicle's dashcam FOV).
    pub capture_prob: f64,
    /// Image frames per passing dashcam.
    pub frames_per_pass: f64,
}

impl Default for DashcamConfig {
    fn default() -> Self {
        // Baseline is for a MEDIAN-density taxi zone at peak (the field scales it
        // by local rideshare density, up to ~8× in Midtown). ~40% camera fitting.
        DashcamConfig {
            penetration: 0.40,
            vehicles_per_min_peak: 12.0,
            capture_prob: 0.40,
            frames_per_pass: 5.0,
        }
    }
}

/// Smart-glasses ambient exposure (Tier D: speculative / emerging).
#[derive(Debug, Clone, Copy)]
pub struct GlassesConfig {
    /// Wearers per 1000 pedestrians (no empirical NYC basis — a scenario knob).
    pub per_1000_pedestrians: f64,
    /// Fraction of wearers actively recording (fundamentally unknowable).
    pub p_recording: f64,
    /// Passing pedestrians per minute on a typical sidewalk at peak.
    pub peds_per_min_peak: f64,
    /// P(walker falls in a passing wearer's camera FOV).
    pub capture_prob: f64,
    pub frames_per_pass: f64,
}

impl Default for GlassesConfig {
    fn default() -> Self {
        GlassesConfig {
            per_1000_pedestrians: 10.0,
            p_recording: 0.05,
            peds_per_min_peak: 60.0,
            capture_prob: 0.4,
            frames_per_pass: 3.0,
        }
    }
}

/// Sidewalk delivery-robot exposure (Tier D: speculative). Robots carry always-on
/// navigation cameras; density follows the Robotability Score field (passed
/// separately), so this config holds only the rate/capture assumptions.
#[derive(Debug, Clone, Copy)]
pub struct RobotConfig {
    /// Robots passing per minute in a *top-robotability* spot at peak activity
    /// (the Robotability field, 0..1, scales this down spatially). Speculative.
    pub robots_per_min_peak: f64,
    /// P(walker falls in a passing robot's navigation-camera FOV).
    pub capture_prob: f64,
    pub frames_per_pass: f64,
}

impl Default for RobotConfig {
    fn default() -> Self {
        RobotConfig {
            robots_per_min_peak: 2.0,
            capture_prob: 0.5,
            frames_per_pass: 4.0,
        }
    }
}

/// Tesla-camera exposure (Tier C: real cameras, assumed capture). Teslas run
/// always-on cameras (Sentry when parked + Autopilot while driving); density
/// follows the Tesla field (private DMV registrations by ZIP), passed separately.
#[derive(Debug, Clone, Copy)]
pub struct TeslaConfig {
    /// Tesla-camera "passes" per minute in a typical-density ZIP (the Tesla field,
    /// ~1.0 typical, scales this spatially). Always-on, so no diurnal term.
    pub teslas_per_min_peak: f64,
    /// P(walker falls in a passing/parked Tesla camera's FOV).
    pub capture_prob: f64,
    pub frames_per_pass: f64,
}

impl Default for TeslaConfig {
    fn default() -> Self {
        TeslaConfig {
            teslas_per_min_peak: 4.0,
            capture_prob: 0.3,
            frames_per_pass: 8.0,
        }
    }
}

/// The set of enabled mobile/ambient classes. `None` = class disabled.
#[derive(Debug, Clone, Default)]
pub struct MobileScenario {
    pub ace: Option<AceConfig>,
    pub dashcam: Option<DashcamConfig>,
    pub glasses: Option<GlassesConfig>,
    pub robots: Option<RobotConfig>,
    pub tesla: Option<TeslaConfig>,
}

impl MobileScenario {
    /// Dashcam + smart-glasses on at defaults; ACE off (needs baked corridors).
    pub fn fields_only() -> Self {
        MobileScenario {
            ace: None,
            dashcam: Some(DashcamConfig::default()),
            glasses: Some(GlassesConfig::default()),
            robots: None,
            tesla: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headway_shorter_at_rush() {
        assert!(bus_headway_minutes(8.0) < bus_headway_minutes(14.0));
        assert!(bus_headway_minutes(14.0) < bus_headway_minutes(3.0));
    }

    #[test]
    fn diurnal_peaks_above_overnight() {
        assert!(traffic_multiplier(8.5) > traffic_multiplier(3.0));
        assert!(pedestrian_multiplier(18.0) > pedestrian_multiplier(4.0));
        // bounded in [floor, 1]
        for h in 0..24 {
            let m = traffic_multiplier(h as f64);
            assert!((0.10..=1.0001).contains(&m), "hour {h} -> {m}");
        }
    }

    #[test]
    fn from_day_builds_real_curves() {
        use crate::assets::{BusDayLayer, BusTrip, Provenance, TaxiDayLayer, TaxiOdMinute};
        use crate::projection::GeoOrigin;
        let prov = || Provenance {
            source: String::new(),
            url: String::new(),
            license: String::new(),
            as_of: String::new(),
            notes: String::new(),
        };
        // Buses: heavy service around 08:00 (60 trips), light at 03:00 (2 trips).
        let trip = |h: f32| BusTrip {
            route_idx: 0,
            shape_idx: 0,
            start_min: h * 60.0,
            end_min: h * 60.0 + 30.0,
            keyframes: vec![],
        };
        let mut trips = vec![trip(8.0); 60];
        trips.extend(vec![trip(3.0); 2]);
        let bus = BusDayLayer {
            origin: GeoOrigin::MANHATTAN,
            service_date: 0,
            routes: vec![],
            shapes: vec![],
            trips,
            provenance: prov(),
        };
        // Taxis: peak pickups at 09:00, few at 04:00.
        let od = vec![
            TaxiOdMinute { pu_min: 9 * 60, pu_zone: 1, do_zone: 2, trips: 500 },
            TaxiOdMinute { pu_min: 4 * 60, pu_zone: 1, do_zone: 2, trips: 10 },
        ];
        let taxi = TaxiDayLayer {
            origin: GeoOrigin::MANHATTAN,
            service_date: 0,
            routes: vec![],
            trips: vec![],
            od_per_minute: od,
            provenance: prov(),
            sensing: Default::default(),
        };

        let r = RealDayRates::from_day(&bus, &taxi);
        // Real ACE headway is shorter when more buses are active (08:00 < 03:00).
        assert!(
            r.ace_headway_at(8.0) < r.ace_headway_at(3.0),
            "8h={} 3h={}",
            r.ace_headway_at(8.0),
            r.ace_headway_at(3.0)
        );
        // Real taxi multiplier peaks (=1.0) at the busiest minute, low at the trough.
        assert!((r.taxi_mult_at(9.0) - 1.0).abs() < 1e-9);
        assert!(r.taxi_mult_at(9.0) > r.taxi_mult_at(4.0));
    }
}
