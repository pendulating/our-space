//! Ambient moving sensing agents: rideshare **dashcam vehicles** (Tier C,
//! following real TLC trip-O-D routes) and smart-glasses **pedestrians** (Tier D,
//! wandering via graph random walks). These are a *visualization* of the mobile
//! sensing layer; the deterministic analytical estimate is untouched. In
//! `ExposureMode::Narrative`, agents that pass the walker increment a live,
//! stochastic "saw you" tally — a Monte-Carlo sample of the same model.
//!
//! Performance: a fixed entity pool (no runtime spawn/despawn), one shared
//! mesh+material per class (draw-call batching), O(log n) `position_at` per agent
//! per frame, no runtime A* (vehicles replay baked polylines; peds random-walk).

use bevy::prelude::*;
use std::f64::consts::FRAC_PI_2;

use std::sync::Arc;

use sim_core::assets::{BusTrip, TaxiTrip};
use sim_core::math::point_segment_distance;
use sim_core::rng::{RngLike, WyRand};
use sim_core::{Route, Vec2};

use crate::operators::{OperatorSlot, OperatorsView};
use crate::{world, ExposureMode, Mode, Params, RouteState, Sim, SimClock, Walker, WalkLive};

// Fixed caps → provable worst-case per-frame cost on single-threaded WASM.
// `replay_agents` viewport-culls: only trips whose route passes through the visible
// rect are admitted, so this pool bounds the *on-screen* taxi count, not the day's
// global peak. 6000 covers a full-city zoom-out (where 13k+ trips are concurrent but
// merge into a blob and subsample harmlessly); zoomed into any neighborhood, every
// in-view taxi shows. Manhattan's true peak (~3,823) fits with headroom.
const MAX_VEHICLES: usize = 6000;
const MAX_PEDS: usize = 400;

// Buses are NOT viewport-culled (the bus loop full-scans the timetable), so this cap is
// the only limit on the on-screen ACE fleet. Sized above true peak concurrency (~1,074
// at the evening rush) with headroom, so every active bus renders and the operator
// stack / neighborhood / coverage tallies show the true layer — not a 300-cap sample.
const MAX_BUSES: usize = 2500;
/// Upper bound on taxi trip duration (minutes). Used to seek the start of the
/// currently-active window via binary search on a viewport/scrub rebuild, so the
/// re-scan is O(active) rather than O(whole day). Generous — real FHV trips are well
/// under this.
const MAX_TRIP_DUR_MIN: f32 = 240.0;
/// Speculative sidewalk delivery robots; modest on-screen cap (rare even at peak).
const MAX_ROBOTS: usize = 200;
/// Teslas on-screen cap (NYC has ~29k private Teslas; this bounds the sample).
const MAX_TESLAS: usize = 400;

// Pedestrian walk speed (m/s); the master `SimClock.rate` time-lapse applies on
// top. Vehicles + buses no longer use a fixed speed — they replay real schedules.
const PED_SPEED_MPS: f64 = 1.34;
/// Sidewalk robots crawl (~1.8 m/s ≈ 4 mph, the Serve/Starship class limit).
const ROBOT_SPEED_MPS: f64 = 1.8;
/// Teslas move at the replay taxis' effective Manhattan pace (median ~3.3 m/s, mean
/// ~3.6 over the day's real trips) so the two car classes flow together rather than
/// the Teslas zipping past traffic. (Fixed speed: Teslas are synthetic agents, not
/// real timed trips.)
const TESLA_SPEED_MPS: f64 = 3.5;

// Bus stop-and-go: each inter-stop segment ends with a **dwell** (the bus holds at
// the upcoming stop until its scheduled departure), so buses visibly pause. The dwell
// is `BUS_DWELL_MIN` minutes, capped at `BUS_DWELL_FRAC` of the segment so it never
// eats a whole short hop; the travel before it is smoothstep-eased (decelerate into /
// accelerate out of stops). Tunable.
const BUS_DWELL_MIN: f32 = 0.25; // minutes (~15 s of schedule time)
const BUS_DWELL_FRAC: f32 = 0.4; // ≤ 40% of any one inter-stop segment

// Capture reach (m) for the narrative "passed you" test.
const VEHICLE_CAPTURE_R: f64 = 22.0;
const PED_CAPTURE_R: f64 = 6.0;
const BUS_CAPTURE_R: f64 = 22.0;
/// Robot nav cameras are low + close-range: a short curb-side reach.
const ROBOT_CAPTURE_R: f64 = 8.0;
/// Tesla 360° cameras (Sentry/Autopilot) reach across the street.
const TESLA_CAPTURE_R: f64 = 20.0;
/// Max plausible single-frame travel for the swept capture test. A step longer than
/// this is a slot reassignment / mode-switch teleport (stale `prev_pos`), not real
/// motion, so we fall back to a point test rather than sweep across the map.
const MAX_SWEEP_M: f64 = 600.0;

const PED_WALK_EDGES: usize = 16;
const ACTIVATIONS_PER_FRAME: usize = 32;

// The ambient agents now move on the master `SimClock.rate` (sim-seconds per
// real-second) so the day and the traffic stay congruent; the rate is tweakable
// from the time panel and pausing the clock freezes them too.

// z-order: streets 0.0, cameras 1.0, route line 2.0, agents 2.4–2.7, landmark massing
// 2.8 (occludes agents/cameras behind it), A/B markers 3.0, walker 4.0, labels 5.0.
const Z_VEHICLE: f32 = 2.6;
const Z_PED: f32 = 2.4;
const Z_BUS: f32 = 2.7;
const Z_ROBOT: f32 = 2.5;
const Z_TESLA: f32 = 2.55;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentClass {
    Vehicle,
    Pedestrian,
    /// ACE camera-enforcement bus (Tier A) running a real GTFS route shape.
    /// Schedule-simulated today; the agent is position-drivable, so a future
    /// realtime (GTFS-rt) source could set bus transforms directly instead.
    Bus,
    /// Speculative sidewalk delivery robot (Tier D): walks the sidewalk graph,
    /// spawned weighted by the Robotability Score field.
    DeliveryRobot,
    /// Tesla (Tier C): always-on Sentry/Autopilot cameras, spawned weighted by
    /// private-registration density (the Tesla field).
    Tesla,
}

#[derive(Component)]
pub struct MobileAgent {
    pub class: AgentClass,
    pub route: Arc<Route>,
    pub progress_m: f64,
    pub speed_mps: f64,
    pub active: bool,
    pub flash: f32,
    /// Debounce: already counted the walker this pass (narrative mode).
    pub counted: bool,
    /// World position one capture-frame ago, for the swept closest-approach test
    /// (so a fast agent can't tunnel past the capture radius between frames).
    pub prev_pos: Vec2,
    /// Monotone segment cursor for O(1) amortized position+heading lookups.
    pub seg_hint: usize,
}

