//! The **Operators view**: a presentation overlay that animates every sensor
//! *off the map* into one vertical column per operator/company, packed as a
//! literal 1:1 stack (one chip per device) so the screen becomes an immediate
//! magnitude comparison of *whose lens* is watching the city — the ~4,400-tall
//! CCTV column towering over NYC DOT's ~370.
//!
//! It touches no exposure numbers; it only re-arranges what is already on screen.
//! Fixed cameras are 3 merged textured-quad meshes ([`crate::world::merged_icon_quads`]),
//! so we fly them by lerping `Mesh::ATTRIBUTE_POSITION` in place (3 draw calls
//! hold). Mobile agents are individual entities, flown by their `Transform`
//! (see [`operators_snapshot`] / [`operators_animate_mobile`]). The whole thing
//! is driven by one resource with an eased progress `t`, matching the app's
//! resource-driven, no-`States` style.

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::sprite::Anchor;

use crate::agents::{AgentClass, AgentPool, MobileAgent};
use crate::{AceVis, BaseMap, BuildingVis, LandmarkVis, OutlineVis, RouteVis};

/// Seconds for the full off-the-map / back-home flight.
const TRANSITION_SECS: f32 = 1.1;
/// Fraction of the timeline spent staggering chip departures (the "sweep").
const STAGGER: f32 = 0.40;
/// Chip fill factor: a hair under the grid pitch leaves a hairline gutter.
const CHIP_FILL: f32 = 0.86;
/// Progress at which a sensor's *appearance* morphs from its on-map mark (the branded
/// wordmark / moving icon) to the solid bar-graph chip. Keeping the texture for the
/// first half means the markers fly off the map looking like themselves, then change to
/// the stacked-chart shape mid-flight (rather than turning into squares on click).
const CHIP_MORPH_T: f32 = 0.5;
/// Half-width (in progress units) of the dissolve window around `CHIP_MORPH_T`, and the
/// dimmest alpha at the swap instant. The mark briefly fades as it crosses the swap so
/// the wordmark→solid-chip change reads as a soft cross-dissolve, not a hard pop.
const MORPH_DISSOLVE_HALF: f32 = 0.14;
const MORPH_DISSOLVE_FLOOR: f32 = 0.22;

/// Alpha multiplier that dips to [`MORPH_DISSOLVE_FLOOR`] right at the texture↔chip
/// swap and is 1.0 outside the window — masking the discrete shape change behind a
/// quick fade so the morph feels seamless.
fn morph_dissolve(t: f32) -> f32 {
    let d = (t - CHIP_MORPH_T).abs();
    if d >= MORPH_DISSOLVE_HALF {
        1.0
    } else {
        MORPH_DISSOLVE_FLOOR + (1.0 - MORPH_DISSOLVE_FLOOR) * (d / MORPH_DISSOLVE_HALF)
    }
}

// ----------------------------------------------------------- operator columns --

/// The operator columns. Some are the fixed map markers (merged meshes); the rest
/// are the moving ambient agents. Declaration order is left→right layout order
/// (CCTV first so the tallest tower anchors the comparison; the two car-camera
/// operators — Rideshare + Tesla — sit together).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OperatorCol {
    Cctv,
    Dot,
    Flock,
    Enforcement,
    Mta,
    Rideshare,
    Tesla,
    Meta,
}

/// Left→right column order.
pub const COLS: [OperatorCol; 8] = [
    OperatorCol::Cctv,
    OperatorCol::Dot,
    OperatorCol::Flock,
    OperatorCol::Enforcement,
    OperatorCol::Mta,
    OperatorCol::Rideshare,
    OperatorCol::Tesla,
    OperatorCol::Meta,
];

impl OperatorCol {
    fn index(self) -> usize {
        COLS.iter().position(|&c| c == self).unwrap_or(0)
    }

    /// Branded wordmark shown as the column header.
    pub fn label(self) -> &'static str {
        match self {
            OperatorCol::Cctv => "CCTV",
            OperatorCol::Dot => "DOT",
            OperatorCol::Flock => "ALPR",
            OperatorCol::Enforcement => "ENF",
            OperatorCol::Mta => "MTA",
            OperatorCol::Rideshare => "RIDESHARE",
            OperatorCol::Tesla => "TESLA",
            OperatorCol::Meta => "META",
        }
    }

    /// One-line who-they-are gloss under the wordmark.
    pub fn gloss(self) -> &'static str {
        match self {
            OperatorCol::Cctv => "street CCTV census",
            OperatorCol::Dot => "NYC DOT traffic cams",
            OperatorCol::Flock => "Flock + agency plate readers",
            OperatorCol::Enforcement => "photo-enforcement cams",
            OperatorCol::Mta => "MTA ACE bus cams",
            OperatorCol::Rideshare => "rideshare dashcams",
            OperatorCol::Tesla => "Tesla Sentry/Autopilot",
            OperatorCol::Meta => "smart glasses",
        }
    }

    /// The operator's ink. Surveillance operators sit on the warm red→orange ramp
    /// (matching their map markers); transit/infrastructure (ACE) is cool blue.
    pub fn color(self) -> Color {
        use crate::theme::map;
        match self {
            OperatorCol::Cctv => map::MAROON,          // dense fixed-camera baseline
            OperatorCol::Dot => map::AMBER_700,        // city traffic cams (warm)
            OperatorCol::Flock => map::RED,            // the headline threat — plate readers
            OperatorCol::Enforcement => map::ORANGE_600, // photo-enforcement warning
            OperatorCol::Mta => map::BLUE,             // transit — cool blue
            OperatorCol::Rideshare => map::ORANGE,     // private fleet, warm
            OperatorCol::Tesla => map::TESLA_AMBER,    // amber-600, distinct from rideshare
            OperatorCol::Meta => map::ZINC_600,        // speculative glasses, muted
        }
    }
}

