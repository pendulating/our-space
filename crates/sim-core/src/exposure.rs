//! The exposure model.
//!
//! Headline (per the product decision) is **"cameras that saw you"** — the
//! expected number of distinct devices whose coverage the route entered, summed
//! across all sensing classes. Each class also tracks expected image **frames**
//! (rich for continuous fixed cameras) and a Poisson **P(≥1 capture)**.
//!
//! - Fixed cameras: distinct device ids seen (recall-corrected) + `frame_rate ×
//!   dwell` frames.
//! - Mobile/ambient sources (ACE buses, dashcams, smart glasses): rare
//!   independent encounters, so the per-class device count is the Poisson mean
//!   of encounters accumulated along the route, and `P(≥1) = 1 − e^(−mean)`.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The detection recall of the Dahir et al. street-view camera detector
/// (~63%). Observed fixed-camera density underestimates the truth by this
/// factor; divide by it for an unbiased estimate. Surfaced as an uncertainty
/// band in the UI, never as a silent correction.
pub const DAHIR_RECALL: f64 = 0.63;

/// Confidence tier for a sensing layer, governing UI treatment and honesty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfidenceTier {
    /// Mapped / public infrastructure (DOT cams, ACE routes).
    A,
    /// Estimated from inventories (fixed CCTV; recall ~0.63).
    B,
    /// Modeled field from real structure × assumed penetration (dashcams).
    C,
    /// Speculative / emerging (smart glasses).
    D,
}

/// The sensing classes tracked separately in the breakdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceKind {
    FixedCctv,
    DotLiveView,
    AceBus,
    Dashcam,
    SmartGlasses,
}

impl SourceKind {
    pub fn tier(self) -> ConfidenceTier {
        match self {
            SourceKind::DotLiveView => ConfidenceTier::A,
            SourceKind::AceBus => ConfidenceTier::A,
            SourceKind::FixedCctv => ConfidenceTier::B,
            SourceKind::Dashcam => ConfidenceTier::C,
            SourceKind::SmartGlasses => ConfidenceTier::D,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SourceKind::FixedCctv => "Fixed CCTV",
            SourceKind::DotLiveView => "DOT live-view",
            SourceKind::AceBus => "ACE buses",
            SourceKind::Dashcam => "Dashcams",
            SourceKind::SmartGlasses => "Smart glasses",
        }
    }

    /// Fixed (point-with-frustum) classes count distinct device ids and get the
    /// recall correction; mobile/ambient classes accumulate Poisson encounters.
    pub fn is_fixed(self) -> bool {
        matches!(self, SourceKind::FixedCctv | SourceKind::DotLiveView)
    }

    pub const ALL: [SourceKind; 5] = [
        SourceKind::FixedCctv,
        SourceKind::DotLiveView,
        SourceKind::AceBus,
        SourceKind::Dashcam,
        SourceKind::SmartGlasses,
    ];
}

/// Per-source accumulated exposure.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SourceTally {
    /// Expected number of distinct devices that captured you (the headline
    /// driver). Fixed: integer count of distinct ids. Mobile: Poisson mean.
    pub devices: f64,
    /// Expected number of image frames you appear in. Fixed: `frame_rate ×
    /// dwell`. Mobile: `encounters × frames-per-encounter`.
    pub frames: f64,
}

/// Poisson probability of at least one capture given an expected count.
#[inline]
pub fn p_at_least_one(expected: f64) -> f64 {
    1.0 - (-expected).exp()
}

/// The full exposure result, split by source plus the toggle metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposureTally {
    per_source: [SourceTally; 5],
    /// (source index, device id) pairs already counted, so a fixed camera seen
    /// across many ticks counts once. Transient; not serialized.
    #[serde(skip)]
    fixed_seen: HashSet<(u8, u64)>,
    /// Total route length walked, meters.
    pub route_length_m: f64,
    /// Length of the route under ≥1 active fixed coverage, meters.
    pub covered_length_m: f64,
    /// Recall correction factor applied to fixed device counts (1.0 = none).
    pub recall_factor: f64,
}

impl Default for ExposureTally {
    fn default() -> Self {
        ExposureTally {
            per_source: [SourceTally::default(); 5],
            fixed_seen: HashSet::new(),
            route_length_m: 0.0,
            covered_length_m: 0.0,
            recall_factor: 1.0,
        }
    }
}

impl ExposureTally {
    pub fn new() -> Self {
        Self::default()
    }

    fn idx(kind: SourceKind) -> usize {
        match kind {
            SourceKind::FixedCctv => 0,
            SourceKind::DotLiveView => 1,
            SourceKind::AceBus => 2,
            SourceKind::Dashcam => 3,
            SourceKind::SmartGlasses => 4,
        }
    }

    pub fn source(&self, kind: SourceKind) -> SourceTally {
        self.per_source[Self::idx(kind)]
    }

    /// Record one tick during which fixed device `id` (of `kind`) captured the
    /// walker: `frame_rate × dt` frames accrue; the device counts once.
    pub fn record_fixed_capture(&mut self, kind: SourceKind, id: u64, frame_rate: f64, dt: f64) {
        let i = Self::idx(kind);
        self.per_source[i].frames += frame_rate * dt;
        if self.fixed_seen.insert((i as u8, id)) {
            self.per_source[i].devices += 1.0;
        }
    }

