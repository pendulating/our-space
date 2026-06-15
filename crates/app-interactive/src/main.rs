//! Interactive our-space app (native dev window; WebGPU WASM target in Phase 4).
//!
//! Loads the baked Manhattan walk graph, fixed-camera layer, and ACE bus
//! corridors. Click a start (A) and destination (B); the route is computed once,
//! then exposure across all sensing classes (fixed CCTV + ACE buses + dashcams +
//! smart glasses) is evaluated over the walk on a clock. Scenario sliders and the
//! departure hour re-evaluate the existing route live.

mod ui;
mod world;

use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::math::primitives::Circle;
use bevy::prelude::*;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};

use sim_core::assets::{AceCorridorLayer, FixedSensorLayer, GraphAsset};
use sim_core::simulation::SimParams;
use sim_core::{
    AceConfig, DashcamConfig, FixedCameraDefaults, GlassesConfig, MobileScenario, Route,
    RouteSummary, StreetGraph, Vec2 as Enu,
};

const WALK_SPEED: f64 = sim_core::graph::DEFAULT_WALK_SPEED_MPS;

const GRAPH_PATH: &str = "assets/processed/graph_manhattan.postcard";
const CAMERAS_PATH: &str = "assets/processed/cameras_fixed.postcard";
const ACE_PATH: &str = "assets/processed/ace_corridors.postcard";

// ---------------------------------------------------------------- resources ---

/// The loaded simulation world (routing graph + placed sensors + ACE corridors).
#[derive(Resource)]
pub struct Sim {
    pub graph: StreetGraph,
    pub sensors: Vec<sim_core::SensorInstance>,
    pub layer: FixedSensorLayer,
    pub ace_segments: Vec<[Enu; 2]>,
    pub ace_routes: Vec<String>,
}

#[derive(Resource, Default)]
pub struct RouteState {
    pub a: Option<Enu>,
    pub b: Option<Enu>,
    pub route: Option<Route>,
    pub summary: Option<RouteSummary>,
    pub status: String,
}

/// User-tunable scenario controls.
#[derive(Resource)]
pub struct Params {
    pub show_fov: bool,
    pub show_ace: bool,
    pub departure_hour: f32,
    pub ace_on: bool,
    pub dashcam_on: bool,
    pub glasses_on: bool,
    pub dashcam_penetration: f32,
    pub glasses_per_1000: f32,
}
impl Default for Params {
    fn default() -> Self {
        Params {
            show_fov: false,
            show_ace: true,
            departure_hour: 17.0,
            ace_on: true,
            dashcam_on: true,
            glasses_on: true,
            dashcam_penetration: 0.40,
            glasses_per_1000: 10.0,
        }
    }
}

#[derive(Resource, Default)]
pub struct EguiWantsPointer(pub bool);

#[derive(Resource, Default)]
pub struct ResetRequested(pub bool);

// --------------------------------------------------------------- components ---

#[derive(Component)]
struct MapEntity;
#[derive(Component)]
struct FovWedge;
#[derive(Component)]
struct AceVis;
#[derive(Component)]
struct RouteVis;
#[derive(Component)]
struct Walker {
    progress_m: f64,
}

// --------------------------------------------------------------------- main ---

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "our-space — Manhattan sensing exposure".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.08)))
        .init_resource::<RouteState>()
        .init_resource::<Params>()
        .init_resource::<EguiWantsPointer>()
        .init_resource::<ResetRequested>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                camera_control,
                handle_click,
                recompute_on_change,
                animate_walker,
                sync_layer_visibility,
                apply_reset,
                smoke_exit,
            ),
        )
        .add_systems(EguiPrimaryContextPass, ui::ui_panel)
        .run();
}

// ----------------------------------------------------------------- helpers ----