// --------------------------------------------------------- ALPR maker strata --

/// The plate-reader manufacturers we stratify the ALPR tower by (the DeFlock/OSM
/// `manufacturer` tag, baked per reader). Within the one ALPR operator column,
/// chips are grouped + tinted by maker so the stack reads *which* vendors saturate
/// the city — Flock is the household name, but agencies also run Leonardo/ELSAG,
/// Redspeed, Ekin, Mav, Genetec, Motorola hardware.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Maker {
    Flock,
    Leonardo,
    Redspeed,
    Ekin,
    Mav,
    Genetec,
    Motorola,
    Other,
}

/// Stable display / banding order (most-recognized first; `Other` last).
pub const MAKERS: [Maker; 8] = [
    Maker::Flock,
    Maker::Leonardo,
    Maker::Redspeed,
    Maker::Ekin,
    Maker::Mav,
    Maker::Genetec,
    Maker::Motorola,
    Maker::Other,
];

impl Maker {
    /// Classify a DeFlock/OSM `manufacturer` string (case-insensitive substring).
    /// ELSAG is Leonardo's plate-reader brand, so both fold to `Leonardo`.
    pub fn classify(manufacturer: Option<&str>) -> Maker {
        let Some(m) = manufacturer else { return Maker::Other };
        let m = m.to_ascii_lowercase();
        if m.contains("flock") {
            Maker::Flock
        } else if m.contains("leonardo") || m.contains("elsag") {
            Maker::Leonardo
        } else if m.contains("redspeed") {
            Maker::Redspeed
        } else if m.contains("ekin") {
            Maker::Ekin
        } else if m.contains("mav") {
            Maker::Mav
        } else if m.contains("genetec") || m.contains("autovu") {
            Maker::Genetec
        } else if m.contains("motorola") || m.contains("vigilant") || m.contains("neology") {
            Maker::Motorola
        } else {
            Maker::Other
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Maker::Flock => "Flock",
            Maker::Leonardo => "Leonardo / ELSAG",
            Maker::Redspeed => "Redspeed",
            Maker::Ekin => "Ekin",
            Maker::Mav => "Mav Systems",
            Maker::Genetec => "Genetec",
            Maker::Motorola => "Motorola / Vigilant",
            Maker::Other => "other / unlabeled",
        }
    }

    /// A categorical tint, distinguishable on the pale tower ground. Flock keeps the
    /// ALPR signature red; the rest spread across hue so bands separate cleanly.
    pub fn color(self) -> Color {
        use crate::theme::map;
        match self {
            Maker::Flock => map::RED,             // the headline brand
            Maker::Leonardo => map::ORANGE_600,   // warm
            Maker::Redspeed => map::AMBER_700,    // deep amber
            Maker::Ekin => Color::srgb(0.74, 0.10, 0.36), // rose
            Maker::Mav => Color::srgb(0.06, 0.46, 0.43),  // teal
            Maker::Genetec => map::BLUE,          // blue
            Maker::Motorola => Color::srgb(0.43, 0.16, 0.85), // violet
            Maker::Other => map::ZINC_600,        // muted
        }
    }

    /// `[r,g,b,a]` for a mesh `ATTRIBUTE_COLOR` vertex.
    pub fn rgba(self) -> [f32; 4] {
        let c = self.color().to_srgba();
        [c.red, c.green, c.blue, c.alpha]
    }
}

/// Which column an ambient agent class belongs to. Delivery robots are speculative
/// (not a mapped operator), so they're excluded from the tower → `None`.
pub fn col_of_class(class: AgentClass) -> Option<OperatorCol> {
    match class {
        AgentClass::Vehicle => Some(OperatorCol::Rideshare),
        AgentClass::Bus => Some(OperatorCol::Mta),
        AgentClass::Pedestrian => Some(OperatorCol::Meta),
        AgentClass::DeliveryRobot => None,
        AgentClass::Tesla => Some(OperatorCol::Tesla),
    }
}

// --------------------------------------------------------------- resources -----

