//! Roving-coverage overlay — "watch the street network light up."
//!
//! A self-contained takeover that replays one simulated day in ~60 seconds and
//! accumulates how often each street segment is crossed by a roving camera
//! (rideshare dashcam or ACE bus). Each segment is drawn thicker and hotter the
//! more times it is traversed, so the streets the moving fleet covers *most*
//! glow brightest — the spatiotemporal-density idiom from MIT Senseable City's
//! urban-sensing work, applied to our fleet.
//!
//! The agents follow free-form route polylines, not graph edges, so we map their
//! motion back onto the street graph live: each frame we sub-sample the distance
//! a vehicle covered and snap each sample to the nearest edge (via a coarse
//! spatial grid), incrementing that edge once per fresh entry.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::agents::{AgentClass, MobileAgent};
use crate::operators::{OperatorMesh, OperatorsView};
use crate::{world, BaseMap, BuildingVis, FovWedge, LinkNycVis, Sim, SimClock};
use sim_core::Vec2 as Enu;

/// One simulated day in ~30 real seconds (86 400 s / 30 = 2880× wall clock).
const COVERAGE_RATE: f64 = 2880.0;
/// Sub-sample step (m) when walking a vehicle's per-frame travel onto edges.
const SAMPLE_M: f64 = 16.0;
/// Ignore a sample farther than this from any street (off-graph noise).
const SNAP_MAX_M: f64 = 35.0;
/// Spatial-grid cell size (m) for the nearest-edge lookup.
const GRID_CELL: f64 = 60.0;
/// Traversal count that maps to full heat/width (log-scaled below it). High
/// enough that a full day's busy avenues stay distinctly hotter than the side
/// streets instead of everything saturating to max.
const REF_COUNT: f32 = 75.0;
/// Ribbon half-width (world m) at zero vs. full coverage.
const MIN_HALF: f32 = 4.0;
const MAX_HALF: f32 = 28.0;
/// Painter depth: above streets/outline/ACE/bridges, below markers (hidden anyway).
const COVERAGE_Z: f32 = 0.6;
/// How faint the base streets fade to while the overlay runs.
const BASE_DIM_ALPHA: f32 = 0.22;
/// Rebuild the (growing) coverage mesh every N frames while playing (~10 Hz).
const REBUILD_EVERY: u32 = 6;
/// Fixed overview framing (ENU center + m/px) the overlay flies to on launch —
/// midtown-through-downtown, where the roving fleet is densest.
const VIEW_CX: f32 = -400.0;
const VIEW_CY: f32 = -2800.0;
const VIEW_SCALE: f32 = 8.5;

/// Lifecycle state for the overlay (mirrors `OperatorsView`: one of these
/// takeovers is active at a time).
#[derive(Resource, Default)]
pub struct CoverageView {
    pub active: bool,
    /// The day finished replaying; hold the final image until dismissed.
    pub finished: bool,
    /// Set by the Replay control to restart the day in place (already active).
    pub restart: bool,
    /// Simulated hours elapsed in this run (single 0→24 pass, no wrap).
    pub elapsed_h: f64,
    saved_time: f64,
    saved_rate: f64,
    saved_playing: bool,
    saved_cam: Option<(Vec3, f32)>,
}

/// Per-edge traversal tally plus the overlay's mesh handle.
#[derive(Resource, Default)]
pub struct Coverage {
    /// One count per graph edge (index matches `GraphAsset.edges`).
    pub counts: Vec<f32>,
    pub max: f32,
    /// Number of edges with at least one traversal.
    pub covered: usize,
    dirty: bool,
    frame: u32,
    mesh: Option<Handle<Mesh>>,
    /// The last edge each agent was snapped to, so a slow crossing counts once.
    last_edge: HashMap<Entity, u32>,
}