impl MobileAgent {
    fn idle(class: AgentClass) -> Self {
        MobileAgent {
            class,
            route: Arc::new(Route::from_points(Vec::new())),
            progress_m: 0.0,
            speed_mps: 0.0,
            active: false,
            flash: 0.0,
            counted: false,
            prev_pos: Vec2::ZERO,
            seg_hint: 0,
        }
    }
}

/// Pool of pre-spawned agent entities + weighted vehicle-route sampler + RNG.
#[derive(Resource)]
pub struct AgentPool {
    pub vehicles: Vec<Entity>,
    pub peds: Vec<Entity>,
    pub buses: Vec<Entity>,
    pub robots: Vec<Entity>,
    pub teslas: Vec<Entity>,
    rng: WyRand,
    target_peds: usize,
    target_robots: usize,
    target_teslas: usize,
    // Tracked active counts for the count-scaled classes (peds + robots + teslas;
    // vehicles + buses are schedule-driven by `replay_agents`). Set in `reconcile`.
    active_peds: usize,
    active_robots: usize,
    active_teslas: usize,
    // Shared per-class materials + icon textures, exposed so the Operators view can
    // swap the mobile chips to a solid operator fill in the tower (and restore the
    // moving icon on the map). Robots never fly to a tower, so theirs isn't kept.
    pub ped_mat: Handle<ColorMaterial>,
    pub bus_mat: Handle<ColorMaterial>,
    pub veh_mat: Handle<ColorMaterial>,
    pub tesla_mat: Handle<ColorMaterial>,
    pub glasses_icon: Handle<Image>,
    pub bus_icon: Handle<Image>,
    pub taxi_icon: Handle<Image>,
    pub tesla_icon: Handle<Image>,
}

impl AgentPool {
    /// Placeholder inserted at startup so the agent systems always have the
    /// resource; replaced by [`spawn_pool`] once `build_world` runs.
    pub fn empty() -> Self {
        AgentPool {
            vehicles: Vec::new(),
            peds: Vec::new(),
            buses: Vec::new(),
            robots: Vec::new(),
            teslas: Vec::new(),
            rng: WyRand::new(0x9E37_79B9_7F4A_7C15),
            target_peds: 0,
            target_robots: 0,
            target_teslas: 0,
            active_peds: 0,
            active_robots: 0,
            active_teslas: 0,
            ped_mat: Handle::default(),
            bus_mat: Handle::default(),
            veh_mat: Handle::default(),
            tesla_mat: Handle::default(),
            glasses_icon: Handle::default(),
            bus_icon: Handle::default(),
            taxi_icon: Handle::default(),
            tesla_icon: Handle::default(),
        }
    }
}

/// The five top-down agent icons (drawn nose-up; `position_agents` rotates each
/// along its travel heading), loaded once in `setup` and handed to [`spawn_pool`].
pub struct AgentIcons {
    pub taxi: Handle<Image>,
    pub tesla: Handle<Image>,
    pub bus: Handle<Image>,
    pub glasses: Handle<Image>,
    pub robot: Handle<Image>,
}

/// Schedule-driven replay bookkeeping: which baked trip occupies each pooled entity
/// slot. Taxis use a forward cursor over the start-sorted trip list (rebuilt on a
/// clock wrap/scrub); buses admit from two `partition_point`-bounded windows of the
/// same start-sorted list (`now` and the after-midnight `now + 1440`).
#[derive(Resource, Default)]
pub struct ReplayState {
    veh_slot_trip: Vec<i32>, // vehicle slot -> taxi trip idx (-1 = free)
    veh_cursor: usize,       // next taxi trip (start-sorted) to consider
    veh_last_now: f32,       // last frame's minute-of-day (wrap/scrub detection)
    veh_vp: [f64; 3],        // viewport (cx, cy, width) the taxi slots are synced to
    veh_vp_prev: [f64; 3],   // last frame's viewport, for motion-settle detection
    bus_slot_trip: Vec<i32>, // bus slot -> bus trip idx (-1 = free)
    bus_mapped: Vec<bool>,   // bus trip -> currently occupies a slot
    bus_max_span_min: Option<f32>, // widest bus trip (end-start); admit-window lower bound, computed once
}

impl ReplayState {
    pub fn new(_n_taxi: usize, n_bus: usize) -> Self {
        ReplayState {
            veh_slot_trip: vec![-1; MAX_VEHICLES],
            veh_cursor: 0,
            veh_last_now: -1.0,
            veh_vp: [0.0; 3],
            veh_vp_prev: [0.0; 3],
            bus_slot_trip: vec![-1; MAX_BUSES],
            bus_mapped: vec![false; n_bus],
            bus_max_span_min: None,
        }
    }
}

/// Tag for visibility toggling (hidden in heatmap mode / when agents are off).
#[derive(Component)]
pub struct MobileVis;

