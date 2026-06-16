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

use bevy::math::primitives::{Circle, RegularPolygon};
use bevy::prelude::*;
use std::f64::consts::FRAC_PI_2;

use sim_core::assets::VehicleRoute;
use sim_core::rng::{RngLike, WyRand};
use sim_core::{Route, Vec2};

use crate::{world, Mode, Params, RouteState, Sim, Walker, WalkLive, ExposureMode, ANIM_SPEEDUP};

// Fixed caps → provable worst-case per-frame cost on single-threaded WASM.
const MAX_VEHICLES: usize = 250;
const MAX_PEDS: usize = 400;

// Real speeds (m/s); the shared ANIM_SPEEDUP time-lapse applies on top.
const VEHICLE_SPEED_MPS: f64 = 8.0; // ~29 km/h urban crawl
const PED_SPEED_MPS: f64 = 1.34;

// Capture reach (m) for the narrative "passed you" test.
const VEHICLE_CAPTURE_R: f64 = 22.0;
const PED_CAPTURE_R: f64 = 6.0;

const PED_WALK_EDGES: usize = 16;
const ACTIVATIONS_PER_FRAME: usize = 32;

// z-order: streets 0.0, cameras 1.0, route 0.2, walker 4.0. Agents between.
const Z_VEHICLE: f32 = 2.6;
const Z_PED: f32 = 2.4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentClass {
    Vehicle,
    Pedestrian,
}

#[derive(Component)]
pub struct MobileAgent {
    pub class: AgentClass,
    pub route: Route,
    pub progress_m: f64,
    pub speed_mps: f64,
    pub active: bool,
    pub flash: f32,
    /// Debounce: already counted the walker this pass (narrative mode).
    pub counted: bool,
}

impl MobileAgent {
    fn idle(class: AgentClass) -> Self {
        MobileAgent {
            class,
            route: Route::from_points(Vec::new()),
            progress_m: 0.0,
            speed_mps: 0.0,
            active: false,
            flash: 0.0,
            counted: false,
        }
    }
}

/// Pool of pre-spawned agent entities + weighted vehicle-route sampler + RNG.
#[derive(Resource)]
pub struct AgentPool {
    pub vehicles: Vec<Entity>,
    pub peds: Vec<Entity>,
    /// Cumulative weights over `Sim.vehicle_routes` (ends at ~1.0) for O(log n) sampling.
    cumulative: Vec<f32>,
    rng: WyRand,
    target_vehicles: usize,
    target_peds: usize,
}

impl AgentPool {
    /// Placeholder inserted at startup so the agent systems always have the
    /// resource; replaced by [`spawn_pool`] once `build_world` runs.
    pub fn empty() -> Self {
        AgentPool {
            vehicles: Vec::new(),
            peds: Vec::new(),
            cumulative: Vec::new(),
            rng: WyRand::new(0x9E37_79B9_7F4A_7C15),
            target_vehicles: 0,
            target_peds: 0,
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
    vehicle_routes: &[VehicleRoute],
) -> AgentPool {
    let veh_mesh = meshes.add(RegularPolygon::new(7.0, 3));
    let veh_mat = materials.add(Color::srgb_u8(0xa8, 0x50, 0x1f)); // clay (Tier C)
    let ped_mesh = meshes.add(Circle::new(4.0));
    let ped_mat = materials.add(Color::srgb_u8(0x34, 0x51, 0x69)); // slate (Tier D)

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
                ))
                .id(),
        );
    }

    // Cumulative weight table (baker normalized weights to sum 1.0).
    let mut cumulative = Vec::with_capacity(vehicle_routes.len());
    let mut acc = 0.0f32;
    for r in vehicle_routes {
        acc += r.weight.max(0.0);
        cumulative.push(acc);
    }

    AgentPool {
        vehicles,
        peds,
        cumulative,
        // Fixed seed: agent motion is cosmetic; reproducible run-to-run.
        rng: WyRand::new(0x9E37_79B9_7F4A_7C15),
        target_vehicles: 0,
        target_peds: 0,
    }
}

fn route_from(vr: &VehicleRoute) -> Route {
    Route::from_points(
        vr.polyline
            .iter()
            .map(|p| Vec2::new(p[0] as f64, p[1] as f64))
            .collect(),
    )
}

