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
    /// Direct-capture: the cameras whose field of view points straight at a point (lat/lon).
    DirectCapture { lat: f64, lon: f64 },
    /// Neighborhood-density choropleth; optional camera target (`at = None` = island overview).
    Neighborhoods { at: Option<(f64, f64, f32)> },
    /// Institutions explore view (subjects of surveillance, ranked by nearby cameras).
    /// `parks_only` filters the on-map markers to parks (else all classes show); `rank`
    /// selects + flies to the rank-th most-watched matching institution (0 = the most
    /// watched). Applied by a dedicated system (`crate::story_apply_institutions`), not
    /// `storymap_tick`, since the view's resources aren't in the tick's param set.
    Institutions { parks_only: bool, rank: usize },
    /// Sweep the simulated clock from `from` (or the hour at entry) to `to` across the
    /// step's dwell — the "a day in N seconds" time-lapse. Runs every frame (not once);
    /// `to` may exceed 24 to scrub past midnight (the value wraps mod 24 for display).
    ClockScrub { from: Option<f64>, to: f64 },
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

// ---- data-driven reel specs (docs/REELS_PLAN.md G2) --------------------
// A reel is authored as a JSON "spec" (a title + a list of steps) and handed to the app
// via `?reelspec=<base64url(json)>` (see `crate::url_reelspec`), so a new tour needs no
// recompile. `ReelSpec`/`ReelStep` are the wire format; `from_spec_json` turns them into
// the same `StoryStep`s the hardcoded tours produce. This is the mechanism behind
// `tools/reels/render.mjs --spec`.

/// The JSON reel spec: a human title + ordered steps.
#[derive(serde::Deserialize)]
pub struct ReelSpec {
    #[serde(default)]
    pub title: String,
    pub steps: Vec<ReelStep>,
}

/// One JSON step. `action` is the tag (matching `StepAction`); the rest are the fields
/// that action needs (validated in `to_action`). All geometry is lat/lon degrees; `at` is
/// `[lat, lon, zoom]`.
#[derive(serde::Deserialize)]
pub struct ReelStep {
    pub action: String,
    pub secs: f32,
    #[serde(default)]
    pub caption: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub zoom: Option<f32>,
    pub a: Option<[f64; 2]>,
    pub b: Option<[f64; 2]>,
    pub at: Option<[f64; 3]>,
    pub from: Option<f64>,
    pub to: Option<f64>,
    /// `Institutions`: filter markers to parks (default true) and which ranked match to
    /// select+fly to (default 0 = the most-watched).
    pub parks_only: Option<bool>,
    pub rank: Option<usize>,
    #[serde(default)]
    pub linknyc: bool,
    #[serde(default)]
    pub future: bool,
    #[serde(default)]
    pub operators: bool,
    #[serde(default)]
    pub heatmap: bool,
}

impl ReelStep {
    fn to_action(&self) -> Result<StepAction, String> {
        let need = |o: Option<f64>, f: &str| o.ok_or_else(|| format!("`{}` needs `{f}`", self.action));
        let at = self.at.map(|[la, lo, z]| (la, lo, z as f32));
        Ok(match self.action.as_str() {
            "Caption" => StepAction::Caption,
            "Overview" => StepAction::Overview,
            "FlyTo" => StepAction::FlyTo {
                lat: need(self.lat, "lat")?,
                lon: need(self.lon, "lon")?,
                zoom: self.zoom.unwrap_or(1.6),
            },
            "Route" => StepAction::Route {
                a: self.a.map(|p| (p[0], p[1])).ok_or("`Route` needs `a`")?,
                b: self.b.map(|p| (p[0], p[1])).ok_or("`Route` needs `b`")?,
            },
            "Walkshed" => StepAction::Walkshed {
                lat: need(self.lat, "lat")?,
                lon: need(self.lon, "lon")?,
            },
            "DirectCapture" => StepAction::DirectCapture {
                lat: need(self.lat, "lat")?,
                lon: need(self.lon, "lon")?,
            },
            "Neighborhoods" => StepAction::Neighborhoods { at },
            "Institutions" => StepAction::Institutions {
                parks_only: self.parks_only.unwrap_or(true),
                rank: self.rank.unwrap_or(0),
            },
            "ClockScrub" => StepAction::ClockScrub {
                from: self.from,
                to: self.to.ok_or("`ClockScrub` needs `to`")?,
            },
            "Operators" => StepAction::Operators,
            "Future" => StepAction::Future,
            "Heatmap" => StepAction::Heatmap,
            "Scene" => StepAction::Scene {
                at,
                linknyc: self.linknyc,
                future: self.future,
                operators: self.operators,
                heatmap: self.heatmap,
            },
            other => return Err(format!("unknown action `{other}`")),
        })
    }
}