/// Build the fixed entity pool once (called from `build_world`). All entities
/// start hidden + inactive; `scale_agent_population` activates them.
pub fn spawn_pool(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    icons: AgentIcons,
) -> AgentPool {
    // Every mobile class is a textured quad carrying its top-down icon (drawn
    // nose-up; `position_agents` spins it along travel). The class color lives in
    // the icon's ink, so each material stays WHITE (a tint would multiply it away).
    let icon_mat = |materials: &mut Assets<ColorMaterial>, icon: &Handle<Image>| {
        materials.add(ColorMaterial {
            color: Color::WHITE,
            texture: Some(icon.clone()),
            ..default()
        })
    };
    let veh_mesh = meshes.add(world::merged_icon_quads(&[Vec2::new(0.0, 0.0)], 17.0));
    let veh_mat = icon_mat(materials, &icons.taxi); // rideshare dashcam (amber ink)
    // Quad sizes compensate for how much of the 128px tile each silhouette fills
    // (the person is ~half the tile wide; the bus nearly full height).
    let ped_mesh = meshes.add(world::merged_icon_quads(&[Vec2::new(0.0, 0.0)], 20.0));
    let ped_mat = icon_mat(materials, &icons.glasses);
    let bus_mesh = meshes.add(world::merged_icon_quads(&[Vec2::new(0.0, 0.0)], 30.0));
    let bus_mat = icon_mat(materials, &icons.bus);
    // Delivery robots: violet cargo box (speculative outlier).
    let robot_mesh = meshes.add(world::merged_icon_quads(&[Vec2::new(0.0, 0.0)], 14.0));
    let robot_mat = icon_mat(materials, &icons.robot);
    // Teslas: orange glass-roof car (deeper warning than the rideshare amber).
    let tesla_mat = icon_mat(materials, &icons.tesla);

    let mut vehicles = Vec::with_capacity(MAX_VEHICLES);
    for _ in 0..MAX_VEHICLES {
        vehicles.push(
            commands
                .spawn((
                    Mesh2d(veh_mesh.clone()),
                    MeshMaterial2d(veh_mat.clone()),
                    Transform::from_xyz(0.0, 0.0, Z_VEHICLE),
                    Visibility::Hidden,
                    MobileVis,
                    MobileAgent::idle(AgentClass::Vehicle),
                    OperatorSlot::default(),
                ))
                .id(),
        );
    }
    let mut peds = Vec::with_capacity(MAX_PEDS);
    for _ in 0..MAX_PEDS {
        peds.push(
            commands
                .spawn((
                    Mesh2d(ped_mesh.clone()),
                    MeshMaterial2d(ped_mat.clone()),
                    Transform::from_xyz(0.0, 0.0, Z_PED),
                    Visibility::Hidden,
                    MobileVis,
                    MobileAgent::idle(AgentClass::Pedestrian),
                    OperatorSlot::default(),
                ))
                .id(),
        );
    }

    let mut buses = Vec::with_capacity(MAX_BUSES);
    for _ in 0..MAX_BUSES {
        buses.push(
            commands
                .spawn((
                    Mesh2d(bus_mesh.clone()),
                    MeshMaterial2d(bus_mat.clone()),
                    Transform::from_xyz(0.0, 0.0, Z_BUS),
                    Visibility::Hidden,
                    MobileVis,
                    MobileAgent::idle(AgentClass::Bus),
                    OperatorSlot::default(),
                ))
                .id(),
        );
    }

    let mut robots = Vec::with_capacity(MAX_ROBOTS);
    for _ in 0..MAX_ROBOTS {
        robots.push(
            commands
                .spawn((
                    Mesh2d(robot_mesh.clone()),
                    MeshMaterial2d(robot_mat.clone()),
                    Transform::from_xyz(0.0, 0.0, Z_ROBOT),
                    Visibility::Hidden,
                    MobileVis,
                    MobileAgent::idle(AgentClass::DeliveryRobot),
                    OperatorSlot::default(),
                ))
                .id(),
        );
    }

    let mut teslas = Vec::with_capacity(MAX_TESLAS);
    for _ in 0..MAX_TESLAS {
        teslas.push(
            commands
                .spawn((
                    Mesh2d(veh_mesh.clone()),
                    MeshMaterial2d(tesla_mat.clone()),
                    Transform::from_xyz(0.0, 0.0, Z_TESLA),
                    Visibility::Hidden,
                    MobileVis,
                    MobileAgent::idle(AgentClass::Tesla),
                    OperatorSlot::default(),
                ))
                .id(),
        );
    }

    AgentPool {
        vehicles,
        peds,
        buses,
        robots,
        teslas,
        // Fixed seed: pedestrian motion is cosmetic; reproducible run-to-run.
        rng: WyRand::new(0x9E37_79B9_7F4A_7C15),
        target_peds: 0,
        target_robots: 0,
        target_teslas: 0,
        active_peds: 0,
        active_robots: 0,
        active_teslas: 0,
        ped_mat,
        bus_mat,
        veh_mat,
        tesla_mat,
        glasses_icon: icons.glasses,
        bus_icon: icons.bus,
        taxi_icon: icons.taxi,
        tesla_icon: icons.tesla,
    }
}

fn activate_ped(agent: &mut MobileAgent, sim: &Sim, rng: &mut WyRand, start_node: u32) {
    let route = sim.graph.random_walk_route(start_node, PED_WALK_EDGES, rng);
    agent.progress_m = rng.next_f64() * route.total_m.max(1.0);
    agent.route = Arc::new(route);
    agent.speed_mps = PED_SPEED_MPS;
    agent.active = true;
    agent.counted = false;
    agent.seg_hint = 0;
}

/// Sample a graph node from a cumulative weight table (so agents cluster where the
/// weight is high). Uniform fallback if the table is missing/empty.
fn weighted_node(cum: &[f32], n: usize, rng: &mut WyRand) -> u32 {
    if cum.len() != n || n == 0 {
        return if n > 0 { rng.below(n) as u32 } else { 0 };
    }
    let total = *cum.last().unwrap();
    if total <= 0.0 {
        return rng.below(n) as u32;
    }
    let r = rng.next_f64() as f32 * total;
    cum.partition_point(|&c| c < r).min(n - 1) as u32
}

/// Activate a delivery robot: spawn at a robotability-weighted node and wander the
/// sidewalk graph (same walk model as pedestrians, slower).
fn activate_robot(agent: &mut MobileAgent, sim: &Sim, rng: &mut WyRand) {
    let start = weighted_node(&sim.robot_node_cumulative, sim.graph.node_count(), rng);
    let route = sim.graph.random_walk_route(start, PED_WALK_EDGES, rng);
    agent.progress_m = rng.next_f64() * route.total_m.max(1.0);
    agent.route = Arc::new(route);
    agent.speed_mps = ROBOT_SPEED_MPS;
    agent.active = true;
    agent.counted = false;
    agent.seg_hint = 0;
}

/// Activate a Tesla: spawn at a registration-density-weighted node + drive the graph.
fn activate_tesla(agent: &mut MobileAgent, sim: &Sim, rng: &mut WyRand) {
    let start = weighted_node(&sim.tesla_node_cumulative, sim.graph.node_count(), rng);
    let route = sim.graph.random_walk_route(start, PED_WALK_EDGES, rng);
    agent.progress_m = rng.next_f64() * route.total_m.max(1.0);
    agent.route = Arc::new(route);
    agent.speed_mps = TESLA_SPEED_MPS;
    agent.active = true;
    agent.counted = false;
    agent.seg_hint = 0;
}