fn build_mobile(params: &Params, sim: &Sim) -> MobileScenario {
    MobileScenario {
        ace: if params.ace_on && !sim.ace_segments.is_empty() {
            Some(AceConfig::new(sim.ace_segments.clone()))
        } else {
            None
        },
        dashcam: if params.dashcam_on {
            Some(DashcamConfig {
                penetration: params.dashcam_penetration as f64,
                ..Default::default()
            })
        } else {
            None
        },
        glasses: if params.glasses_on {
            Some(GlassesConfig {
                per_1000_pedestrians: params.glasses_per_1000 as f64,
                ..Default::default()
            })
        } else {
            None
        },
    }
}

fn sim_params(sim: &Sim) -> SimParams {
    SimParams {
        recall_factor: 1.0 / sim.layer.recall.unwrap_or(1.0),
        speed_mps: WALK_SPEED,
        dt: 1.0,
    }
}

// ------------------------------------------------------------------- startup --

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut route: ResMut<RouteState>,
) {
    commands.spawn((Camera2d, Transform::from_scale(Vec3::splat(6.0))));

    let graph_bytes = std::fs::read(GRAPH_PATH)
        .unwrap_or_else(|e| panic!("could not read {GRAPH_PATH} ({e}). Bake assets first (see README)."));
    let graph = StreetGraph::from_asset(GraphAsset::from_bytes(&graph_bytes).expect("decoding graph"));

    let cam_bytes = std::fs::read(CAMERAS_PATH)
        .unwrap_or_else(|e| panic!("could not read {CAMERAS_PATH} ({e}). Bake assets first."));
    let layer = FixedSensorLayer::from_bytes(&cam_bytes).expect("decoding camera layer");
    let sensors = sim_core::sensors_from_layer(&layer, FixedCameraDefaults::default());

    // Optional ACE corridors.
    let (ace_segments, ace_routes): (Vec<[Enu; 2]>, Vec<String>) = std::fs::read(ACE_PATH)
        .ok()
        .and_then(|b| AceCorridorLayer::from_bytes(&b).ok())
        .map(|l| {
            let segs = l
                .segments
                .iter()
                .map(|s| [Enu::new(s[0][0], s[0][1]), Enu::new(s[1][0], s[1][1])])
                .collect();
            (segs, l.routes)
        })
        .unwrap_or_default();

    // Streets.
    let street_mesh = meshes.add(world::line_list_mesh(world::street_line_positions(graph.asset())));
    let street_mat = materials.add(Color::srgb(0.26, 0.28, 0.34));
    commands.spawn((
        Mesh2d(street_mesh),
        MeshMaterial2d(street_mat),
        Transform::from_xyz(0.0, 0.0, 0.0),
        MapEntity,
    ));

    // ACE corridors (teal), above streets.
    if !ace_segments.is_empty() {
        let mut pos = Vec::with_capacity(ace_segments.len() * 2);
        for [a, b] in &ace_segments {
            pos.push([a.x as f32, a.y as f32, 0.2]);
            pos.push([b.x as f32, b.y as f32, 0.2]);
        }
        let ace_mesh = meshes.add(world::line_list_mesh(pos));
        let ace_mat = materials.add(Color::srgb(0.20, 0.85, 0.60));
        commands.spawn((
            Mesh2d(ace_mesh),
            MeshMaterial2d(ace_mat),
            Transform::from_xyz(0.0, 0.0, 0.2),
            MapEntity,
            AceVis,
        ));
    }

    // Camera dots + translucent FOV wedges.
    let cam_circle = meshes.add(Circle::new(11.0));
    let cam_mat = materials.add(Color::srgb(0.95, 0.27, 0.27));
    let wedge_mat = materials.add(Color::srgba(0.95, 0.4, 0.25, 0.10));
    for s in &sensors {
        let apex = s.wedge.apex;
        commands.spawn((
            Mesh2d(cam_circle.clone()),
            MeshMaterial2d(cam_mat.clone()),
            Transform::from_translation(world::to_world(apex, 1.0)),
            MapEntity,
        ));
        let wedge = meshes.add(world::wedge_mesh(
            s.wedge.heading_rad as f32,
            s.wedge.half_fov_rad as f32,
            s.wedge.range_m as f32,
            16,
        ));
        commands.spawn((
            Mesh2d(wedge),
            MeshMaterial2d(wedge_mat.clone()),
            Transform::from_translation(world::to_world(apex, 0.5)),
            MapEntity,
            FovWedge,
        ));
    }

    info!(
        "loaded {} nodes / {} edges, {} cameras, {} ACE segments ({} routes)",
        graph.node_count(),
        graph.edge_count(),
        sensors.len(),
        ace_segments.len(),
        ace_routes.len(),
    );
    route.status = "Click the map to set a start point (A).".into();

    if std::env::var("OURSPACE_SMOKE").is_ok() {
        let _ = std::fs::write(
            "/tmp/ourspace_setup.txt",
            format!(
                "setup_ok nodes={} cameras={} ace_segments={}\n",
                graph.node_count(),
                sensors.len(),
                ace_segments.len()
            ),
        );
    }

    commands.insert_resource(Sim {
        graph,
        sensors,
        layer,
        ace_segments,
        ace_routes,
    });
}