/// Drives the whole view. `t` is raw linear progress in `0..=1` (0 = on the map,
/// 1 = fully stacked); per-chip easing/stagger/arc are applied in the animation
/// systems, not here. `idle`/`settled` gate the per-frame mesh rewrite so we only
/// pay the vertex cost during the ~1.1 s flight, never at rest.
#[derive(Resource)]
pub struct OperatorsView {
    pub active: bool,
    pub t: f32,
    pub dir: f32,
    pub reduced_motion: bool,
    /// Fully home for ≥1 frame (the landing write already happened): skip animation.
    pub idle: bool,
    /// Fully stacked for ≥1 frame: skip animation.
    pub settled: bool,
}

impl Default for OperatorsView {
    fn default() -> Self {
        OperatorsView {
            active: false,
            t: 0.0,
            dir: 1.0,
            reduced_motion: false,
            idle: true,
            settled: false,
        }
    }
}

/// A single column's packed-grid geometry (world space, computed on enter/resize).
#[derive(Clone)]
pub struct ColLayout {
    pub col: OperatorCol,
    /// Lane center x.
    pub lane_cx: f32,
    /// Bottom baseline y (towers grow upward from here).
    pub base_y: f32,
    /// Chips per row before wrapping.
    pub per_row: usize,
    /// Grid pitch (shared across all columns → exact magnitude comparison).
    pub pitch: f32,
    /// Chips in this column at build time.
    pub count: usize,
    /// World pos for the wordmark header (top-center anchor, hangs below baseline).
    pub header_pos: Vec3,
}

impl ColLayout {
    /// World-space center of chip slot `i` (bottom row first, wrapping upward).
    pub fn slot(&self, i: usize) -> Vec2 {
        let per_row = self.per_row.max(1);
        let col = (i % per_row) as f32;
        let row = (i / per_row) as f32;
        let row_w = per_row as f32 * self.pitch;
        let x = self.lane_cx - row_w * 0.5 + (col + 0.5) * self.pitch;
        let y = self.base_y + (row + 0.5) * self.pitch;
        Vec2::new(x, y)
    }
}

/// The current column layout. Rebuilt on enter and on window resize while active.
#[derive(Resource, Default)]
pub struct OperatorsLayout {
    pub built: bool,
    pub win: Vec2,
    /// Shared chip half-size in the tower (world meters).
    pub chip_half: f32,
    /// Meters per CSS pixel of the frozen camera (sizes world-space header text).
    pub mpp: f32,
    /// One per [`COLS`] entry, in column order.
    pub cols: Vec<ColLayout>,
}

impl OperatorsLayout {
    pub fn get(&self, col: OperatorCol) -> Option<&ColLayout> {
        self.cols.iter().find(|c| c.col == col)
    }
}

// ------------------------------------------------------------- components ------

/// Tags each of the 3 merged fixed-camera meshes with its operator column and a
/// copy of its per-chip home positions (parallel to the mesh's quad order).
#[derive(Component)]
pub struct OperatorMesh {
    pub col: OperatorCol,
    pub mesh: Handle<Mesh>,
    /// Current on-map half-size of the icon quad (zoom-responsive — `scale_camera_icons`
    /// floors it to a minimum on-screen size; chips lerp from here to the tower size).
    pub home_half: f32,
    /// Intrinsic (un-floored) half-size in world meters — the size when zoomed in far
    /// enough that the screen-floor no longer applies.
    pub base_half: f32,
    /// Per-chip home center in world space (chip `i` owns mesh verts `[4i, 4i+3]`).
    pub homes: Vec<Vec2>,
    /// Shared material — branded wordmark on the resting map, solid operator color
    /// in the tower (swapped by [`operators_chip_material`]).
    pub material: Handle<ColorMaterial>,
    /// The branded-wordmark texture (restored when home).
    pub branded: Handle<Image>,
    /// ALPR only: per-vertex maker tints (4 entries per chip, parallel to the mesh
    /// quad order). The mesh ships with white vertex colors so map owls stay
    /// uniform; [`operators_chip_material`] writes these in when the tower is up so
    /// the column bands by manufacturer. `None` for the non-stratified columns.
    pub maker_colors: Option<Vec<[f32; 4]>>,
}

/// Per-agent flight slot for the mobile columns. Filled on enter by
/// [`operators_snapshot`]; consumed by [`operators_animate_mobile`].
#[derive(Component, Default)]
pub struct OperatorSlot {
    pub home: Vec2,
    pub target: Vec2,
    pub idx: usize,
    pub n: usize,
    pub assigned: bool,
}

/// A column's branded wordmark + live count, drawn as world-space text above the
/// tower baseline (one per [`COLS`] entry, spawned once).
#[derive(Component)]
pub struct OperatorHeader {
    pub col: OperatorCol,
}