/// Scale active agent counts by time-of-day and the two scenario sliders, so the
/// rendered density tracks the same fields the analytical model integrates.
pub fn scale_agent_population(
    params: Res<Params>,
    sim: Option<Res<Sim>>,
    ov: Res<OperatorsView>,
    clock: Res<SimClock>,
    mut pool: ResMut<AgentPool>,
    mut q: Query<(&mut MobileAgent, &mut Visibility)>,
) {
    let Some(sim) = sim else { return };
    if ov.active || ov.t > 0.0 {
        return; // freeze the active set while the Operators view holds the snapshot
    }
    let hour = clock.time_of_day; // live time-of-day → ped density tracks the clock
    let ped_scale =
        sim_core::mobile::pedestrian_multiplier(hour) * (params.glasses_per_1000 as f64 / 10.0);
    let want_ped = if params.glasses_on {
        ((MAX_PEDS as f64) * ped_scale).round().clamp(0.0, MAX_PEDS as f64) as usize
    } else {
        0
    };
    // Delivery robots: count-scaled by their activity curve × the density slider.
    let robot_scale =
        sim_core::mobile::robot_activity_multiplier(hour) * (params.robots_density as f64 / 2.0);
    let want_robot = if params.robots_on {
        ((MAX_ROBOTS as f64) * robot_scale).round().clamp(0.0, MAX_ROBOTS as f64) as usize
    } else {
        0
    };
    // Teslas: moving agents follow traffic, but with a high floor — Sentry keeps
    // parked Teslas recording 24/7, so they're never really "off the street".
    let tesla_scale =
        (0.4 + 0.6 * sim_core::mobile::traffic_multiplier(hour)) * (params.tesla_density as f64 / 4.0);
    let want_tesla = if params.tesla_on {
        ((MAX_TESLAS as f64) * tesla_scale).round().clamp(0.0, MAX_TESLAS as f64) as usize
    } else {
        0
    };
    pool.target_peds = want_ped;
    pool.target_robots = want_robot;
    pool.target_teslas = want_tesla;
    if pool.active_peds == want_ped
        && pool.active_robots == want_robot
        && pool.active_teslas == want_tesla
    {
        return; // steady state
    }

    let mut budget = ACTIVATIONS_PER_FRAME;
    let ped_entities = pool.peds.clone();
    let node_count = sim.graph.node_count() as u32;
    let mut active_ped = pool.active_peds;
    reconcile(
        &ped_entities,
        &mut active_ped,
        want_ped,
        &mut budget,
        &mut q,
        &mut |agent, rng| {
            let node = if node_count > 0 { (rng.below(node_count as usize)) as u32 } else { 0 };
            activate_ped(agent, &sim, rng, node);
        },
        &mut pool.rng,
    );
    pool.active_peds = active_ped;

    let robot_entities = pool.robots.clone();
    let mut active_robot = pool.active_robots;
    reconcile(
        &robot_entities,
        &mut active_robot,
        want_robot,
        &mut budget,
        &mut q,
        &mut |agent, rng| activate_robot(agent, &sim, rng),
        &mut pool.rng,
    );
    pool.active_robots = active_robot;

    let tesla_entities = pool.teslas.clone();
    let mut active_tesla = pool.active_teslas;
    reconcile(
        &tesla_entities,
        &mut active_tesla,
        want_tesla,
        &mut budget,
        &mut q,
        &mut |agent, rng| activate_tesla(agent, &sim, rng),
        &mut pool.rng,
    );
    pool.active_teslas = active_tesla;
}

/// Nudge `*active` toward `target` by flipping ≤`budget` pooled agents this
/// frame, keeping the tracked count in sync.
#[allow(clippy::too_many_arguments)]
fn reconcile(
    entities: &[Entity],
    active: &mut usize,
    target: usize,
    budget: &mut usize,
    q: &mut Query<(&mut MobileAgent, &mut Visibility)>,
    activate: &mut dyn FnMut(&mut MobileAgent, &mut WyRand),
    rng: &mut WyRand,
) {
    if *active < target {
        let mut to_add = (target - *active).min(*budget);
        for &e in entities {
            if to_add == 0 {
                break;
            }
            if let Ok((mut agent, mut vis)) = q.get_mut(e) {
                if !agent.active {
                    activate(&mut agent, rng);
                    *vis = Visibility::Inherited;
                    *active += 1;
                    to_add -= 1;
                    *budget -= 1;
                }
            }
        }
    } else if *active > target {
        let mut to_remove = (*active - target).min(*budget);
        for &e in entities {
            if to_remove == 0 {
                break;
            }
            if let Ok((mut agent, mut vis)) = q.get_mut(e) {
                if agent.active {
                    agent.active = false;
                    *vis = Visibility::Hidden;
                    *active -= 1;
                    to_remove -= 1;
                    *budget -= 1;
                }
            }
        }
    }
}

/// Advance every active agent along its route; recycle in place on completion.
pub fn animate_agents(
    time: Res<Time>,
    params: Res<Params>,
    sim: Option<Res<Sim>>,
    ov: Res<OperatorsView>,
    clock: Res<SimClock>,
    mut pool: ResMut<AgentPool>,
    mut q: Query<(&mut MobileAgent, &mut Transform)>,
) {
    let Some(sim) = sim else { return };
    if ov.active || ov.t > 0.0 {
        return; // the Operators view drives the agents (operators_animate_mobile)
    }
    if params.heatmap_on || !params.show_agents {
        return; // hidden; skip the work entirely
    }
    let dt = time.delta_secs_f64() * clock.rate;
    for (mut agent, mut tf) in &mut q {
        if !agent.active {
            continue;
        }
        // Pedestrians + delivery robots wander on the diurnal model (advance +
        // recycle), but only when playing; vehicles + buses are driven by
        // `replay_agents` (progress set from the real schedule), so here they are
        // only positioned. Positioning runs even when paused (scrub-and-pause).
        if clock.playing
            && matches!(
                agent.class,
                AgentClass::Pedestrian | AgentClass::DeliveryRobot | AgentClass::Tesla
            )
        {
            agent.progress_m += agent.speed_mps * dt;
            if agent.progress_m > agent.route.total_m {
                let end = agent.route.position_at(agent.route.total_m);
                let node = sim.graph.snap_nearest(end).unwrap_or(0);
                agent.route = Arc::new(sim.graph.random_walk_route(node, PED_WALK_EDGES, &mut pool.rng));
                agent.progress_m = 0.0;
                agent.counted = false;
                agent.seg_hint = 0;
            }
        }
        let (p, h) = {
            let MobileAgent { route, progress_m, seg_hint, .. } = &mut *agent;
            route.position_and_heading_at(*progress_m, seg_hint)
        };
        let z = match agent.class {
            AgentClass::Vehicle => Z_VEHICLE,
            AgentClass::Bus => Z_BUS,
            AgentClass::Pedestrian => Z_PED,
            AgentClass::DeliveryRobot => Z_ROBOT,
            AgentClass::Tesla => Z_TESLA,
        };
        tf.translation = world::to_world(p, z);
        tf.rotation = Quat::from_rotation_z((h.y.atan2(h.x) - FRAC_PI_2) as f32);
        if agent.flash > 0.0 {
            agent.flash = (agent.flash - time.delta_secs() * 2.5).max(0.0);
        }
        tf.scale = Vec3::splat(1.0 + agent.flash * 1.4);
    }
}