// ----------------------------------------------------------------- systems ----

/// Right-drag to pan, scroll to zoom (no built-in PanCamera in Bevy 0.18).
fn camera_control(
    mut scroll: MessageReader<MouseWheel>,
    mut motion: MessageReader<MouseMotion>,
    buttons: Res<ButtonInput<MouseButton>>,
    wants: Res<EguiWantsPointer>,
    mut q: Query<&mut Transform, With<Camera2d>>,
) {
    let Ok(mut t) = q.single_mut() else {
        return;
    };

    let mut zoom = 0.0f32;
    for e in scroll.read() {
        zoom += e.y;
    }
    if zoom != 0.0 && !wants.0 {
        let factor = (1.0 - zoom * 0.12).clamp(0.6, 1.6);
        t.scale = (t.scale * Vec3::new(factor, factor, 1.0)).clamp(Vec3::splat(0.3), Vec3::splat(40.0));
    }

    if buttons.pressed(MouseButton::Right) {
        let mut delta = Vec2::ZERO;
        for e in motion.read() {
            delta += e.delta;
        }
        t.translation.x -= delta.x * t.scale.x;
        t.translation.y += delta.y * t.scale.y;
    } else {
        motion.clear();
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_click(
    buttons: Res<ButtonInput<MouseButton>>,
    wants: Res<EguiWantsPointer>,
    params: Res<Params>,
    windows: Query<&Window>,
    cam_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    sim: Option<Res<Sim>>,
    mut route: ResMut<RouteState>,
    route_vis: Query<Entity, With<RouteVis>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if wants.0 || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(sim) = sim else { return };
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else { return };
    let Ok((cam, cam_t)) = cam_q.single() else { return };
    let Ok(world) = cam.viewport_to_world_2d(cam_t, cursor) else { return };
    let enu = Enu::new(world.x as f64, world.y as f64);

    if route.a.is_none() || route.b.is_some() {
        for e in &route_vis {
            commands.entity(e).despawn();
        }
        *route = RouteState {
            a: Some(enu),
            status: "Click again to set the destination (B).".into(),
            ..default()
        };
        spawn_marker(&mut commands, &mut meshes, &mut materials, enu, Color::srgb(0.25, 0.95, 0.45));
        return;
    }

    route.b = Some(enu);
    spawn_marker(&mut commands, &mut meshes, &mut materials, enu, Color::srgb(0.95, 0.3, 0.85));

    let a = route.a.unwrap();
    let mobile = build_mobile(&params, &sim);
    match sim_core::run_route(
        &sim.graph,
        &sim.sensors,
        &[],
        &mobile,
        a,
        enu,
        sim_params(&sim),
        params.departure_hour as f64,
    ) {
        Ok((r, summary)) => {
            let line = meshes.add(world::line_strip_mesh(&r.points, 2.0));
            let line_mat = materials.add(Color::srgb(0.25, 0.8, 1.0));
            commands.spawn((Mesh2d(line), MeshMaterial2d(line_mat), Transform::default(), RouteVis));

            let walker = meshes.add(Circle::new(16.0));
            let walker_mat = materials.add(Color::srgb(1.0, 0.95, 0.25));
            commands.spawn((
                Mesh2d(walker),
                MeshMaterial2d(walker_mat),
                Transform::from_translation(world::to_world(r.position_at(0.0), 4.0)),
                RouteVis,
                Walker { progress_m: 0.0 },
            ));

            route.status = "Walking the route…".into();
            route.route = Some(r);
            route.summary = Some(summary);
        }
        Err(e) => route.status = format!("No walkable route found ({e})."),
    }
}

/// Re-evaluate the existing route when scenario sliders / hour change.
fn recompute_on_change(params: Res<Params>, sim: Option<Res<Sim>>, mut route: ResMut<RouteState>) {
    if !params.is_changed() {
        return;
    }
    let Some(sim) = sim else { return };
    let Some(r) = route.route.clone() else { return };
    let mobile = build_mobile(&params, &sim);
    let summary = sim_core::summarize(&r, &sim.sensors, &[], &mobile, sim_params(&sim), params.departure_hour as f64);
    route.summary = Some(summary);
}

fn spawn_marker(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    enu: Enu,
    color: Color,
) {
    let mesh = meshes.add(Circle::new(17.0));
    let mat = materials.add(color);
    commands.spawn((
        Mesh2d(mesh),
        MeshMaterial2d(mat),
        Transform::from_translation(world::to_world(enu, 3.0)),
        RouteVis,
    ));
}

fn animate_walker(time: Res<Time>, route: Res<RouteState>, mut q: Query<(&mut Transform, &mut Walker)>) {
    let Some(r) = &route.route else { return };
    if r.total_m <= 0.0 {
        return;
    }
    for (mut t, mut w) in &mut q {
        w.progress_m += WALK_SPEED * time.delta_secs_f64();
        if w.progress_m > r.total_m {
            w.progress_m = 0.0;
        }
        t.translation = world::to_world(r.position_at(w.progress_m), 4.0);
    }
}

fn sync_layer_visibility(
    params: Res<Params>,
    mut fov: Query<&mut Visibility, (With<FovWedge>, Without<AceVis>)>,
    mut ace: Query<&mut Visibility, (With<AceVis>, Without<FovWedge>)>,
) {
    let fov_target = if params.show_fov { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in fov.iter_mut() {
        if *v != fov_target {
            *v = fov_target;
        }
    }
    let ace_target = if params.show_ace { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in ace.iter_mut() {
        if *v != ace_target {
            *v = ace_target;
        }
    }
}

fn apply_reset(
    mut reset: ResMut<ResetRequested>,
    mut route: ResMut<RouteState>,
    q: Query<Entity, With<RouteVis>>,
    mut commands: Commands,
) {
    if !reset.0 {
        return;
    }
    reset.0 = false;
    for e in &q {
        commands.entity(e).despawn();
    }
    *route = RouteState {
        status: "Click the map to set a start point (A).".into(),
        ..default()
    };
}

/// In `OURSPACE_SMOKE` mode, exit after a few rendered frames so headless runs
/// can confirm the render loop ticked without panicking.
fn smoke_exit(mut frames: Local<u32>, mut exit: MessageWriter<AppExit>) {
    if std::env::var("OURSPACE_SMOKE").is_err() {
        return;
    }
    *frames += 1;
    if *frames == 8 {
        let _ = std::fs::write("/tmp/ourspace_frames.txt", format!("frames_ok={}\n", *frames));
        exit.write(AppExit::Success);
    }
}