// ------------------------------------------------------------ easing helpers ---

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Per-chip eased sub-progress: chip `i` of `n` peels off a little later than the
/// previous (a sweep), then eases. Reduced motion snaps to a hard 0/1 (no arc).
pub fn chip_progress(t: f32, col: OperatorCol, i: usize, n: usize, reduced: bool) -> f32 {
    if reduced {
        return if t >= 1.0 { 1.0 } else { 0.0 };
    }
    let frac = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 };
    let col_phase = col.index() as f32 * 0.015;
    let start = (frac * STAGGER + col_phase).min(STAGGER + 0.1);
    let local = ((t - start) / (1.0 - start)).clamp(0.0, 1.0);
    ease_in_out_cubic(local)
}

/// Quadratic-Bézier flight: the control point lifts perpendicular to the
/// home→target chord so chips curve in (the "sucked off the map" swoop). Reverse
/// (`t` running 1→0) traces the same arc home. `te` is the eased sub-progress.
pub fn arc_lerp(home: Vec2, target: Vec2, te: f32) -> Vec2 {
    let chord = target - home;
    let len = chord.length().max(1.0);
    let perp = Vec2::new(-chord.y, chord.x) / len;
    let lift = (len * 0.16).min(160.0);
    let ctrl = home + chord * 0.5 + perp * lift;
    let u = 1.0 - te;
    home * (u * u) + ctrl * (2.0 * u * te) + target * (te * te)
}

/// Largest shared grid pitch (world m) such that the tallest column — `n_max`
/// chips wrapping at `floor(lane_w / pitch)` per row — still fits in `avail_h`.
/// Every column uses this one pitch, so tower heights compare 1:1.
fn fit_pitch(n_max: f32, lane_w: f32, avail_h: f32) -> f32 {
    let tower_h = |pitch: f32| -> f32 {
        let per_row = (lane_w / pitch).floor().max(1.0);
        (n_max / per_row).ceil() * pitch
    };
    let mut lo = 0.3_f32;
    let mut hi = lane_w.max(0.5);
    for _ in 0..40 {
        let p = 0.5 * (lo + hi);
        if tower_h(p) <= avail_h {
            lo = p; // fits → try bigger
        } else {
            hi = p;
        }
    }
    lo
}

// --------------------------------------------------------------- systems -------

/// Advance `t` toward the toggle target and maintain the idle/settled gates.
/// Chained before layout/animation so the gates are current when they read them.
pub fn operators_drive(time: Res<Time>, mut ov: ResMut<OperatorsView>) {
    ov.dir = if ov.active { 1.0 } else { -1.0 };
    let prev = ov.t;
    if ov.reduced_motion {
        ov.t = if ov.active { 1.0 } else { 0.0 };
    } else {
        let step = time.delta_secs() / TRANSITION_SECS;
        ov.t = (ov.t + ov.dir * step).clamp(0.0, 1.0);
    }
    // One landing write happens on the frame a terminus is first reached (prev not
    // yet at the terminus); the next frame gates the animation off.
    ov.idle = !ov.active && ov.t <= 0.0 && prev <= 0.0;
    ov.settled = ov.active && ov.t >= 1.0 && prev >= 1.0;
}

/// (Re)build the column layout from the frozen camera viewport. Kept valid while
/// `t > 0` so the home flight still has targets; dropped only when fully home.
/// The operator columns currently worth a lane — those with at least one device.
/// Toggled-off / empty layers (count 0) drop out so the tower shows no empty lanes.
pub fn present_columns(counts: &[usize]) -> Vec<OperatorCol> {
    COLS.iter()
        .copied()
        .filter(|c| counts.get(c.index()).copied().unwrap_or(0) > 0)
        .collect()
}