/// Narrative mode: agents passing the walker increment a live tally. Dashcam
/// vehicles count *every* in-range pass (deterministic — the live walk is meant to
/// show each device that physically drives by, decoupled from the analytical
/// headline estimate); the other mobile classes keep a capture-given-pass Bernoulli
/// roll. Detection uses a swept closest-approach test so fast agents (high clock
/// rates) can't tunnel past the capture radius between sampled frames.
pub fn mobile_capture_events(
    params: Res<Params>,
    route: Res<RouteState>,
    mut pool: ResMut<AgentPool>,
    mut walk_live: ResMut<WalkLive>,
    walker_q: Query<&Walker>,
    mut agents: Query<&mut MobileAgent>,
) {
    if params.mode != Mode::Route || params.exposure_mode != ExposureMode::Narrative {
        return;
    }
    let Some(r) = &route.route else { return };
    let Ok(walker) = walker_q.single() else { return };
    let wpos = r.position_at(walker.progress_m);

    // Per-encounter probability that a passing agent is a recording device that
    // captures you (mirrors the analytical product, sans the rate terms which
    // are already expressed by the agent flux).
    //
    // Dashcams are deterministic (p = 1.0): every rideshare vehicle that drives
    // within range registers + flashes, so the live walk renders each pass you can
    // see on the map. This intentionally runs hotter than the analytical estimate
    // (which discounts by fleet penetration × capture_prob); the headline still
    // reports the calibrated expectation.
    let p_vehicle = 1.0_f64;
    let p_glasses = ((params.glasses_per_1000 as f64 / 1000.0) * 0.05 * 0.4).clamp(0.0, 1.0);
    // ACE buses are Tier A: the camera is always present, so the only chance term
    // is capture-given-pass (the bus's curb-side FOV catching you).
    let p_bus = 0.7;
    // Robots always carry nav cameras; the chance term is capture-given-pass.
    let p_robot = 0.5;
    // Teslas always carry Sentry/Autopilot cameras; chance term is capture-given-pass.
    let p_tesla = 0.3;

    for mut agent in &mut agents {
        if !agent.active {
            continue;
        }
        let cur = agent.route.position_at(agent.progress_m);
        let (range, p) = match agent.class {
            AgentClass::Vehicle => (VEHICLE_CAPTURE_R, p_vehicle),
            AgentClass::Pedestrian => (PED_CAPTURE_R, p_glasses),
            AgentClass::Bus => (BUS_CAPTURE_R, p_bus),
            AgentClass::DeliveryRobot => (ROBOT_CAPTURE_R, p_robot),
            AgentClass::Tesla => (TESLA_CAPTURE_R, p_tesla),
        };
        // Closest approach of the agent's swept segment (last frame → this frame) to
        // the walker, so a fast agent can't skip the radius between samples. A
        // teleport-sized step (slot reassignment / stale `prev_pos`) degrades to a
        // point test.
        let from = if agent.prev_pos.distance(cur) > MAX_SWEEP_M {
            cur
        } else {
            agent.prev_pos
        };
        let near = point_segment_distance(wpos, from, cur) <= range;
        agent.prev_pos = cur;
        if near && !agent.counted {
            agent.counted = true;
            if pool.rng.next_f64() < p {
                match agent.class {
                    AgentClass::Vehicle => walk_live.mobile_vehicle += 1,
                    AgentClass::Pedestrian => walk_live.mobile_glasses += 1,
                    AgentClass::Bus => walk_live.mobile_bus += 1,
                    AgentClass::DeliveryRobot => walk_live.mobile_robot += 1,
                    AgentClass::Tesla => walk_live.mobile_tesla += 1,
                }
                agent.flash = 1.0;
            }
        } else if !near {
            agent.counted = false; // re-arm after leaving range
        }
    }
}

/// Indices of the start-sorted `btrips` that *may* be live at minute `now`: the union
/// of the `now` window and the after-midnight `now + 1440` window (trips encoded past
/// 24h). A trip live at effective minute `m` has `start ∈ (m - max_span, m]` — since
/// `end = start + span > m` forces `start > m - max_span`, and `start ≤ m` — so each
/// window is a contiguous `partition_point`-bounded range. The caller still applies the
/// exact `[start,end)` `bus_active` test; this only *bounds the scan* (replacing a full
/// timetable sweep) and yields ascending indices, so the admitted set — and the over-cap
/// earliest-start subsample — is byte-for-byte the same as the old `0..len` full scan.
fn bus_admit_indices(
    btrips: &[BusTrip],
    now: f32,
    max_span: f32,
) -> std::iter::Chain<std::ops::Range<usize>, std::ops::Range<usize>> {
    let window_at = |m: f32| {
        let lo = btrips.partition_point(|t| t.start_min <= m - max_span);
        let hi = btrips.partition_point(|t| t.start_min <= m);
        lo..hi
    };
    window_at(now).chain(window_at(now + 1440.0))
}

