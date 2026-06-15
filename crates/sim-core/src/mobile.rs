//! Time-of-day model and mobile/ambient sensing scenario configuration.
//!
//! Mobile classes (ACE buses, dashcams, smart glasses) are modeled as rare
//! independent encounters along the walked route: an intensity λ integrated over
//! route-time gives a Poisson mean of devices that captured you. λ scales with
//! the hour of day (bus headways, traffic, foot traffic). Dashcam and
//! smart-glasses intensities are **scenario assumptions**, not measurements —
//! their configs are user-tunable and labeled speculative in the UI.

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

/// Dashcam field exposure (Tier C: modeled field × assumed penetration).
#[derive(Debug, Clone, Copy)]
pub struct DashcamConfig {
    /// Fraction of passing vehicles with a forward-facing dashcam.
    pub penetration: f64,
    /// Passing vehicles per minute on a typical street at peak traffic.
    pub vehicles_per_min_peak: f64,
    /// P(walker falls in a passing vehicle's dashcam FOV).
    pub capture_prob: f64,
    /// Image frames per passing dashcam.
    pub frames_per_pass: f64,
}

impl Default for DashcamConfig {
    fn default() -> Self {
        // Urban ownership ~40% (single non-peer-reviewed survey; tunable).
        DashcamConfig {
            penetration: 0.40,
            vehicles_per_min_peak: 30.0,
            capture_prob: 0.5,
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

/// The set of enabled mobile/ambient classes. `None` = class disabled.
#[derive(Debug, Clone, Default)]
pub struct MobileScenario {
    pub ace: Option<AceConfig>,
    pub dashcam: Option<DashcamConfig>,
    pub glasses: Option<GlassesConfig>,
}

impl MobileScenario {
    /// Dashcam + smart-glasses on at defaults; ACE off (needs baked corridors).
    pub fn fields_only() -> Self {
        MobileScenario {
            ace: None,
            dashcam: Some(DashcamConfig::default()),
            glasses: Some(GlassesConfig::default()),
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
}
