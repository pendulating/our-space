//! **StoryMap** — a scripted, auto-playing tour of the app: an ordered list of steps,
//! each a caption + a scene (camera move, mode switch, layer toggle…) that plays back
//! like a short video. The engine here is pure data + timing logic (so it unit-tests
//! without a running app); `crate::storymap_tick` applies each step's scene against the
//! live world, and `crate::ui::storymap_ui` draws the caption + transport controls.
//!
//! Deep-linkable: `?story=tutorial` in the URL auto-starts the tutorial on the web build
//! (see `crate::storymap_autostart`).

use bevy::prelude::Resource;

/// One scene a step sets up. Applied once when the step is entered; the engine resets
/// the transient overlays (Operators view, heatmap, future mode) to a baseline first,
/// so each step is a clean slate and order doesn't leak between steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepAction {
    /// Narrate only — leave the current view as-is.
    Caption,
    /// Pull back to a wide view of the island.
    Overview,
    /// Fly to a point (lat/lon) at a given camera scale (m/px).
    FlyTo { lat: f64, lon: f64, zoom: f32 },
    /// Plan-a-walk: drop A and B (lat/lon) and route between them.
    Route { a: (f64, f64), b: (f64, f64) },
    /// My-area: a 10-minute walkshed centered on a point (lat/lon).
    Walkshed { lat: f64, lon: f64 },
    /// Raise the Operators view (every sensor sorted by who runs it).
    Operators,
    /// Enter the "In 5 years…" speculative future (glasses + robots).
    Future,
    /// Show the citywide camera-density heatmap.
    Heatmap,
    /// A fully-composed era scene (used by the longitudinal story): camera target,
    /// plus explicit on/off for the layers that distinguish eras. `at = None` is the
    /// island overview. Unlike the simpler actions this sets LinkNYC explicitly (the
    /// kiosks launched 2016, so they're a real "then vs now" tell).
    Scene {
        at: Option<(f64, f64, f32)>, // lat, lon, camera scale (m/px)
        linknyc: bool,
        future: bool,
        operators: bool,
        heatmap: bool,
    },
}

/// A single tour step.
#[derive(Clone, Copy, Debug)]
pub struct StoryStep {
    pub caption: &'static str,
    /// Seconds to dwell before auto-advancing.
    pub secs: f32,
    pub action: StepAction,
}

/// Playback state for the active StoryMap. A resource so the tick system + UI share it.
#[derive(Resource, Default)]
pub struct StoryMap {
    pub steps: Vec<StoryStep>,
    pub idx: usize,
    /// Seconds elapsed in the current step.
    pub elapsed: f32,
    pub active: bool,
    pub paused: bool,
    /// Set when a step is (re)entered: the tick system applies its scene once, then clears.
    pub apply_pending: bool,
    /// Human title of the running story (for the overlay header).
    pub title: &'static str,
}

impl StoryMap {
    /// Begin a story from its first step.
    pub fn start(&mut self, title: &'static str, steps: Vec<StoryStep>) {
        self.title = title;
        self.steps = steps;
        self.idx = 0;
        self.elapsed = 0.0;
        self.active = !self.steps.is_empty();
        self.paused = false;
        self.apply_pending = self.active;
    }

    /// Stop playback (leaves the current scene on screen).
    pub fn stop(&mut self) {
        self.active = false;
        self.paused = false;
        self.apply_pending = false;
    }

    /// Jump to a step (clamped); re-arms the scene application.
    pub fn goto(&mut self, idx: usize) {
        if idx < self.steps.len() {
            self.idx = idx;
            self.elapsed = 0.0;
            self.apply_pending = true;
        }
    }

    /// Advance to the next step, or stop at the end.
    pub fn next(&mut self) {
        if self.idx + 1 < self.steps.len() {
            self.goto(self.idx + 1);
        } else {
            self.stop();
        }
    }

    /// Step back (no-op at the first step).
    pub fn prev(&mut self) {
        if self.idx > 0 {
            self.goto(self.idx - 1);
        }
    }

    /// Advance the clock; auto-advances (and may stop at the end) when the current
    /// step's dwell elapses. A no-op while paused or inactive.
    pub fn tick(&mut self, dt: f32) {
        if !self.active || self.paused {
            return;
        }
        self.elapsed += dt;
        if let Some(step) = self.steps.get(self.idx) {
            if self.elapsed >= step.secs {
                self.next();
            }
        }
    }

    pub fn current(&self) -> Option<&StoryStep> {
        self.steps.get(self.idx)
    }
}

/// The first StoryMap: a guided tour through every part of the app.
pub fn tutorial() -> Vec<StoryStep> {
    use StepAction::*;
    vec![
        StoryStep {
            caption: "Welcome to Our Space — a living map of who is watching Manhattan.",
            secs: 5.5,
            action: Overview,
        },
        StoryStep {
            caption: "Every marker is a fixed surveillance camera: NYPD CCTV, DOT traffic \
                      cams, and private license-plate readers. Midtown alone is saturated.",
            secs: 7.0,
            action: FlyTo { lat: 40.7549, lon: -73.9840, zoom: 1.6 },
        },
        StoryStep {
            caption: "Plan a walk — drop a start and a destination, and the sim counts \
                      every camera that could capture you along the way.",
            secs: 7.5,
            action: Route { a: (40.7580, -73.9855), b: (40.7527, -73.9772) },
        },
        StoryStep {
            caption: "Or study your whole neighborhood: a 10-minute walkshed, and every \
                      camera whose view reaches into it.",
            secs: 7.5,
            action: Walkshed { lat: 40.7233, lon: -74.0030 },
        },
        StoryStep {
            caption: "Meet the operators — the same sensors, regrouped by who runs them. \
                      The towers show just how lopsided the watching is.",
            secs: 7.0,
            action: Operators,
        },
        StoryStep {
            caption: "In 5 years: AI smart glasses and sidewalk delivery robots add \
                      always-on, roving cameras to the same streets.",
            secs: 7.0,
            action: Future,
        },
        StoryStep {
            caption: "Zoom out to the citywide density field — the darker the block, the \
                      more cameras can see it.",
            secs: 6.5,
            action: Heatmap,
        },
        StoryStep {
            caption: "That's the tour. Click anywhere to start exploring your own corner \
                      of the surveilled city.",
            secs: 6.0,
            action: Overview,
        },
    ]
}