/// Drive vehicles (taxis) + buses from the baked real-day schedules: a trip occupies
/// a pooled entity while the clock is within its window; `animate_agents` then
/// positions it from the `progress_m` we set here. Over the cap the active set is
/// deterministically subsampled (earliest-by-start fill). Pedestrians are untouched
/// (they stay on the diurnal model).
#[allow(clippy::too_many_arguments)]
pub fn replay_agents(
    sim: Option<Res<Sim>>,
    clock: Res<SimClock>,
    ov: Res<OperatorsView>,
    params: Res<Params>,
    pool: Res<AgentPool>,
    mut rs: ResMut<ReplayState>,
    mut q: Query<&mut MobileAgent>,
    cam: Query<(&Camera, &Transform), With<Camera2d>>,
    cov: Option<Res<crate::coverage::CoverageView>>,
) {
    let Some(sim) = sim else { return };
    if ov.active || ov.t > 0.0 {
        return; // Operators view owns the agents
    }
    // The roving-coverage overlay is a downstream *statistic* (which streets the fleet
    // covers over a day), so it must see the unbiased active set, not the viewport-culled
    // on-screen subset. While it runs, admit by start order only (cull off) — the pool
    // then samples the true fleet uniformly as it cycles, instead of whatever's on camera.
    let cull_off = cov.as_ref().map(|c| c.active).unwrap_or(false);
    // Runs whether playing or paused: it only *reads* `clock.time_of_day`, so a
    // scrub-and-pause still shows exactly the trips active at that minute.
    if pool.vehicles.len() != rs.veh_slot_trip.len() || pool.buses.len() != rs.bus_slot_trip.len() {
        return; // pool / replay-state not yet aligned (pre-build)
    }
    let now = (clock.time_of_day * 60.0) as f32;

    // ---------- Taxis: forward-cursor sweep over start-sorted trips ----------
    let trips = &sim.taxi_day.trips;
    let taxi_on = params.show_agents && params.dashcam_on && !params.heatmap_on;
    let taxi_active = |t: &TaxiTrip| now >= t.pu_min && now < t.pu_min + t.dur_min;

    // ---------- Viewport cull ----------
    // Rendering is in ENU metres (1 world unit = 1 m), so the camera translation/scale
    // map straight onto route bboxes. Visible rect (+ margin); `None` (no camera) admits
    // everything. A taxi is admitted only if its whole route's bbox overlaps the rect —
    // a cheap rect test, no `position_at` to decide visibility. The pool then bounds the
    // *on-screen* count, letting every trip be routable without a day-sized pool.
    let viewport: Option<[f64; 4]> = cam.single().ok().and_then(|(c, t)| {
        c.logical_viewport_size().map(|v| {
            let s = t.scale.x as f64;
            let (cx, cy) = (t.translation.x as f64, t.translation.y as f64);
            let (hx, hy) = (v.x as f64 * s * 0.6, v.y as f64 * s * 0.6); // half-frame + 20% margin
            [cx - hx, cy - hy, cx + hx, cy + hy]
        })
    });
    let in_view = |bb: &[f32; 4]| {
        if cull_off {
            return true; // coverage stat: admit the unbiased active set, not the on-screen cull
        }
        match viewport {
            None => true,
            Some(v) => {
                (bb[0] as f64) <= v[2]
                    && (bb[2] as f64) >= v[0]
                    && (bb[1] as f64) <= v[3]
                    && (bb[3] as f64) >= v[1]
            }
        }
    };
    // The view has panned/zoomed to a *new resting place* → re-sync the visible taxi set.
    // We rebuild once motion settles (a frame still vs. the last) rather than every drag
    // frame, so a gesture costs one rebuild, not dozens. Thresholds are fractions of the
    // current frame width.
    let vp_rebuild = match viewport {
        Some(v) => {
            let w = (v[2] - v[0]).max(1.0);
            let cur = [(v[0] + v[2]) * 0.5, (v[1] + v[3]) * 0.5, w];
            let dist = |a: [f64; 3], b: [f64; 3]| {
                (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).max((a[2] - b[2]).abs())
            };
            let still = dist(cur, rs.veh_vp_prev) < 0.02 * w;
            let desynced = dist(cur, rs.veh_vp) > 0.10 * w;
            rs.veh_vp_prev = cur;
            let go = still && desynced;
            if go {
                rs.veh_vp = cur;
            }
            go
        }
        None => false,
    };

    // Wrap / scrub / toggled-off / viewport-resync → drop all taxi slots and rebuild,
    // re-scanning from the start of the currently-active window (binary search keeps the
    // re-scan O(active), not O(whole day)).
    let jumped = now < rs.veh_last_now - 1.0 || now > rs.veh_last_now + 30.0;
    if !taxi_on || jumped || vp_rebuild {
        for slot in 0..rs.veh_slot_trip.len() {
            if rs.veh_slot_trip[slot] >= 0 {
                if let Ok(mut a) = q.get_mut(pool.vehicles[slot]) {
                    a.active = false;
                }
                rs.veh_slot_trip[slot] = -1;
            }
        }
        rs.veh_cursor = trips.partition_point(|t| t.pu_min < now - MAX_TRIP_DUR_MIN);
    }
    if taxi_on {
        // Free slots whose trip has ended.
        for slot in 0..rs.veh_slot_trip.len() {
            let ti = rs.veh_slot_trip[slot];
            if ti >= 0 && !taxi_active(&trips[ti as usize]) {
                if let Ok(mut a) = q.get_mut(pool.vehicles[slot]) {
                    a.active = false;
                }
                rs.veh_slot_trip[slot] = -1;
            }
        }
        // Admit started trips into free slots, in start order (subsample over cap).
        let mut free = 0usize;
        while rs.veh_cursor < trips.len() && trips[rs.veh_cursor].pu_min <= now {
            let ti = rs.veh_cursor;
            rs.veh_cursor += 1;
            if !taxi_active(&trips[ti]) {
                continue;
            }
            if !in_view(&sim.taxi_route_bboxes[trips[ti].route_idx as usize]) {
                continue; // route never enters the viewport → don't spend a slot on it
            }
            while free < rs.veh_slot_trip.len() && rs.veh_slot_trip[free] >= 0 {
                free += 1;
            }
            if free >= rs.veh_slot_trip.len() {
                break; // pool full
            }
            rs.veh_slot_trip[free] = ti as i32;
            let route = Arc::clone(&sim.taxi_routes[trips[ti].route_idx as usize]);
            if let Ok(mut a) = q.get_mut(pool.vehicles[free]) {
                a.route = route;
                a.active = true;
                a.flash = 0.0;
                a.counted = false;
                a.seg_hint = 0;
            }
        }
        // Position active taxis along their route by elapsed fraction — but through a
        // turn-aware, speed-limit-capped pace (brake into corners, cruise on straights)
        // instead of a flat constant-speed glide. The pace is precomputed per route.
        for slot in 0..rs.veh_slot_trip.len() {
            let ti = rs.veh_slot_trip[slot];
            if ti >= 0 {
                let t = &trips[ti as usize];
                let frac = ((now - t.pu_min) / t.dur_min).clamp(0.0, 1.0) as f64;
                if let Ok(mut a) = q.get_mut(pool.vehicles[slot]) {
                    a.progress_m = match sim.taxi_paces.get(t.route_idx as usize) {
                        Some(pace) => pace.arc_at(&a.route, frac),
                        None => frac * a.route.total_m,
                    };
                }
            }
        }
    }
    rs.veh_last_now = now;

    // ---------- Buses: two windowed scans over the start-sorted timetable ----------
    // A bus is live if `now` is inside its `[start,end)`, or (for trips encoded past
    // 24h) if `now + 1440` is. Both conditions are contiguous start-sorted ranges, so
    // instead of scanning the whole day's timetable every frame we admit from just those
    // two `partition_point`-bounded windows. Same `bus_active` predicate and same
    // ascending-index (earliest-start-first) admission order as the old full scan, so the
    // active set — and the over-cap subsample — is identical.
    let btrips = &sim.bus_day.trips;
    let bus_on = params.show_agents && params.ace_on && !params.heatmap_on;
    let bus_active = |t: &BusTrip| {
        (now >= t.start_min && now < t.end_min)
            || (now >= t.start_min - 1440.0 && now < t.end_min - 1440.0)
    };
    // Widest trip span → how far before `now` a still-running trip may have started.
    // Computed once (start-sorted trips make this the only bound the scan windows need).
    let max_span = *rs
        .bus_max_span_min
        .get_or_insert_with(|| btrips.iter().map(|t| t.end_min - t.start_min).fold(0.0_f32, f32::max));
    for slot in 0..rs.bus_slot_trip.len() {
        let ti = rs.bus_slot_trip[slot];
        if ti >= 0 && (!bus_on || !bus_active(&btrips[ti as usize])) {
            rs.bus_mapped[ti as usize] = false;
            rs.bus_slot_trip[slot] = -1;
            if let Ok(mut a) = q.get_mut(pool.buses[slot]) {
                a.active = false;
            }
        }
    }
    if bus_on {
        let mut free = 0usize;
        // Window A (now) has lower indices than window B (now+1440), so the chained
        // iterator preserves earliest-start-first order; `bus_mapped` guards any
        // (impossible, for buses) double-admit. See [`bus_admit_indices`].
        'admit: for ti in bus_admit_indices(btrips, now, max_span) {
            if !bus_active(&btrips[ti]) || rs.bus_mapped[ti] {
                continue;
            }
            while free < rs.bus_slot_trip.len() && rs.bus_slot_trip[free] >= 0 {
                free += 1;
            }
            if free >= rs.bus_slot_trip.len() {
                break 'admit;
            }
            rs.bus_slot_trip[free] = ti as i32;
            rs.bus_mapped[ti] = true;
            let route = Arc::clone(&sim.bus_routes[btrips[ti].shape_idx as usize]);
            if let Ok(mut a) = q.get_mut(pool.buses[free]) {
                a.route = route;
                a.active = true;
                a.flash = 0.0;
                a.counted = false;
                a.seg_hint = 0;
            }
        }
        // Position active buses by their real per-stop time→arc keyframes, so motion
        // tracks the actual timetable (dwell near stops, faster between them) rather
        // than a flat constant speed.
        for slot in 0..rs.bus_slot_trip.len() {
            let ti = rs.bus_slot_trip[slot];
            if ti >= 0 {
                let t = &btrips[ti as usize];
                let nm = if now >= t.start_min { now } else { now + 1440.0 };
                let arc = bus_arc_at(&t.keyframes, nm) as f64;
                if let Ok(mut a) = q.get_mut(pool.buses[slot]) {
                    a.progress_m = arc;
                }
            }
        }
    }
}