pub fn operators_layout(
    ov: Res<OperatorsView>,
    mut layout: ResMut<OperatorsLayout>,
    windows: Query<&Window>,
    cam: Query<&Transform, With<Camera2d>>,
    fixed: Query<&OperatorMesh>,
    agents: Query<&MobileAgent>,
) {
    if !ov.active && ov.t <= 0.0 {
        layout.built = false;
        return;
    }
    let Ok(win) = windows.single() else { return };
    let size = Vec2::new(win.width(), win.height());
    // Keep the existing layout while flying home; rebuild on resize only while active.
    if layout.built && (!ov.active || layout.win == size) {
        return;
    }
    let Ok(cam_t) = cam.single() else { return };

    // Per-column device counts (fixed = mesh chip count; mobile = active agents).
    let mut counts = [0usize; COLS.len()];
    for om in &fixed {
        counts[om.col.index()] += om.homes.len();
    }
    for a in &agents {
        if a.active {
            if let Some(col) = col_of_class(a.class) {
                counts[col.index()] += 1;
            }
        }
    }
    let n_max = (*counts.iter().max().unwrap_or(&1)).max(1) as f32;

    // Only lay out lanes for layers actually present (count > 0): a toggled-off class
    // (smart glasses outside "In 5 years…", rideshare/Tesla when off) drops out of the
    // tower instead of leaving an empty lane.
    let present = present_columns(&counts);

    // Frozen viewport → world rect (ortho, no rotation: same convention as
    // `camera_control`'s pixel→world pan).
    let s = cam_t.scale.x.max(f32::EPSILON);
    let (cx, cy) = (cam_t.translation.x, cam_t.translation.y);
    let world_w = win.width() * s;
    let world_h = win.height() * s;
    let left = cx - world_w * 0.5;
    let bottom = cy - world_h * 0.5;

    // Exclude the 350 px right control panel so the last lane isn't hidden.
    let panel_w = (350.0 + 12.0) * s;
    let outer_m = 0.03 * world_w;
    let gutter = 0.012 * world_w;
    let n_cols = present.len().max(1) as f32;
    let usable_w = (world_w - panel_w - 2.0 * outer_m - (n_cols - 1.0) * gutter).max(1.0);
    let lane_w = usable_w / n_cols;

    // Vertical budget: a label strip at the bottom (room for the two-line headers,
    // which are bottom-anchored just above the viewport edge), a little top padding.
    let label_strip = 0.12 * world_h;
    let top_pad = 0.04 * world_h;
    let base_y = bottom + label_strip;
    let avail_h = (world_h - label_strip - top_pad).max(1.0);

    // Auto-fit the shared grid pitch so the tallest column exactly fills `avail_h`.
    let pitch = fit_pitch(n_max, lane_w, avail_h);
    let per_row = (lane_w / pitch).floor().max(1.0) as usize;
    layout.chip_half = pitch * 0.5 * CHIP_FILL;
    layout.mpp = s;

    layout.cols.clear();
    for (k, &col) in present.iter().enumerate() {
        let lane_cx = left + outer_m + k as f32 * (lane_w + gutter) + lane_w * 0.5;
        let count = counts[col.index()];
        // Header sits a comfortable margin above the viewport bottom (bottom-center
        // anchor → text grows upward toward the tower), so all column labels line up
        // in a row and never clip the bottom edge.
        let header_pos = Vec3::new(lane_cx, bottom + 0.035 * world_h, 5.0);
        layout.cols.push(ColLayout {
            col,
            lane_cx,
            base_y,
            per_row,
            pitch,
            count,
            header_pos,
        });
    }

    layout.built = true;
    layout.win = size;
}

/// Fly the fixed-camera chips between their map homes and column slots by
/// rewriting each merged mesh's vertex positions in place (3 draw calls hold).
pub fn operators_animate_fixed(
    ov: Res<OperatorsView>,
    layout: Res<OperatorsLayout>,
    mut meshes: ResMut<Assets<Mesh>>,
    q: Query<&OperatorMesh>,
) {
    if ov.idle || ov.settled {
        return; // at rest (home or stacked): no rewrite
    }
    for om in &q {
        let Some(cl) = layout.get(om.col) else { continue };
        let Some(mesh) = meshes.get_mut(&om.mesh) else { continue };
        let Some(VertexAttributeValues::Float32x3(pos)) =
            mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
        else {
            continue;
        };
        let n = om.homes.len();
        for i in 0..n {
            let te = chip_progress(ov.t, om.col, i, n, ov.reduced_motion);
            let center = arc_lerp(om.homes[i], cl.slot(i), te);
            // Chips shrink/grow from on-map icon size to the uniform tower size.
            let h = lerp(om.home_half, layout.chip_half, te);
            let b = i * 4;
            // Same corner winding as `world::merged_icon_quads` (TL,TR,BR,BL).
            pos[b] = [center.x - h, center.y + h, 0.0];
            pos[b + 1] = [center.x + h, center.y + h, 0.0];
            pos[b + 2] = [center.x + h, center.y - h, 0.0];
            pos[b + 3] = [center.x - h, center.y - h, 0.0];
        }
    }
}

/// On entering the view, snapshot the currently-active agents and assign each a
/// column slot (its current map position is the home for the return flight). The
/// ambient sim is frozen while active (see the guards in `agents.rs`), so this
/// active set stays valid for the whole view. Runs only on the enter edge.
pub fn operators_snapshot(
    ov: Res<OperatorsView>,
    layout: Res<OperatorsLayout>,
    mut last_active: Local<bool>,
    mut q: Query<(&MobileAgent, &Transform, &mut OperatorSlot)>,
) {
    if ov.active == *last_active {
        return; // no edge
    }
    *last_active = ov.active;
    if !ov.active {
        return; // exit: keep slots so the home flight has data; ambient reclaims at t=0
    }
    let mut cursor = [0usize; COLS.len()];
    for (agent, tf, mut slot) in &mut q {
        if !agent.active {
            slot.assigned = false;
            continue;
        }
        let Some(col) = col_of_class(agent.class) else {
            slot.assigned = false; // robots are excluded from the operator tower
            continue;
        };
        let Some(cl) = layout.get(col) else { continue };
        let i = cursor[col.index()];
        cursor[col.index()] += 1;
        slot.home = tf.translation.truncate();
        slot.target = cl.slot(i);
        slot.idx = i;
        slot.n = cl.count.max(1);
        slot.assigned = true;
    }
}