fn activate_vehicle(agent: &mut MobileAgent, route: Route, rng: &mut WyRand) {
    let total = route.total_m;
    agent.route = route;
    agent.progress_m = rng.next_f64() * total.max(1.0);
    agent.speed_mps = VEHICLE_SPEED_MPS;
    agent.active = true;
    agent.counted = false;
}

fn activate_ped(agent: &mut MobileAgent, sim: &Sim, rng: &mut WyRand, start_node: u32) {
    let route = sim.graph.random_walk_route(start_node, PED_WALK_EDGES, rng);
    agent.progress_m = rng.next_f64() * route.total_m.max(1.0);
    agent.route = route;
    agent.speed_mps = PED_SPEED_MPS;
    agent.active = true;
    agent.counted = false;
}

/// Scale active agent counts by time-of-day and the two scenario sliders, so the
/// rendered density tracks the same fields the analytical model integrates.
pub fn scale_agent_population(
    params: Res<Params>,
    sim: Option<Res<Sim>>,
    mut pool: ResMut<AgentPool>,
    mut q: Query<(&mut MobileAgent, &mut Visibility)>,
) {
    let Some(sim) = sim else { return };
    let hour = params.departure_hour as f64;

    let veh_scale = sim_core::mobile::traffic_multiplier(hour) * (params.dashcam_penetration as f64 / 0.40);
    let ped_scale = sim_core::mobile::pedestrian_multiplier(hour) * (params.glasses_per_1000 as f64 / 10.0);
    let want_veh = if params.dashcam_on {
        ((MAX_VEHICLES as f64) * veh_scale).round().clamp(0.0, MAX_VEHICLES as f64) as usize
    } else {
        0
    };
    let want_ped = if params.glasses_on {
        ((MAX_PEDS as f64) * ped_scale).round().clamp(0.0, MAX_PEDS as f64) as usize
    } else {
        0
    };
    pool.target_vehicles = want_veh;
    pool.target_peds = want_ped;

    let mut budget = ACTIVATIONS_PER_FRAME;

    // Vehicles: nudge active count toward target, ≤budget changes this frame.
    // Clone the (small) cumulative table to a local so the activation closure
    // doesn't borrow `pool` while `pool.rng` is borrowed mutably.
    let cumulative = pool.cumulative.clone();
    let veh_entities = pool.vehicles.clone();
    let active_veh = count_active(&veh_entities, &q);
    reconcile(
        &veh_entities,
        active_veh,
        want_veh,
        &mut budget,
        &mut q,
        &mut |agent, rng| {
            if let Some(vr) = pick_route_owned(rng, &cumulative, &sim.vehicle_routes) {
                activate_vehicle(agent, route_from(&vr), rng);
            }
        },
        &mut pool.rng,
    );

    let ped_entities = pool.peds.clone();
    let active_ped = count_active(&ped_entities, &q);
    let node_count = sim.graph.node_count() as u32;
    reconcile(
        &ped_entities,
        active_ped,
        want_ped,
        &mut budget,
        &mut q,
        &mut |agent, rng| {
            let node = if node_count > 0 { (rng.below(node_count as usize)) as u32 } else { 0 };
            activate_ped(agent, &sim, rng, node);
        },
        &mut pool.rng,
    );
}

fn count_active(entities: &[Entity], q: &Query<(&mut MobileAgent, &mut Visibility)>) -> usize {
    entities.iter().filter(|&&e| q.get(e).map(|(a, _)| a.active).unwrap_or(false)).count()
}

#[allow(clippy::too_many_arguments)]
fn reconcile(
    entities: &[Entity],
    active: usize,
    target: usize,
    budget: &mut usize,
    q: &mut Query<(&mut MobileAgent, &mut Visibility)>,
    activate: &mut dyn FnMut(&mut MobileAgent, &mut WyRand),
    rng: &mut WyRand,
) {
    if active < target {
        let mut to_add = (target - active).min(*budget);
        for &e in entities {
            if to_add == 0 {
                break;
            }
            if let Ok((mut agent, mut vis)) = q.get_mut(e) {
                if !agent.active {
                    activate(&mut agent, rng);
                    *vis = Visibility::Inherited;
                    to_add -= 1;
                    *budget -= 1;
                }
            }
        }
    } else if active > target {
        let mut to_remove = (active - target).min(*budget);
        for &e in entities {
            if to_remove == 0 {
                break;
            }
            if let Ok((mut agent, mut vis)) = q.get_mut(e) {
                if agent.active {
                    agent.active = false;
                    *vis = Visibility::Hidden;
                    to_remove -= 1;
                    *budget -= 1;
                }
            }
        }
    }
}