/// The longitudinal StoryMap: the same streets watched more every year — sparse 2015,
/// saturated today, speculative +5 years (the "In 5 years…" future layer). Each step is
/// a composed `Scene`, so LinkNYC kiosks genuinely appear between "then" and "now".
pub fn longitudinal() -> Vec<StoryStep> {
    use StepAction::Scene;
    // A camera-dense midtown vantage reused across the "now" beats.
    let midtown = |z: f32| Some((40.7549, -73.9840, z));
    vec![
        StoryStep {
            caption: "Rewind ten years. In 2015 the city's eye was real but sparse — NYPD \
                      domes, DOT traffic cams. No plate-readers on the avenues, no Wi-Fi \
                      kiosks logging your phone.",
            secs: 8.0,
            action: Scene {
                at: None,
                linknyc: false,
                future: false,
                operators: false,
                heatmap: false,
            },
        },
        StoryStep {
            caption: "By today, the lens is everywhere. Thousands of fixed cameras, \
                      license-plate readers, and a LinkNYC kiosk on every other block — \
                      watch the kiosks switch on as we reach the present.",
            secs: 8.0,
            action: Scene {
                at: midtown(1.6),
                linknyc: true,
                future: false,
                operators: false,
                heatmap: false,
            },
        },
        StoryStep {
            caption: "The density makes it plain: most of Manhattan is now seen from many \
                      angles at once. The darkest blocks are watched the most.",
            secs: 7.5,
            action: Scene {
                at: None,
                linknyc: true,
                future: false,
                operators: false,
                heatmap: true,
            },
        },
        StoryStep {
            caption: "And the watching is concentrated — a handful of operators run the \
                      overwhelming majority of the city's cameras.",
            secs: 7.0,
            action: Scene {
                at: None,
                linknyc: true,
                future: false,
                operators: true,
                heatmap: false,
            },
        },
        StoryStep {
            caption: "Five years on: AI smart glasses on commuters and sidewalk delivery \
                      robots add cameras that move with the crowd — the 'In 5 years…' layer. \
                      Same streets, watched more every year.",
            secs: 8.5,
            action: Scene {
                at: midtown(1.8),
                linknyc: true,
                future: true,
                operators: false,
                heatmap: false,
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(n: usize) -> Vec<StoryStep> {
        (0..n)
            .map(|_| StoryStep { caption: "x", secs: 2.0, action: StepAction::Caption })
            .collect()
    }

    #[test]
    fn start_arms_first_step() {
        let mut s = StoryMap::default();
        s.start("t", steps(3));
        assert!(s.active && s.apply_pending && s.idx == 0);
        // empty story never activates
        let mut e = StoryMap::default();
        e.start("t", vec![]);
        assert!(!e.active);
    }

    #[test]
    fn tick_auto_advances_then_stops_at_end() {
        let mut s = StoryMap::default();
        s.start("t", steps(2));
        s.apply_pending = false; // pretend the scene was applied
        s.tick(1.0);
        assert_eq!(s.idx, 0, "still within the first step");
        s.tick(1.5); // crosses 2.0s dwell
        assert_eq!(s.idx, 1, "advanced to step 2");
        assert!(s.apply_pending, "new step re-arms scene application");
        s.apply_pending = false;
        s.tick(2.5); // past the last step's dwell
        assert!(!s.active, "stops at the end");
    }

    #[test]
    fn paused_tick_is_inert_and_nav_is_bounded() {
        let mut s = StoryMap::default();
        s.start("t", steps(3));
        s.paused = true;
        s.tick(99.0);
        assert_eq!(s.idx, 0, "paused: clock frozen");
        s.paused = false;
        s.prev();
        assert_eq!(s.idx, 0, "prev clamped at first");
        s.goto(2);
        s.next();
        assert!(!s.active, "next past last stops");
    }

    #[test]
    fn tutorial_has_steps_and_covers_key_actions() {
        let t = tutorial();
        assert!(t.len() >= 6);
        let has = |a: StepAction| t.iter().any(|s| s.action == a);
        assert!(has(StepAction::Operators));
        assert!(has(StepAction::Future));
        assert!(has(StepAction::Heatmap));
        assert!(t.iter().any(|s| matches!(s.action, StepAction::Route { .. })));
        assert!(t.iter().any(|s| matches!(s.action, StepAction::Walkshed { .. })));
    }

    #[test]
    fn longitudinal_runs_then_to_now_to_future() {
        let l = longitudinal();
        assert!(l.len() >= 4);
        // 2015: no kiosks, no future layer.
        assert!(matches!(
            l[0].action,
            StepAction::Scene { linknyc: false, future: false, .. }
        ));
        // Kiosks switch on for the "now" beats (a real then-vs-now tell).
        assert!(l
            .iter()
            .any(|s| matches!(s.action, StepAction::Scene { linknyc: true, .. })));
        // The finale uses the "In 5 years…" future layer.
        assert!(matches!(
            l.last().unwrap().action,
            StepAction::Scene { future: true, .. }
        ));
    }
}