/// Fly the snapshotted agents between map and column by tweening their transforms
/// (they're individual entities, so no mesh surgery). When fully home the ambient
/// `animate_agents` owns them again, snapping them to `position_at(progress_m)`
/// (their untouched route position == their home), so the handoff is seamless.
pub fn operators_animate_mobile(
    ov: Res<OperatorsView>,
    layout: Res<OperatorsLayout>,
    mut q: Query<(&mut Transform, &OperatorSlot, &MobileAgent)>,
) {
    if ov.idle || ov.settled {
        return; // home (ambient sim drives them) or parked in the tower
    }
    let cell = 2.0 * layout.chip_half; // target on-grid chip size (world m)
    for (mut tf, slot, agent) in &mut q {
        if !slot.assigned || !agent.active {
            continue;
        }
        let Some(col) = col_of_class(agent.class) else { continue };
        let te = chip_progress(ov.t, col, slot.idx, slot.n, ov.reduced_motion);
        let c = arc_lerp(slot.home, slot.target, te);
        tf.translation.x = c.x;
        tf.translation.y = c.y;
        tf.rotation = Quat::IDENTITY; // clear the dashcam travel-spin while stacked
        // Shrink/grow from the agent's native icon size into the uniform grid cell
        // (mirrors the fixed chips so all columns pack at one scale).
        let fit = (cell / agent_base_size(agent.class)).clamp(0.05, 4.0);
        tf.scale = Vec3::splat(lerp(1.0, fit, te));
    }
}

/// On-map render size (world m) of each ambient agent's icon mesh (set in
/// `agents::spawn_pool`), so the Operators view can normalize them to the grid.
fn agent_base_size(class: AgentClass) -> f32 {
    match class {
        AgentClass::Vehicle => 14.0,   // RegularPolygon(7.0, 3) circumradius ×2
        AgentClass::Bus => 30.0,       // bus.png quad
        AgentClass::Pedestrian => 16.0, // glasses.png quad
        AgentClass::DeliveryRobot => 10.0, // Rectangle(10×8) box (excluded from tower)
        AgentClass::Tesla => 14.0,      // RegularPolygon(7,3) triangle (excluded from tower)
    }
}

/// `4422` → `"4,422"`.
fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let b = s.as_bytes();
    let mut out = String::with_capacity(b.len() + b.len() / 3);
    for (i, &c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c as char);
    }
    out
}