// Owned-route pick to sidestep borrow conflicts inside the closure.
fn pick_route_owned(rng: &mut WyRand, cumulative: &[f32], routes: &[VehicleRoute]) -> Option<VehicleRoute> {
    if routes.is_empty() || cumulative.is_empty() {
        return None;
    }
    let total = *cumulative.last().unwrap();
    let r = rng.next_f64() as f32 * total;
    let idx = cumulative.partition_point(|&c| c < r).min(routes.len() - 1);
    routes.get(idx).cloned()
}

/// Advance every active agent along its route; recycle in place on completion.
pub fn animate_agents(
    time: Res<Time>,
    params: Res<Params>,
    sim: Option<Res<Sim>>,
    mut pool: ResMut<AgentPool>,
    mut q: Query<(&mut MobileAgent, &mut Transform)>,
) {
    let Some(sim) = sim else { return };
    if params.heatmap_on || !params.show_agents {
        return; // hidden; skip the work entirely
    }
    let dt = time.delta_secs_f64() * ANIM_SPEEDUP;
    // Local copy so per-agent recycling can borrow it while `pool.rng` is mutable.
    let cumulative = pool.cumulative.clone();

    for (mut agent, mut tf) in &mut q {
        if !agent.active {
            continue;
        }
        agent.progress_m += agent.speed_mps * dt;
        if agent.progress_m > agent.route.total_m {
            // Recycle in place — no despawn.
            match agent.class {
                AgentClass::Vehicle => {
                    if let Some(vr) = pick_route_owned(&mut pool.rng, &cumulative, &sim.vehicle_routes) {
                        let route = route_from(&vr);
                        agent.route = route;
                        agent.progress_m = 0.0;
                        agent.counted = false;
                    } else {
                        agent.progress_m = 0.0;
                    }
                }
                AgentClass::Pedestrian => {
                    let end = agent.route.position_at(agent.route.total_m);
                    let node = sim.graph.snap_nearest(end).unwrap_or(0);
                    let route = sim.graph.random_walk_route(node, PED_WALK_EDGES, &mut pool.rng);
                    agent.route = route;
                    agent.progress_m = 0.0;
                    agent.counted = false;
                }
            }
        }
        let p = agent.route.position_at(agent.progress_m);
        let z = if agent.class == AgentClass::Vehicle { Z_VEHICLE } else { Z_PED };
        tf.translation = world::to_world(p, z);
        // Orient vehicles along travel; pulse on a recent capture.
        if agent.class == AgentClass::Vehicle {
            let h = agent.route.heading_at(agent.progress_m);
            tf.rotation = Quat::from_rotation_z((h.y.atan2(h.x) - FRAC_PI_2) as f32);
        }
        if agent.flash > 0.0 {
            agent.flash = (agent.flash - time.delta_secs() * 2.5).max(0.0);
        }
        tf.scale = Vec3::splat(1.0 + agent.flash * 1.4);
    }
}

/// Narrative mode: agents passing the walker increment a live stochastic tally,
/// with the device∧capture probability folded into a Bernoulli roll so the
/// expectation tracks the analytical dashcam/glasses rates.
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
    let p_vehicle = (params.dashcam_penetration as f64 * 0.40).clamp(0.0, 1.0); // penetration × capture_prob
    let p_glasses = ((params.glasses_per_1000 as f64 / 1000.0) * 0.05 * 0.4).clamp(0.0, 1.0);

    for mut agent in &mut agents {
        if !agent.active {
            continue;
        }
        let apos = agent.route.position_at(agent.progress_m);
        let (range, p) = match agent.class {
            AgentClass::Vehicle => (VEHICLE_CAPTURE_R, p_vehicle),
            AgentClass::Pedestrian => (PED_CAPTURE_R, p_glasses),
        };
        let near = apos.distance(wpos) <= range;
        if near && !agent.counted {
            agent.counted = true;
            if pool.rng.next_f64() < p {
                match agent.class {
                    AgentClass::Vehicle => walk_live.mobile_vehicle += 1,
                    AgentClass::Pedestrian => walk_live.mobile_glasses += 1,
                }
                agent.flash = 1.0;
            }
        } else if !near {
            agent.counted = false; // re-arm after leaving range
        }
    }
}