/// Coarse uniform grid mapping a cell to the edges that pass through it.
#[derive(Resource, Default)]
pub struct EdgeGrid {
    built: bool,
    cell: f64,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

/// Tags the single entity that renders the accumulating coverage ribbons.
#[derive(Component)]
pub struct CoverageVis;

// ---- geometry helpers -------------------------------------------------------

fn point_seg_dist2(p: Enu, a: Enu, b: Enu) -> f64 {
    let (abx, aby) = (b.x - a.x, b.y - a.y);
    let (apx, apy) = (p.x - a.x, p.y - a.y);
    let ab2 = abx * abx + aby * aby;
    let t = if ab2 > 0.0 {
        ((apx * abx + apy * aby) / ab2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (a.x + abx * t, a.y + aby * t);
    let (dx, dy) = (p.x - cx, p.y - cy);
    dx * dx + dy * dy
}

fn point_polyline_dist2(p: Enu, poly: &[[f64; 2]]) -> f64 {
    if poly.len() == 1 {
        let d = p.sub(Enu::new(poly[0][0], poly[0][1]));
        return d.x * d.x + d.y * d.y;
    }
    let mut best = f64::INFINITY;
    for w in poly.windows(2) {
        let d2 = point_seg_dist2(p, Enu::new(w[0][0], w[0][1]), Enu::new(w[1][0], w[1][1]));
        if d2 < best {
            best = d2;
        }
    }
    best
}

/// Bucket every edge into the grid cells its polyline passes through.
fn build_edge_grid(graph: &sim_core::StreetGraph) -> EdgeGrid {
    let cell = GRID_CELL;
    let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (i, e) in graph.asset().edges.iter().enumerate() {
        let id = i as u32;
        for w in e.polyline.windows(2) {
            let a = Enu::new(w[0][0], w[0][1]);
            let b = Enu::new(w[1][0], w[1][1]);
            let steps = ((a.distance(b) / (cell * 0.6)).ceil() as usize).max(1);
            for k in 0..=steps {
                let p = a.lerp(b, k as f64 / steps as f64);
                cells
                    .entry(((p.x / cell).floor() as i32, (p.y / cell).floor() as i32))
                    .or_default()
                    .push(id);
            }
        }
    }
    for v in cells.values_mut() {
        v.sort_unstable();
        v.dedup();
    }
    EdgeGrid {
        built: true,
        cell,
        cells,
    }
}

/// Nearest street edge to `p` within `SNAP_MAX_M`, or `None` if off-graph.
fn snap_edge(p: Enu, grid: &EdgeGrid, graph: &sim_core::StreetGraph) -> Option<u32> {
    let (cx, cy) = ((p.x / grid.cell).floor() as i32, (p.y / grid.cell).floor() as i32);
    let mut best = None;
    let mut best_d2 = SNAP_MAX_M * SNAP_MAX_M;
    for dx in -1..=1 {
        for dy in -1..=1 {
            let Some(es) = grid.cells.get(&(cx + dx, cy + dy)) else {
                continue;
            };
            for &e in es {
                let d2 = point_polyline_dist2(p, &graph.asset().edges[e as usize].polyline);
                if d2 < best_d2 {
                    best_d2 = d2;
                    best = Some(e);
                }
            }
        }
    }
    best
}

/// Pale-amber → orange → crimson heat ramp; alpha rises with coverage.
fn ramp(norm: f32) -> [f32; 4] {
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let lo = [0.97, 0.82, 0.40];
    let mid = [0.93, 0.46, 0.13];
    let hi = [0.65, 0.09, 0.07];
    let rgb = if norm < 0.5 {
        let t = norm / 0.5;
        [
            lerp(lo[0], mid[0], t),
            lerp(lo[1], mid[1], t),
            lerp(lo[2], mid[2], t),
        ]
    } else {
        let t = (norm - 0.5) / 0.5;
        [
            lerp(mid[0], hi[0], t),
            lerp(mid[1], hi[1], t),
            lerp(mid[2], hi[2], t),
        ]
    };
    [rgb[0], rgb[1], rgb[2], 0.45 + 0.5 * norm]
}

// ---- systems ----------------------------------------------------------------

/// Lifecycle + the single-day fast clock. Detects the activate/deactivate edges,
/// resets state, frames the overview, and advances `time_of_day` 0→24 once.
pub fn coverage_drive(
    time: Res<Time>,
    mut clock: ResMut<SimClock>,
    mut cov_view: ResMut<CoverageView>,
    mut cov: ResMut<Coverage>,
    mut grid: ResMut<EdgeGrid>,
    sim: Option<Res<Sim>>,
    mut cam_q: Query<&mut Transform, With<Camera2d>>,
    mut ov: ResMut<OperatorsView>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut was_active: Local<bool>,
) {
    let active = cov_view.active;

    // ---- rising edge: set up the run ----
    if active && !*was_active {
        let Some(sim) = sim.as_ref() else {
            // World not loaded yet; cancel the launch.
            cov_view.active = false;
            return;
        };
        ov.active = false; // the two takeovers are mutually exclusive

        if !grid.built {
            *grid = build_edge_grid(&sim.drive_graph);
        }
        let n = sim.drive_graph.edge_count();
        cov.counts = vec![0.0; n];
        cov.max = 0.0;
        cov.covered = 0;
        cov.dirty = true;
        cov.frame = 0;
        cov.last_edge.clear();

        if cov.mesh.is_none() {
            let handle = meshes.add(world::colored_ribbon_mesh(&[], &[], &[], 0.0));
            let mat = materials.add(ColorMaterial {
                color: Color::WHITE,
                ..default()
            });
            commands.spawn((
                Mesh2d(handle.clone()),
                MeshMaterial2d(mat),
                Transform::from_xyz(0.0, 0.0, COVERAGE_Z),
                CoverageVis,
            ));
            cov.mesh = Some(handle);
        }

        cov_view.saved_time = clock.time_of_day;
        cov_view.saved_rate = clock.rate;
        cov_view.saved_playing = clock.playing;
        clock.time_of_day = 0.0;
        clock.rate = COVERAGE_RATE;
        clock.playing = true;
        cov_view.elapsed_h = 0.0;
        cov_view.finished = false;

        if let Ok(mut t) = cam_q.single_mut() {
            cov_view.saved_cam = Some((t.translation, t.scale.x));
            t.translation.x = VIEW_CX;
            t.translation.y = VIEW_CY;
            t.scale = Vec3::splat(VIEW_SCALE);
        }
    }

    // ---- replay request while already running: reset counts + clock in place ----
    if active && *was_active && cov_view.restart {
        if let Some(sim) = sim.as_ref() {
            cov.counts = vec![0.0; sim.drive_graph.edge_count()];
            cov.max = 0.0;
            cov.covered = 0;
            cov.dirty = true;
            cov.frame = 0;
            cov.last_edge.clear();
            clock.time_of_day = 0.0;
            clock.rate = COVERAGE_RATE;
            clock.playing = true;
            cov_view.elapsed_h = 0.0;
            cov_view.finished = false;
        }
    }
    cov_view.restart = false;

    // ---- falling edge: restore the prior clock + framing ----
    if !active && *was_active {
        clock.time_of_day = cov_view.saved_time;
        clock.rate = cov_view.saved_rate;
        clock.playing = cov_view.saved_playing;
        if let Some((tr, sc)) = cov_view.saved_cam.take() {
            if let Ok(mut t) = cam_q.single_mut() {
                t.translation = tr;
                t.scale = Vec3::splat(sc);
            }
        }
        cov_view.finished = false;
    }

    // ---- advance the one-day clock while playing ----
    if active && clock.playing && !cov_view.finished {
        let dh = clock.rate * time.delta_secs_f64() / 3600.0;
        cov_view.elapsed_h += dh;
        if cov_view.elapsed_h >= 24.0 {
            clock.time_of_day = 24.0 - 1e-6;
            clock.playing = false;
            cov_view.finished = true;
            // Confirm highways (FDR/West Side Hwy, rw_type 2) actually got covered.
            if let Some(sim) = sim.as_ref() {
                let edges = &sim.drive_graph.asset().edges;
                let (mut hw_cov, mut hw_tot) = (0usize, 0usize);
                for (i, e) in edges.iter().enumerate() {
                    // Highway = CSCL rw_type 2, decoded from the packed segment_id.
                    if sim_core::graph::unpack_class(e.segment_id).0 == 2 {
                        hw_tot += 1;
                        if cov.counts.get(i).copied().unwrap_or(0.0) > 0.0 {
                            hw_cov += 1;
                        }
                    }
                }
                info!(
                    "coverage finished: {hw_cov}/{hw_tot} highway edges covered, {} segments total",
                    cov.covered
                );
            }
        } else {
            clock.time_of_day = cov_view.elapsed_h;
        }
    }

    *was_active = active;
}

/// Snap each roving camera's per-frame travel onto street edges, incrementing a
/// segment once per fresh entry.
pub fn coverage_accumulate(
    time: Res<Time>,
    clock: Res<SimClock>,
    cov_view: Res<CoverageView>,
    mut cov: ResMut<Coverage>,
    grid: Res<EdgeGrid>,
    sim: Option<Res<Sim>>,
    agents: Query<(Entity, &MobileAgent)>,
) {
    if !cov_view.active || cov_view.finished || !clock.playing || !grid.built {
        return;
    }
    let Some(sim) = sim.as_ref() else {
        return;
    };
    let sim_dt = time.delta_secs_f64() * clock.rate;
    if sim_dt <= 0.0 {
        return;
    }

    for (ent, a) in agents.iter() {
        if !a.active || !matches!(a.class, AgentClass::Vehicle | AgentClass::Bus) {
            continue;
        }
        let moved = (a.speed_mps * sim_dt).max(0.0);
        let end = a.progress_m;
        let start = (end - moved).max(0.0);
        let steps = ((moved / SAMPLE_M).ceil() as usize).clamp(1, 256);
        let mut prev = cov.last_edge.get(&ent).copied();
        for k in 1..=steps {
            let d = start + (end - start) * (k as f64 / steps as f64);
            if let Some(e) = snap_edge(a.route.position_at(d), &grid, &sim.drive_graph) {
                if Some(e) != prev {
                    let idx = e as usize;
                    let was_zero = cov.counts[idx] == 0.0;
                    cov.counts[idx] += 1.0;
                    if was_zero {
                        cov.covered += 1;
                    }
                    let v = cov.counts[idx];
                    if v > cov.max {
                        cov.max = v;
                    }
                    prev = Some(e);
                }
            }
        }
        if let Some(e) = prev {
            cov.last_edge.insert(ent, e);
        }
        cov.dirty = true;
    }
}

/// Rebuild the coverage ribbon mesh (throttled) and toggle its visibility.
pub fn coverage_render(
    cov_view: Res<CoverageView>,
    mut cov: ResMut<Coverage>,
    sim: Option<Res<Sim>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut vis: Query<&mut Visibility, With<CoverageVis>>,
) {
    let want = if cov_view.active {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut vis {
        if *v != want {
            *v = want;
        }
    }
    if !cov_view.active {
        return;
    }
    let Some(sim) = sim.as_ref() else {
        return;
    };
    let Some(handle) = cov.mesh.clone() else {
        return;
    };

    cov.frame = cov.frame.wrapping_add(1);
    if !cov.dirty {
        return;
    }
    // Always repaint the final frame; otherwise throttle to ~10 Hz.
    if !cov_view.finished && cov.frame % REBUILD_EVERY != 0 {
        return;
    }
    cov.dirty = false;

    let edges = &sim.drive_graph.asset().edges;
    let denom = (1.0 + REF_COUNT).ln();
    let mut segs: Vec<[Enu; 2]> = Vec::with_capacity(cov.covered);
    let mut halfs: Vec<f32> = Vec::with_capacity(cov.covered);
    let mut cols: Vec<[f32; 4]> = Vec::with_capacity(cov.covered);
    for (i, &count) in cov.counts.iter().enumerate() {
        if count <= 0.0 {
            continue;
        }
        let norm = ((1.0 + count).ln() / denom).clamp(0.0, 1.0);
        let e = &edges[i];
        segs.push([sim.drive_graph.node_pos(e.from), sim.drive_graph.node_pos(e.to)]);
        halfs.push(MIN_HALF + (MAX_HALF - MIN_HALF) * norm);
        cols.push(ramp(norm));
    }
    let mesh = world::colored_ribbon_mesh(&segs, &halfs, &cols, 0.0);
    if let Some(m) = meshes.get_mut(&handle) {
        *m = mesh;
    }
}

/// Dim the base map and hide the static clutter so the coverage glow reads. Runs
/// after `sync_visibility`, overriding it only while the overlay is up; on the
/// closing edge it hands control back.
pub fn coverage_scene(
    cov_view: Res<CoverageView>,
    mut was: Local<bool>,
    base: Query<&MeshMaterial2d<ColorMaterial>, With<BaseMap>>,
    mut mats: ResMut<Assets<ColorMaterial>>,
    mut sets: ParamSet<(
        Query<&mut Visibility, With<OperatorMesh>>,
        Query<&mut Visibility, With<FovWedge>>,
        Query<&mut Visibility, With<BuildingVis>>,
        Query<&mut Visibility, With<LinkNycVis>>,
        Query<(&mut Visibility, &MobileAgent)>,
    )>,
) {
    let active = cov_view.active;
    if active {
        for mh in &base {
            if let Some(m) = mats.get_mut(&mh.0) {
                use bevy::color::Alpha;
                if (m.color.alpha() - BASE_DIM_ALPHA).abs() > 0.001 {
                    m.color.set_alpha(BASE_DIM_ALPHA);
                }
            }
        }
        for mut v in &mut sets.p0() {
            *v = Visibility::Hidden;
        }
        for mut v in &mut sets.p1() {
            *v = Visibility::Hidden;
        }
        for mut v in &mut sets.p2() {
            *v = Visibility::Hidden;
        }
        for mut v in &mut sets.p3() {
            *v = Visibility::Hidden;
        }
        for (mut v, a) in &mut sets.p4() {
            let show = a.active && matches!(a.class, AgentClass::Vehicle | AgentClass::Bus);
            *v = if show {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    } else if *was {
        // Closing: restore the base alpha and let sync_visibility re-gate the rest.
        for mh in &base {
            if let Some(m) = mats.get_mut(&mh.0) {
                use bevy::color::Alpha;
                m.color.set_alpha(1.0);
            }
        }
        for mut v in &mut sets.p0() {
            *v = Visibility::Inherited;
        }
        for mut v in &mut sets.p1() {
            *v = Visibility::Inherited;
        }
        for mut v in &mut sets.p2() {
            *v = Visibility::Inherited;
        }
        for mut v in &mut sets.p3() {
            *v = Visibility::Inherited;
        }
    }
    *was = active;
}
