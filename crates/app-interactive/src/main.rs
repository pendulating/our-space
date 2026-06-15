//! Interactive our-space app (native dev window; WebGPU WASM target in Phase 4).
//!
//! Loads the baked Manhattan walk graph + fixed-camera layer, lets you click a
//! start (A) and destination (B), routes between them, walks the route on a
//! clock, and reports "how many cameras could have captured you".

mod ui;
mod world;

use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::math::primitives::Circle;
use bevy::prelude::*;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};

use sim_core::assets::{FixedSensorLayer, GraphAsset};
use sim_core::simulation::SimParams;
use sim_core::{FixedCameraDefaults, Route, RouteSummary, StreetGraph, Vec2 as Enu};

const WALK_SPEED: f64 = sim_core::graph::DEFAULT_WALK_SPEED_MPS;

const GRAPH_PATH: &str = "assets/processed/graph_manhattan.postcard";
const CAMERAS_PATH: &str = "assets/processed/cameras_fixed.postcard";

// ---------------------------------------------------------------- resources ---

/// The loaded simulation world (routing graph + placed sensors + provenance).
#[derive(Resource)]
pub struct Sim {
    pub graph: StreetGraph,
    pub sensors: Vec<sim_core::SensorInstance>,
    pub layer: FixedSensorLayer,
}

#[derive(Resource, Default)]
pub struct RouteState {
    pub a: Option<Enu>,
    pub b: Option<Enu>,
    pub route: Option<Route>,
    pub summary: Option<RouteSummary>,
    pub status: String,
}

#[derive(Resource)]
pub struct Params {
    pub show_fov: bool,
}
impl Default for Params {
    fn default() -> Self {
        Params { show_fov: true }
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
/// Route-specific visuals (route line, A/B markers, walker) — despawned on reset.
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
                animate_walker,
                sync_fov_visibility,
                apply_reset,
                smoke_exit,
            ),
        )
        .add_systems(EguiPrimaryContextPass, ui::ui_panel)
        .run();
}

// ------------------------------------------------------------------- startup --

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut route: ResMut<RouteState>,
) {
    // ENU meters == world units. Zoom out so most of Manhattan is visible.
    commands.spawn((Camera2d, Transform::from_scale(Vec3::splat(6.0))));

    let graph_bytes = std::fs::read(GRAPH_PATH)
        .unwrap_or_else(|e| panic!("could not read {GRAPH_PATH} ({e}). Bake assets first (see README)."));
    let asset = GraphAsset::from_bytes(&graph_bytes).expect("decoding graph asset");
    let graph = StreetGraph::from_asset(asset);

    let cam_bytes = std::fs::read(CAMERAS_PATH)
        .unwrap_or_else(|e| panic!("could not read {CAMERAS_PATH} ({e}). Bake assets first (see README)."));
    let layer = FixedSensorLayer::from_bytes(&cam_bytes).expect("decoding camera layer");
    let sensors = sim_core::sensors_from_layer(&layer, FixedCameraDefaults::default());

    // Streets as one static line mesh.
    let street_mesh = meshes.add(world::line_list_mesh(world::street_line_positions(graph.asset())));
    let street_mat = materials.add(Color::srgb(0.28, 0.30, 0.36));
    commands.spawn((
        Mesh2d(street_mesh),
        MeshMaterial2d(street_mat),
        Transform::from_xyz(0.0, 0.0, 0.0),
        MapEntity,
    ));

    // Cameras as red dots + translucent FOV wedges.
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
        "loaded {} nodes / {} edges, {} cameras",
        graph.node_count(),
        graph.edge_count(),
        sensors.len()
    );
    route.status = "Click the map to set a start point (A).".into();

    // Smoke-test sentinel (bypasses stdout buffering): reaching Startup means the
    // window + renderer initialized and assets loaded without panicking.
    if std::env::var("OURSPACE_SMOKE").is_ok() {
        let _ = std::fs::write(
            "/tmp/ourspace_setup.txt",
            format!(
                "setup_ok nodes={} edges={} cameras={}\n",
                graph.node_count(),
                graph.edge_count(),
                sensors.len()
            ),
        );
    }

    commands.insert_resource(Sim { graph, sensors, layer });
}

/// In `OURSPACE_SMOKE` mode, exit after a few rendered frames so CI/headless
/// runs can confirm the render loop ticked without panicking.
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
        t.scale = (t.scale * Vec3::new(factor, factor, 1.0))
            .clamp(Vec3::splat(0.3), Vec3::splat(40.0));
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

    // State machine: A -> B -> (next click) reset to A.
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

    // Second click: set B and route.
    route.b = Some(enu);
    spawn_marker(&mut commands, &mut meshes, &mut materials, enu, Color::srgb(0.95, 0.3, 0.85));

    let a = route.a.unwrap();
    let params = SimParams {
        recall_factor: 1.0 / sim.layer.recall.unwrap_or(1.0),
        speed_mps: WALK_SPEED,
        dt: 1.0,
    };
    match sim_core::run_route(&sim.graph, &sim.sensors, &[], a, enu, params) {
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
        Err(e) => {
            route.status = format!("No walkable route found ({e}).");
        }
    }
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

fn animate_walker(
    time: Res<Time>,
    route: Res<RouteState>,
    mut q: Query<(&mut Transform, &mut Walker)>,
) {
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

fn sync_fov_visibility(params: Res<Params>, mut q: Query<&mut Visibility, With<FovWedge>>) {
    let target = if params.show_fov {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut q {
        if *v != target {
            *v = target;
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