/// Parse a JSON reel spec into a `(title, steps)` ready for [`StoryMap::start`]. Captions
/// and the title are leaked to `'static` (the tours are hardcoded `&'static str`); a reel
/// capture is a one-shot session, so leaking a handful of short strings is inconsequential.
pub fn from_spec_json(json: &str) -> Result<(&'static str, Vec<StoryStep>), String> {
    let spec: ReelSpec = serde_json::from_str(json).map_err(|e| format!("bad reel spec: {e}"))?;
    if spec.steps.is_empty() {
        return Err("reel spec has no steps".into());
    }
    let steps = spec
        .steps
        .iter()
        .map(|s| {
            Ok(StoryStep {
                caption: &*Box::leak(s.caption.clone().into_boxed_str()),
                secs: s.secs,
                action: s.to_action()?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let title: &'static str = if spec.title.is_empty() {
        "Reel"
    } else {
        &*Box::leak(spec.title.into_boxed_str())
    };
    Ok((title, steps))
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
            caption: "Welcome to Our Space, a living map of who is watching Manhattan.",
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
            caption: "Plan a walk. Drop a start and a destination, and the sim counts \
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
            caption: "The same sensors, regrouped by who runs them. A few operators \
                      account for most of the towers.",
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
            caption: "Zoom out to the citywide density field. The darker the block, the \
                      more cameras can see it.",
            secs: 6.5,
            action: Heatmap,
        },
        StoryStep {
            caption: "That's the tour. Click anywhere on the map to check your own block.",
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
            caption: "Rewind ten years. In 2015 the cameras were real but sparse: NYPD \
                      domes and DOT traffic cams. No plate readers on the avenues, no \
                      Wi-Fi kiosks logging phones.",
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
            caption: "Today: thousands of fixed cameras, license-plate readers, and a \
                      LinkNYC kiosk every few blocks. The kiosks switch on as the story \
                      reaches the present.",
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
            caption: "Most of Manhattan is now seen from several angles at once. The \
                      darkest blocks are watched the most.",
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
            caption: "A handful of operators run most of the city's cameras.",
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
            caption: "Five years out, a scenario: smart glasses and sidewalk delivery \
                      robots add cameras that move with the crowd. Same streets, more \
                      watching every year.",
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
    fn parses_a_reel_spec_and_maps_actions() {
        let json = r#"{
            "title": "Watch Astor Place",
            "steps": [
                { "action": "Overview", "secs": 1.5, "caption": "New York City" },
                { "action": "FlyTo", "lat": 40.73, "lon": -73.99, "zoom": 2.6, "secs": 2, "caption": "Astor Place" },
                { "action": "Walkshed", "lat": 40.73, "lon": -73.99, "secs": 4, "caption": "10-minute walk" },
                { "action": "DirectCapture", "lat": 40.73, "lon": -73.99, "secs": 3, "caption": "pointed at you" },
                { "action": "Neighborhoods", "at": [40.75, -73.98, 9.0], "secs": 3, "caption": "density" },
                { "action": "Institutions", "parks_only": true, "rank": 2, "secs": 4, "caption": "parks" },
                { "action": "ClockScrub", "from": 6.0, "to": 22.0, "secs": 14, "caption": "a day" }
            ]
        }"#;
        let (title, steps) = from_spec_json(json).unwrap();
        assert_eq!(title, "Watch Astor Place");
        assert_eq!(steps.len(), 7);
        assert!(matches!(steps[1].action, StepAction::FlyTo { zoom, .. } if (zoom - 2.6).abs() < 1e-6));
        assert!(matches!(steps[3].action, StepAction::DirectCapture { .. }));
        assert!(matches!(steps[4].action, StepAction::Neighborhoods { at: Some(_) }));
        assert!(matches!(steps[5].action, StepAction::Institutions { parks_only: true, rank: 2 }));
        assert!(matches!(steps[6].action, StepAction::ClockScrub { from: Some(_), to } if (to - 22.0).abs() < 1e-6));
        assert!((steps[0].secs - 1.5).abs() < 1e-6);
        // `Institutions` defaults: parks_only=true, rank=0 when the fields are omitted.
        let dflt = from_spec_json(r#"{"steps":[{"action":"Institutions","secs":3}]}"#).unwrap().1;
        assert!(matches!(dflt[0].action, StepAction::Institutions { parks_only: true, rank: 0 }));
    }

    #[test]
    fn reel_spec_rejects_missing_fields_and_empty() {
        // FlyTo without lat/lon is an error, not a silent default.
        let bad = r#"{"steps":[{"action":"FlyTo","secs":2}]}"#;
        assert!(from_spec_json(bad).is_err());
        // Empty step list is rejected.
        assert!(from_spec_json(r#"{"steps":[]}"#).is_err());
        // Unknown action is named in the error.
        let unk = r#"{"steps":[{"action":"Teleport","secs":1}]}"#;
        assert!(from_spec_json(unk).unwrap_err().contains("Teleport"));
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