    /// Accumulate mobile/ambient exposure for a class over a tick: expected
    /// distinct devices (Poisson mean increment) and expected frames.
    pub fn record_mobile(&mut self, kind: SourceKind, expected_devices: f64, expected_frames: f64) {
        let i = Self::idx(kind);
        self.per_source[i].devices += expected_devices;
        self.per_source[i].frames += expected_frames;
    }

    /// Mark that the walker advanced `step_m` meters this tick, `covered` of
    /// which were under at least one active fixed coverage.
    pub fn record_progress(&mut self, step_m: f64, covered: bool) {
        self.route_length_m += step_m;
        if covered {
            self.covered_length_m += step_m;
        }
    }

    /// Expected devices for a class, with the recall correction applied to fixed
    /// classes only.
    pub fn adjusted_devices(&self, kind: SourceKind) -> f64 {
        let d = self.source(kind).devices;
        if kind.is_fixed() {
            d * self.recall_factor
        } else {
            d
        }
    }

    /// HEADLINE: total expected distinct devices that could have captured you,
    /// across all classes (fixed recall-corrected), rounded.
    pub fn headline_device_count(&self) -> u32 {
        let total: f64 = SourceKind::ALL.iter().map(|&k| self.adjusted_devices(k)).sum();
        total.round().max(0.0) as u32
    }

    /// Poisson P(≥1 capture) from a class (using its adjusted device mean).
    pub fn p_capture(&self, kind: SourceKind) -> f64 {
        p_at_least_one(self.adjusted_devices(kind))
    }

    /// Total expected capture-events (image frames) across all sources.
    pub fn total_expected_frames(&self) -> f64 {
        self.per_source.iter().map(|s| s.frames).sum()
    }

    /// Fraction of the route under (fixed) surveillance, in [0, 1].
    pub fn fraction_surveilled(&self) -> f64 {
        if self.route_length_m <= 0.0 {
            0.0
        } else {
            (self.covered_length_m / self.route_length_m).clamp(0.0, 1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poisson_bounds() {
        assert!((p_at_least_one(0.0) - 0.0).abs() < 1e-12);
        assert!(p_at_least_one(0.7) > 0.49 && p_at_least_one(0.7) < 0.51);
        assert!(p_at_least_one(5.0) > 0.99);
    }

    #[test]
    fn distinct_devices_counted_once_frames_accumulate() {
        let mut t = ExposureTally::new();
        for _ in 0..3 {
            t.record_fixed_capture(SourceKind::FixedCctv, 1, 15.0, 1.0);
        }
        t.record_fixed_capture(SourceKind::FixedCctv, 2, 15.0, 1.0);
        let s = t.source(SourceKind::FixedCctv);
        assert_eq!(s.devices, 2.0);
        assert!((s.frames - 60.0).abs() < 1e-9); // 4 capture-ticks × 15 fps
    }

    #[test]
    fn same_id_different_kind_counts_separately() {
        let mut t = ExposureTally::new();
        t.record_fixed_capture(SourceKind::FixedCctv, 7, 1.0, 1.0);
        t.record_fixed_capture(SourceKind::DotLiveView, 7, 1.0, 1.0);
        assert_eq!(t.source(SourceKind::FixedCctv).devices, 1.0);
        assert_eq!(t.source(SourceKind::DotLiveView).devices, 1.0);
    }

    #[test]
    fn mobile_accumulates_poisson_mean() {
        let mut t = ExposureTally::new();
        t.record_mobile(SourceKind::AceBus, 0.3, 0.3);
        t.record_mobile(SourceKind::AceBus, 0.2, 0.2);
        assert!((t.source(SourceKind::AceBus).devices - 0.5).abs() < 1e-9);
        // P(>=1) for mean 0.5
        assert!((t.p_capture(SourceKind::AceBus) - (1.0 - (-0.5f64).exp())).abs() < 1e-9);
    }

    #[test]
    fn recall_correction_inflates_fixed_only() {
        let mut t = ExposureTally::new();
        t.recall_factor = 1.0 / DAHIR_RECALL; // ~1.587
        for id in 0..10u64 {
            t.record_fixed_capture(SourceKind::FixedCctv, id, 1.0, 1.0);
        }
        t.record_mobile(SourceKind::Dashcam, 4.0, 4.0); // mobile NOT recall-corrected
        // fixed: 10 -> ~16 ; dashcam: 4 -> 4 ; headline ~20
        assert!((t.adjusted_devices(SourceKind::FixedCctv) - 10.0 / DAHIR_RECALL).abs() < 1e-9);
        assert_eq!(t.adjusted_devices(SourceKind::Dashcam), 4.0);
        assert_eq!(t.headline_device_count(), 20);
    }

    #[test]
    fn fraction_surveilled() {
        let mut t = ExposureTally::new();
        t.record_progress(50.0, true);
        t.record_progress(50.0, false);
        assert!((t.fraction_surveilled() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn tiers() {
        assert_eq!(SourceKind::FixedCctv.tier(), ConfidenceTier::B);
        assert_eq!(SourceKind::SmartGlasses.tier(), ConfidenceTier::D);
        assert_eq!(SourceKind::AceBus.tier(), ConfidenceTier::A);
        assert!(SourceKind::FixedCctv.is_fixed());
        assert!(!SourceKind::AceBus.is_fixed());
    }
}