/// Spawn (once) and drive the six branded column headers: world-space `Text2d`
/// wordmark + live count, sized in CSS pixels off the frozen camera and faded in
/// over the back half of the flight.
pub fn operators_headers(
    ov: Res<OperatorsView>,
    layout: Res<OperatorsLayout>,
    mut commands: Commands,
    mut q: Query<(
        &OperatorHeader,
        &mut Transform,
        &mut Text2d,
        &mut TextColor,
        &mut Visibility,
    )>,
) {
    // Spawn lazily, once we have a layout to place them against.
    if q.is_empty() {
        if layout.built {
            for &col in &COLS {
                commands.spawn((
                    Text2d::new(col.label()),
                    TextFont {
                        font_size: 64.0,
                        ..default()
                    },
                    TextColor(col.color()),
                    TextLayout::new_with_justify(Justify::Center),
                    Anchor::BOTTOM_CENTER,
                    Transform::from_translation(Vec3::new(0.0, 0.0, 5.0)),
                    Visibility::Hidden,
                    OperatorHeader { col },
                ));
            }
        }
        return; // place + reveal them next frame
    }

    // Wordmarks resolve over the back half of the flight.
    let alpha = ((ov.t - 0.45) / 0.55).clamp(0.0, 1.0);
    // 22 CSS px tall, mapped to world meters via the frozen camera scale.
    let scale = (22.0 * layout.mpp / 64.0).max(f32::EPSILON);
    for (h, mut tf, mut text, mut color, mut vis) in &mut q {
        let Some(cl) = layout.get(h.col) else {
            *vis = Visibility::Hidden; // toggled-off layer has no lane — hide its header
            continue;
        };
        tf.translation = cl.header_pos;
        tf.scale = Vec3::splat(scale);
        text.0 = format!("{}\n{}", h.col.label(), fmt_count(cl.count));
        let mut c = h.col.color();
        c.set_alpha(alpha);
        color.0 = c;
        *vis = if ov.t > 0.01 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Swap the fixed-chip material per view state: a solid operator-color fill in
/// the tower (so the stacks read as bold magnitude bars), and the branded
/// wordmark texture on the resting map (the "DOT"/"ALPR"/"CCTV" markers). Edge-
/// triggered, so it costs nothing except on the two state changes.
pub fn operators_chip_material(
    ov: Res<OperatorsView>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    q: Query<&OperatorMesh>,
    // (last solid state, last dissolve alpha) — so we act on the texture edge *and* on
    // every frame the dissolve is mid-fade.
    mut last: Local<(Option<bool>, f32)>,
) {
    // Drive the icon→chip morph off animation *progress*: the marks keep their map
    // appearance through the first half of the flight, then swap to solid chips at the
    // midpoint (`CHIP_MORPH_T`) — with a brief alpha dissolve around that instant so the
    // wordmark→square change cross-fades instead of popping.
    let solid = ov.t >= CHIP_MORPH_T;
    let dim = morph_dissolve(ov.t);
    let (last_solid, last_dim) = *last;
    // Outside the dissolve window `dim` is a constant 1.0, so this still no-ops every
    // frame except the swap edge; inside it we update per-frame to run the fade.
    if last_solid == Some(solid) && (dim - last_dim).abs() < 1e-3 {
        return;
    }
    *last = (Some(solid), dim);
    for om in &q {
        if let Some(mat) = materials.get_mut(&om.material) {
            if solid {
                mat.texture = None; // solid fill → punchy tower
                mat.color = om.col.color();
            } else {
                mat.texture = Some(om.branded.clone()); // branded wordmark on the map
                mat.color = Color::WHITE;
            }
        }
        // ALPR stratification: in the tower, paint each chip its maker tint (white
        // material × per-vertex maker color); on the map, restore white so the
        // branded owls aren't tinted. Other columns carry no `maker_colors`.
        if let Some(tints) = &om.maker_colors {
            if let Some(mesh) = meshes.get_mut(&om.mesh) {
                let colors: Vec<[f32; 4]> = if solid {
                    tints.clone()
                } else {
                    vec![[1.0, 1.0, 1.0, 1.0]; tints.len()]
                };
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
            }
            // In the tower the maker tints carry the color, so keep the material white
            // (the uniform ALPR red would otherwise multiply the bands away).
            if solid {
                if let Some(mat) = materials.get_mut(&om.material) {
                    mat.color = Color::WHITE;
                }
            }
        }
        // Apply the dissolve dim last, on whatever base color this chip ended up with.
        if let Some(mat) = materials.get_mut(&om.material) {
            mat.color.set_alpha(dim);
        }
    }
}

/// Same idea for the mobile chips: peds (glasses) → solid META slate and buses →
/// solid MTA steel in the tower, restoring their moving-icon texture on the map.
/// (Vehicles are already a solid clay that matches RIDESHARE, so they're left be.)
pub fn operators_mobile_material(
    ov: Res<OperatorsView>,
    pool: Res<AgentPool>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut last: Local<(Option<bool>, f32)>,
) {
    // Same midpoint morph + dissolve as the fixed chips (see `operators_chip_material`):
    // peds and buses keep their moving icons through the first half of the flight, then
    // cross-dissolve to solid META/MTA chips around `CHIP_MORPH_T`.
    let solid = ov.t >= CHIP_MORPH_T;
    let dim = morph_dissolve(ov.t);
    let (last_solid, last_dim) = *last;
    if last_solid == Some(solid) && (dim - last_dim).abs() < 1e-3 {
        return;
    }
    *last = (Some(solid), dim);
    for (mat_handle, icon, chip) in [
        (&pool.ped_mat, &pool.glasses_icon, OperatorCol::Meta),
        (&pool.bus_mat, &pool.bus_icon, OperatorCol::Mta),
    ] {
        if let Some(m) = materials.get_mut(mat_handle) {
            if solid {
                m.texture = None;
                m.color = chip.color();
            } else {
                m.texture = Some(icon.clone());
                m.color = Color::WHITE;
            }
            m.color.set_alpha(dim);
        }
    }
}

/// Recede the map (streets, ACE corridors, route visuals) as the sensors lift off
/// it, so the towers read against a quiet ground. These layers are opaque, so an
/// absolute alpha is exact. Only writes when the fade level actually changes.
pub fn operators_fade_scene(
    ov: Res<OperatorsView>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    q: Query<&MeshMaterial2d<ColorMaterial>, Or<(With<BaseMap>, With<AceVis>, With<OutlineVis>, With<BuildingVis>, With<LandmarkVis>, With<RouteVis>)>>,
    mut last: Local<f32>,
) {
    let target = 1.0 - ov.t * 0.85;
    if (target - *last).abs() < 1e-3 {
        return;
    }
    *last = target;
    for mm in &q {
        if let Some(mat) = materials.get_mut(&mm.0) {
            mat.color.set_alpha(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn alpr_column_labeled_alpr_not_flock() {
        // The ALPR layer is labeled by the generic category, not the Flock brand.
        assert_eq!(OperatorCol::Flock.label(), "ALPR");
        assert!(!OperatorCol::Flock.label().contains("FLOCK"));
    }

    #[test]
    fn maker_classify_maps_brands_and_aliases() {
        assert_eq!(Maker::classify(Some("Flock Safety")), Maker::Flock);
        assert_eq!(Maker::classify(Some("flock")), Maker::Flock);
        assert_eq!(Maker::classify(Some("Leonardo")), Maker::Leonardo);
        assert_eq!(Maker::classify(Some("ELSAG")), Maker::Leonardo); // Leonardo's brand
        assert_eq!(Maker::classify(Some("Redspeed")), Maker::Redspeed);
        assert_eq!(Maker::classify(Some("Ekin")), Maker::Ekin);
        assert_eq!(Maker::classify(Some("Mav Systems")), Maker::Mav);
        assert_eq!(Maker::classify(Some("Genetec AutoVu")), Maker::Genetec);
        assert_eq!(Maker::classify(Some("Vigilant Solutions")), Maker::Motorola);
        assert_eq!(Maker::classify(Some("some other vendor")), Maker::Other);
        assert_eq!(Maker::classify(None), Maker::Other);
    }

    #[test]
    fn maker_colors_are_distinct() {
        // Every banded maker needs a visually separable tint, or the strata blur.
        for (i, &a) in MAKERS.iter().enumerate() {
            for &b in &MAKERS[i + 1..] {
                assert_ne!(a.rgba(), b.rgba(), "{a:?} and {b:?} share a color");
            }
        }
    }

    #[test]
    fn present_columns_drops_empty_layers() {
        let mut counts = vec![0usize; COLS.len()];
        counts[OperatorCol::Cctv.index()] = 100;
        counts[OperatorCol::Tesla.index()] = 5;
        // Only the two non-empty layers get lanes, in stable COLS order.
        let idxs: Vec<usize> = present_columns(&counts).iter().map(|c| c.index()).collect();
        assert_eq!(idxs, vec![OperatorCol::Cctv.index(), OperatorCol::Tesla.index()]);
        // Everything off → no lanes at all.
        assert!(present_columns(&vec![0usize; COLS.len()]).is_empty());
    }

    #[test]
    fn ease_endpoints() {
        assert!(approx(ease_in_out_cubic(0.0), 0.0));
        assert!(approx(ease_in_out_cubic(1.0), 1.0));
        assert!(approx(ease_in_out_cubic(0.5), 0.5));
    }

    #[test]
    fn chip_progress_termini() {
        // Every chip is home at t=0 and landed at t=1, regardless of stagger.
        for &n in &[1usize, 7, 4422] {
            for &i in &[0usize, n / 2, n.saturating_sub(1)] {
                assert!(approx(chip_progress(0.0, OperatorCol::Cctv, i, n, false), 0.0));
                assert!(approx(chip_progress(1.0, OperatorCol::Cctv, i, n, false), 1.0));
            }
        }
    }

    #[test]
    fn chip_progress_reduced_snaps() {
        assert_eq!(chip_progress(0.5, OperatorCol::Dot, 3, 10, true), 0.0);
        assert_eq!(chip_progress(0.99, OperatorCol::Dot, 3, 10, true), 0.0);
        assert_eq!(chip_progress(1.0, OperatorCol::Dot, 3, 10, true), 1.0);
    }

    #[test]
    fn arc_lerp_termini() {
        let home = Vec2::new(10.0, -5.0);
        let target = Vec2::new(-200.0, 800.0);
        assert!(arc_lerp(home, target, 0.0).abs_diff_eq(home, 1e-3));
        assert!(arc_lerp(home, target, 1.0).abs_diff_eq(target, 1e-3));
    }

    #[test]
    fn slot_grid_wraps_bottom_first() {
        let cl = ColLayout {
            col: OperatorCol::Cctv,
            lane_cx: 0.0,
            base_y: 0.0,
            per_row: 4,
            pitch: 10.0,
            count: 16,
            header_pos: Vec3::ZERO,
        };
        // Bottom row: 4 across, centered on lane_cx=0 (row width 40) → first at -15.
        assert!(approx(cl.slot(0).x, -15.0));
        assert!(approx(cl.slot(0).y, 5.0));
        // Wraps to the next row up after `per_row`.
        assert!(approx(cl.slot(4).x, -15.0));
        assert!(approx(cl.slot(4).y, 15.0));
    }

    #[test]
    fn fit_pitch_fills_without_overflow() {
        let (lane_w, avail_h) = (120.0_f32, 800.0_f32);
        let p = fit_pitch(4422.0, lane_w, avail_h);
        assert!(p > 0.0);
        let rows = |pitch: f32| ((4422.0_f32) / (lane_w / pitch).floor().max(1.0)).ceil() * pitch;
        assert!(rows(p) <= avail_h + 1e-3, "tower {} should fit {avail_h}", rows(p));
        // A meaningfully larger pitch overflows → p is near-maximal (a tight fill).
        assert!(rows(p * 1.15) > avail_h, "pitch {} should overflow", p * 1.15);
    }

    #[test]
    fn fit_pitch_more_chips_smaller_pitch() {
        let (lane_w, avail_h) = (120.0_f32, 800.0_f32);
        assert!(fit_pitch(4422.0, lane_w, avail_h) < fit_pitch(370.0, lane_w, avail_h));
    }
}