/// Interpolate a bus trip's monotonic `[time_min, arc_m]` keyframes to the arc
/// length at minute `t` (clamped to the trip's first/last stop).
///
/// Each keyframe is a stop's *departure* time, so within an inter-stop segment the
/// bus **travels** (smoothstep-eased, decelerating into the stop) and then **dwells**
/// at the upcoming stop until that departure time — giving visible stop-and-go motion
/// instead of a flat constant-speed glide.
fn bus_arc_at(kf: &[[f32; 2]], t: f32) -> f32 {
    if kf.is_empty() {
        return 0.0;
    }
    if t <= kf[0][0] {
        return kf[0][1];
    }
    let last = kf[kf.len() - 1];
    if t >= last[0] {
        return last[1];
    }
    for w in kf.windows(2) {
        let (t0, a0) = (w[0][0], w[0][1]);
        let (t1, a1) = (w[1][0], w[1][1]);
        if t >= t0 && t <= t1 {
            let seg = t1 - t0;
            if seg <= 0.0 {
                return a1;
            }
            // Dwell at the segment's end (the arrival stop); travel fills the rest.
            let dwell = BUS_DWELL_MIN.min(BUS_DWELL_FRAC * seg);
            let travel = (seg - dwell).max(1e-4);
            let f = ((t - t0) / travel).clamp(0.0, 1.0); // 1.0 → arrived, now dwelling
            let eased = f * f * (3.0 - 2.0 * f); // smoothstep: ease out of / into stops
            return a0 + (a1 - a0) * eased;
        }
    }
    last[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `bus_admit_indices` (the two windowed scans) must admit exactly the trips a full
    /// `0..len` scan filtered by `bus_active` would, for every minute of the day —
    /// including the after-midnight wrap for trips encoded past 1440.
    #[test]
    fn bus_admit_windows_match_full_scan() {
        let mk = |start: f32, end: f32| BusTrip {
            route_idx: 0,
            shape_idx: 0,
            start_min: start,
            end_min: end,
            keyframes: vec![[start, 0.0], [end, 100.0]],
        };
        // Regular daytime trips, an evening trip that runs past midnight (end > 1440),
        // and after-midnight trips encoded past 24h (start ≥ 1440).
        let mut trips = vec![
            mk(0.0, 45.0),
            mk(360.0, 400.0),
            mk(360.0, 500.0), // overlapping-start, longer → sets max_span
            mk(720.0, 765.0),
            mk(1380.0, 1470.0), // starts 23:00, ends 00:30 next day
            mk(1450.0, 1500.0), // after-midnight (01:10–01:30 encoded past 24h)
            mk(1439.0, 1441.0),
        ];
        trips.sort_by(|a, b| a.start_min.total_cmp(&b.start_min));
        let max_span = trips.iter().map(|t| t.end_min - t.start_min).fold(0.0_f32, f32::max);

        for now_i in 0..1440 {
            let now = now_i as f32;
            let bus_active = |t: &BusTrip| {
                (now >= t.start_min && now < t.end_min)
                    || (now >= t.start_min - 1440.0 && now < t.end_min - 1440.0)
            };
            // Ground truth: every trip the old full scan would find active.
            let expected: Vec<usize> =
                (0..trips.len()).filter(|&i| bus_active(&trips[i])).collect();
            // Via the windows (then the same exact predicate + dedup, as the system does).
            let mut got: Vec<usize> = Vec::new();
            for i in bus_admit_indices(&trips, now, max_span) {
                if bus_active(&trips[i]) && !got.contains(&i) {
                    got.push(i);
                }
            }
            got.sort_unstable();
            assert_eq!(got, expected, "active set differs at minute {now_i}");
        }
    }

    #[test]
    fn landmarks_occlude_agents() {
        // The landmark massing must draw above every mobile agent so a building's
        // 2.5D silhouette occludes vehicles/peds/Teslas/robots driving behind it.
        let agent_max = Z_VEHICLE.max(Z_PED).max(Z_BUS).max(Z_ROBOT).max(Z_TESLA);
        assert!(
            crate::LANDMARK_MASSING_Z > agent_max,
            "landmark massing z ({}) must exceed every agent z ({agent_max})",
            crate::LANDMARK_MASSING_Z,
        );
    }

    #[test]
    fn camera_icons_floor_to_min_screen_size() {
        use crate::{
            icon_floor_px, icon_half_for, ICON_FLOOR_FAR_PX, ICON_FLOOR_NEAR_PX, ICON_TAPER_FAR,
            ICON_TAPER_NEAR,
        };
        let base = 13.0_f32; // a 26 m icon
        // Zoomed in (small m/px): the intrinsic size wins, icons grow on screen.
        assert_eq!(icon_half_for(base, 0.5), base);
        // Zoomed out (large m/px): floored to the on-screen pixel floor for that zoom.
        let scale = 9.0_f32; // ~island overview
        let half = icon_half_for(base, scale);
        assert!(half > base, "the pixel floor kicks in when zoomed out");
        assert!(
            (half / scale - icon_floor_px(scale)).abs() < 1e-3,
            "at the floor, on-screen half-size == icon_floor_px(scale)",
        );
        // The floor tapers: smaller (in px) when zoomed out than when zoomed in.
        assert!(icon_floor_px(ICON_TAPER_FAR + 5.0) < icon_floor_px(ICON_TAPER_NEAR - 1.0));
        assert!((icon_floor_px(ICON_TAPER_NEAR - 1.0) - ICON_FLOOR_NEAR_PX).abs() < 1e-3);
        assert!((icon_floor_px(ICON_TAPER_FAR + 1.0) - ICON_FLOOR_FAR_PX).abs() < 1e-3);
        // Never smaller than the intrinsic size, however far in you zoom.
        assert!(icon_half_for(base, 0.01) >= base);
    }

    #[test]
    fn heatmap_colormap_is_perceptually_ordered() {
        use crate::heat_rgba;
        // Empty cells are fully transparent.
        assert_eq!(heat_rgba(0.0), [0, 0, 0, 0]);
        let lum = |c: [u8; 4]| 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
        // Density reads as darkness: luminance decreases monotonically with intensity,
        // so equal data steps look like equal visual steps (perceptual ordering).
        let lums: Vec<f32> = [0.1, 0.3, 0.5, 0.7, 0.9, 1.0]
            .iter()
            .map(|&n| lum(heat_rgba(n)))
            .collect();
        for w in lums.windows(2) {
            assert!(w[0] > w[1], "colormap must darken as density rises: {lums:?}");
        }
        // Alpha is held CONSTANT across density (density reads from color alone — a
        // density-linked alpha would double the lightness gradient and break perceptual
        // uniformity). Every non-empty cell shares one opacity, well above the floor.
        assert_eq!(heat_rgba(0.1)[3], heat_rgba(0.9)[3], "non-empty cells share one alpha");
        assert!(heat_rgba(0.1)[3] > 80, "low-density cells stay visible");
    }

    #[test]
    fn declutter_drops_overlapping_lower_priority_labels() {
        use crate::declutter_keep;
        use bevy::math::Vec2;
        // Boxes are fed in priority order. The first is kept; the second overlaps it (centres
        // 10 apart, half-widths 8 each → |Δx| 10 < 16) so it yields; the third is far → kept.
        let boxes = [
            (Vec2::new(0.0, 0.0), 8.0, 5.0),
            (Vec2::new(10.0, 0.0), 8.0, 5.0),
            (Vec2::new(100.0, 0.0), 8.0, 5.0),
        ];
        assert_eq!(declutter_keep(&boxes), vec![true, false, true]);
        // Disjoint boxes (touching beyond the half-width sum) all survive.
        let disjoint = [
            (Vec2::new(0.0, 0.0), 4.0, 4.0),
            (Vec2::new(9.0, 0.0), 4.0, 4.0),
        ];
        assert_eq!(declutter_keep(&disjoint), vec![true, true]);
    }

    #[test]
    fn future_mode_groups_speculative_layers() {
        use crate::Params;
        let mut p = Params::default();
        // Smart glasses + future mode are off by default (the speculative layers hide).
        assert!(!p.future_on && !p.glasses_on && !p.robots_on);
        p.set_future(true);
        assert!(p.future_on && p.glasses_on && p.robots_on, "one switch shows both");
        p.set_future(false);
        assert!(!p.future_on && !p.glasses_on && !p.robots_on, "and hides both");
    }

    #[test]
    fn evidence_number_formatting() {
        use crate::{compact_usd, group_thousands};
        assert_eq!(group_thousands(242_742), "242,742");
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1_000_000), "1,000,000");
        assert_eq!(compact_usd(27_911_985.0), "$27.9M");
        assert_eq!(compact_usd(950.0), "$950");
        assert_eq!(compact_usd(12_500.0), "$12K");
        assert_eq!(compact_usd(2_500_000_000.0), "$2.5B");
    }

    #[test]
    fn ace_ribbons_floor_to_min_screen_width() {
        use crate::{ace_half_for, MIN_ACE_HALF_PX};
        let base = 4.0_f32;
        assert_eq!(ace_half_for(base, 0.5), base, "zoomed in: intrinsic width");
        let scale = 4.6_f32;
        let half = ace_half_for(base, scale);
        assert!(half > base, "zoomed out: floored wider so it stays visible");
        assert!((half / scale - MIN_ACE_HALF_PX).abs() < 1e-3);
    }

    #[test]
    fn bus_dwells_at_stops() {
        // Stops: arc 0 at t=0, arc 100 at t=1. dwell = min(0.25, 0.4·1) = 0.25, so the
        // bus travels over [0, 0.75] and holds at arc 100 through [0.75, 1.0].
        let kf = [[0.0f32, 0.0], [1.0, 100.0]];
        assert!((bus_arc_at(&kf, 0.75) - 100.0).abs() < 0.5, "arrives by end of travel");
        assert!((bus_arc_at(&kf, 0.9) - 100.0).abs() < 0.5, "holds at the stop (dwell)");
        let mid = bus_arc_at(&kf, 0.375);
        assert!(mid > 0.0 && mid < 100.0, "moving strictly between stops");
    }
}

