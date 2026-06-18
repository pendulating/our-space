//! Interactive our-space app (native dev window; WebGPU WASM target in Phase 4).
//!
//! Loads the baked Manhattan walk graph, fixed-camera layer, and ACE bus
//! corridors. Click a start (A) and destination (B); the route is computed once,
//! then exposure across all sensing classes (fixed CCTV + ACE buses + dashcams +
//! smart glasses) is evaluated over the walk on a clock. Scenario sliders and the
//! departure hour re-evaluate the existing route live.

mod agents;
#[cfg(target_arch = "wasm32")]
mod basemap;
mod loading;
mod ui;
mod world;

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::math::primitives::Circle;
use bevy::prelude::*;
use bevy::window::CursorMoved;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};

use bevy::math::primitives::Annulus;
use rstar::primitives::GeomWithData;
use rstar::RTree;
use agents::AgentPool;
use loading::{
    AceRes, AlprRes, CamerasRes, DashcamFieldRes, DotRes, EquityRes, GraphAssetRes, HeatmapRes,
    LoadingHandles, VehicleRoutesRes,
};
use sim_core::assets::{DashcamFieldLayer, EquityLayer, FixedSensorLayer, HeatmapLayer};
use sim_core::simulation::SimParams;
use sim_core::{
    AceConfig, DashcamConfig, FixedCameraDefaults, GlassesConfig, MobileScenario, Route,
    RouteSummary, StreetGraph, Vec2 as Enu,
};

const WALK_SPEED: f64 = sim_core::graph::DEFAULT_WALK_SPEED_MPS;

/// Playback-only speed-up for the walker dot. The exposure estimate and walkshed
/// are computed at the true `WALK_SPEED` (1.34 m/s); this multiplier *only*
/// scales the on-screen animation so a 15-minute walk doesn't take 15 minutes to
/// watch (≈40× → ~22 s of playback for a 15-min route). It touches no numbers.
/// Shared by the ambient agents so the whole scene runs on one time-lapse clock.
pub(crate) const ANIM_SPEEDUP: f64 = 40.0;

// Zoom feel: gentle multiplicative zoom per normalized scroll notch.
const ZOOM_PER_NOTCH: f32 = 0.06;
const ZOOM_PIXEL_DIVISOR: f32 = 160.0;
const ZOOM_MIN: f32 = 0.4;
const ZOOM_MAX: f32 = 30.0;

// Paths relative to the AssetServer root (`assets/`); works native + web.
// Distinct extensions disambiguate the per-type postcard loaders.
const GRAPH_PATH: &str = "processed/graph_manhattan.osgraph";
const CAMERAS_PATH: &str = "processed/cameras_fixed.oscam";
const ACE_PATH: &str = "processed/ace_corridors.osace";
const HEATMAP_PATH: &str = "processed/heatmap.osheat";
const EQUITY_PATH: &str = "processed/equity.osequity";
const DASHCAM_FIELD_PATH: &str = "processed/dashcam_field.osfield";
const ALPR_PATH: &str = "processed/alpr.osalpr";
const DOT_PATH: &str = "processed/dot_cameras.osdot";
const VEHICLE_ROUTES_PATH: &str = "processed/vehicle_routes.osroutes";

/// Max Shannon entropy over 5 groups = ln(5), for normalizing the choropleth.
const MAX_ENTROPY: f64 = 1.6094379;

/// Interaction mode: route between two points, or a one-point walkshed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Route,
    Walkshed,
}

/// Walkshed time budget (seconds) — a 10-minute walk.
const WALKSHED_SECONDS: f64 = 600.0;

/// Which heatmap layer to display.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HeatClass {
    Total,
    Fixed,
    Ace,
    Dashcam,
}
impl HeatClass {
    pub fn label(self) -> &'static str {
        match self {
            HeatClass::Total => "All sources",
            HeatClass::Fixed => "Fixed cameras (CCTV + ALPR)",
            HeatClass::Ace => "ACE buses",
            HeatClass::Dashcam => "Rideshare cams",
        }
    }
}

// ---------------------------------------------------------------- resources ---

/// The loaded simulation world (routing graph + placed sensors + ACE corridors).
#[derive(Resource)]
pub struct Sim {
    pub graph: StreetGraph,
    pub sensors: Vec<sim_core::SensorInstance>,
    pub layer: FixedSensorLayer,
    pub ace_segments: Vec<[Enu; 2]>,
    pub ace_routes: Vec<String>,
    pub heatmap: Option<HeatmapLayer>,
    pub equity: Option<EquityLayer>,
    /// Pearson r between block-group diversity entropy and detected camera count.
    pub equity_corr: Option<f64>,
    /// Spatial rideshare-camera density field (from real TLC trips).
    pub dashcam_field: DashcamFieldLayer,
    /// Baked weighted pool of vehicle (rideshare) routes for the dashcam agents.
    pub vehicle_routes: Vec<sim_core::assets::VehicleRoute>,
    /// ACE bus route shapes (ordered polylines) for the running-bus agents.
    pub ace_routes_geom: Vec<Route>,
    /// R-tree over fixed-camera apex positions (data = sensor id) so the live
    /// walk tally tests only nearby cameras, not all ~5k every frame.
    pub cam_index: RTree<GeomWithData<[f64; 2], u64>>,
    /// Squared query radius (m²) = (max camera range + margin)² for the cull.
    pub cam_query_r2: f64,
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
    pub heatmap_on: bool,
    pub heatmap_class: HeatClass,
    pub equity_on: bool,
    pub mode: Mode,
    /// Show the ambient moving agents (rideshare dashcams + glasses pedestrians).
    pub show_agents: bool,
    /// Which exposure figure the panel headlines (see [`ExposureMode`]).
    pub exposure_mode: ExposureMode,
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
            heatmap_on: false,
            heatmap_class: HeatClass::Fixed,
            equity_on: false,
            mode: Mode::Route,
            show_agents: true,
            exposure_mode: ExposureMode::Analytical,
        }
    }
}

/// Which exposure figure the panel reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExposureMode {
    /// Deterministic Poisson-field expectation — reproducible, the citable estimate.
    Analytical,
    /// A single stochastic walk: the moving agents that actually pass you this
    /// playback increment a live tally (a Monte-Carlo sample of the same model).
    Narrative,
}

/// Whether egui currently wants pointer / keyboard input (so map controls yield).
#[derive(Resource, Default)]
pub struct EguiWants {
    pub pointer: bool,
    pub keyboard: bool,
}

/// Tracks an in-progress left-drag so we can tell a pan from a click.
#[derive(Resource, Default)]
pub struct DragState {
    pub moved_px: f32,
    pub last_cursor: Option<Vec2>,
}

#[derive(Resource, Default)]
pub struct ResetRequested(pub bool);

/// Live tally for the animated walk: distinct cameras the walker has passed
/// through this loop (climbs, resets each pass).
#[derive(Resource, Default)]
pub struct WalkLive {
    pub seen: std::collections::HashSet<u64>,
    pub count: u32,
    /// Narrative mode: stochastic rideshare-dashcam encounters this pass.
    pub mobile_vehicle: u32,
    /// Narrative mode: stochastic smart-glasses encounters this pass.
    pub mobile_glasses: u32,
    /// Narrative mode: stochastic ACE-bus encounters this pass.
    pub mobile_bus: u32,
    pub last_progress: f64,
}

/// The current one-point walkshed result (for the panel).
#[derive(Resource, Default)]
pub struct WalkshedState {
    pub summary: Option<sim_core::WalkshedSummary>,
}

// --------------------------------------------------------------- components ---

#[derive(Component)]
struct BaseMap; // streets + camera dots (hidden in heatmap mode)
#[derive(Component)]
struct FovWedge;
#[derive(Component)]
struct AceVis;
#[derive(Component)]
struct HeatmapVis;
#[derive(Component)]
struct EquityVis;
#[derive(Component)]
struct RouteVis;
/// Walkshed visuals (reachable streets + in-shed camera rings + center) — cleared per query.
#[derive(Component)]
struct WalkshedVis;
/// A transient "you were seen" pulse overlay (pooled), placed on a fixed camera
/// the moment the live walk passes it. `life` counts down 1→0.
#[derive(Component)]
struct Ping {
    life: f32,
}
/// Fixed pool of pulse entities, recycled (no per-capture spawn/despawn).
#[derive(Resource, Default)]
struct PingPool {
    entities: Vec<Entity>,
}
#[derive(Component)]
struct Walker {
    progress_m: f64,
}

// --------------------------------------------------------------------- main ---

fn main() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "our-space — Manhattan sensing exposure".into(),
            // Web: bind to the page's canvas and fill its parent. Ignored natively.
            canvas: Some("#bevy-canvas".into()),
            fit_canvas_to_parent: true,
            prevent_default_event_handling: true,
            // Web: transparent canvas so the MapLibre basemap shows through behind
            // the sim layers. Native keeps an opaque parchment background.
            transparent: cfg!(target_arch = "wasm32"),
            #[cfg(target_arch = "wasm32")]
            composite_alpha_mode: bevy::window::CompositeAlphaMode::PreMultiplied,
            ..default()
        }),
        ..default()
    }))
    .add_plugins(EguiPlugin::default())
    // Transparent on web (basemap behind); warm parchment on native.
    .insert_resource(ClearColor(if cfg!(target_arch = "wasm32") {
        Color::NONE
    } else {
        Color::srgb_u8(0xe7, 0xdc, 0xc4)
    }))
    .init_resource::<RouteState>()
    .init_resource::<Params>()
    .init_resource::<EguiWants>()
    .init_resource::<DragState>()
    .init_resource::<ResetRequested>()
    .init_resource::<WalkLive>()
    .init_resource::<WalkshedState>()
    .init_resource::<PingPool>()
    .insert_resource(AgentPool::empty())
    .add_systems(Startup, start_loading)
    .add_systems(
        Update,
        (
            build_world,
            camera_control,
            handle_click,
            recompute_on_change,
            animate_walker,
            (walk_capture_events, agents::mobile_capture_events).chain(),
            decay_pings,
            agents::scale_agent_population,
            agents::animate_agents,
            sync_mode,
            sync_visibility,
            rebuild_heatmap,
            rebuild_equity,
            apply_reset,
            smoke_exit,
        ),
    )
    .add_systems(EguiPrimaryContextPass, ui::ui_panel);
    // Web: keep the MapLibre basemap synced to the Bevy camera each frame.
    #[cfg(target_arch = "wasm32")]
    app.add_systems(Update, basemap::sync_basemap);
    loading::register(&mut app);
    app.run();
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

/// Spawn the camera and request the baked layers via the AssetServer.
fn start_loading(mut commands: Commands, asset_server: Res<AssetServer>, mut route: ResMut<RouteState>) {
    commands.spawn((Camera2d, Transform::from_scale(Vec3::splat(6.0))));
    commands.insert_resource(LoadingHandles {
        graph: asset_server.load(GRAPH_PATH),
        cameras: asset_server.load(CAMERAS_PATH),
        ace: asset_server.load(ACE_PATH),
        heatmap: asset_server.load(HEATMAP_PATH),
        equity: asset_server.load(EQUITY_PATH),
        dashcam: asset_server.load(DASHCAM_FIELD_PATH),
        alpr: asset_server.load(ALPR_PATH),
        dot: asset_server.load(DOT_PATH),
        vehicle_routes: asset_server.load(VEHICLE_ROUTES_PATH),
        built: false,
    });
    route.status = "Loading Manhattan map data…".into();
}

/// Once all baked layers have loaded, build the simulation world + map meshes.
#[allow(clippy::too_many_arguments)]
fn build_world(
    mut handles: ResMut<LoadingHandles>,
    graphs: Res<Assets<GraphAssetRes>>,
    cams: Res<Assets<CamerasRes>>,
    aces: Res<Assets<AceRes>>,
    heatmaps: Res<Assets<HeatmapRes>>,
    equities: Res<Assets<EquityRes>>,
    dashcams: Res<Assets<DashcamFieldRes>>,
    alprs: Res<Assets<AlprRes>>,
    dots: Res<Assets<DotRes>>,
    vroutes: Res<Assets<VehicleRoutesRes>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut route: ResMut<RouteState>,
) {
    if handles.built {
        return;
    }
    let (Some(g), Some(c), Some(a), Some(h), Some(e), Some(df), Some(al), Some(dot), Some(vr)) = (
        graphs.get(&handles.graph),
        cams.get(&handles.cameras),
        aces.get(&handles.ace),
        heatmaps.get(&handles.heatmap),
        equities.get(&handles.equity),
        dashcams.get(&handles.dashcam),
        alprs.get(&handles.alpr),
        dots.get(&handles.dot),
        vroutes.get(&handles.vehicle_routes),
    ) else {
        return; // still loading
    };

    let graph = StreetGraph::from_asset(g.0.clone());
    let layer = c.0.clone();
    // Combine fixed-camera layers: Dahir CCTV + DeFlock ALPRs + NYC DOT traffic
    // cameras, re-indexing ids to the combined-vector position (used as the
    // distinct-device key + dot index). DOT cams use monitoring defaults
    // (omnidirectional, wider reach, low frame rate).
    let mut sensors = sim_core::sensors_from_layer(&layer, FixedCameraDefaults::default());
    let cctv_count = sensors.len();
    sensors.extend(sim_core::sensors_from_layer(&al.0, FixedCameraDefaults::default()));
    let alpr_count = sensors.len() - cctv_count;
    sensors.extend(sim_core::sensors_from_layer(&dot.0, FixedCameraDefaults::dot_monitoring()));
    let dot_count = sensors.len() - cctv_count - alpr_count;
    for (i, s) in sensors.iter_mut().enumerate() {
        s.id = i as u64;
    }
    let ace_segments: Vec<[Enu; 2]> = a
        .0
        .segments
        .iter()
        .map(|s| [Enu::new(s[0][0], s[0][1]), Enu::new(s[1][0], s[1][1])])
        .collect();
    let ace_routes = a.0.routes.clone();
    // ACE route shapes -> Route geometry for the running-bus agents.
    let ace_routes_geom: Vec<Route> = a
        .0
        .polylines
        .iter()
        .filter(|p| p.points.len() >= 2)
        .map(|p| {
            Route::from_points(p.points.iter().map(|q| Enu::new(q[0] as f64, q[1] as f64)).collect())
        })
        .collect();
    let heatmap = Some(h.0.clone());
    let equity = Some(e.0.clone());
    let dashcam_field = df.0.clone();
    let equity_corr = {
        let xs: Vec<f64> = e.0.block_groups.iter().filter(|b| b.population > 0).map(|b| b.entropy).collect();
        let ys: Vec<f64> = e.0.block_groups.iter().filter(|b| b.population > 0).map(|b| b.camera_count as f64).collect();
        pearson(&xs, &ys)
    };

    // Streets.
    let street_mesh = meshes.add(world::line_list_mesh(world::street_line_positions(graph.asset())));
    let street_mat = materials.add(Color::srgb_u8(0x8a, 0x75, 0x50)); // warm ink linework
    commands.spawn((
        Mesh2d(street_mesh),
        MeshMaterial2d(street_mat),
        Transform::from_xyz(0.0, 0.0, 0.0),
        BaseMap,
    ));

    // ACE corridors (teal), above streets.
    if !ace_segments.is_empty() {
        let mut pos = Vec::with_capacity(ace_segments.len() * 2);
        for [a, b] in &ace_segments {
            pos.push([a.x as f32, a.y as f32, 0.2]);
            pos.push([b.x as f32, b.y as f32, 0.2]);
        }
        let ace_mesh = meshes.add(world::line_list_mesh(pos));
        let ace_mat = materials.add(Color::srgb_u8(0x72, 0x87, 0xa4)); // cold steel corridor
        commands.spawn((
            Mesh2d(ace_mesh),
            MeshMaterial2d(ace_mat),
            Transform::from_xyz(0.0, 0.0, 0.2),
            AceVis,
        ));
    }

    // Camera markers: one MERGED textured-quad mesh per class, each painting a
    // recognizable icon (CCTV camera, owl for Flock/ALPR, traffic-cam for DOT)
    // via a single ColorMaterial+texture — 3 draw calls for ~5k cameras, not one
    // entity each. FOV wedges stay per-camera (directional only, hidden unless
    // toggled, so cheap).
    let mut cctv_pts: Vec<Enu> = Vec::new();
    let mut alpr_pts: Vec<Enu> = Vec::new();
    let mut dot_pts: Vec<Enu> = Vec::new();
    let wedge_mat = materials.add(Color::srgba(0.11, 0.21, 0.40, 0.34)); // cold projected cone
    for s in &sensors {
        match s.kind {
            sim_core::SourceKind::Alpr => alpr_pts.push(s.wedge.apex),
            sim_core::SourceKind::DotLiveView => dot_pts.push(s.wedge.apex),
            _ => cctv_pts.push(s.wedge.apex),
        }
        // Directional sensors get a cone; omnidirectional (DOT PTZ, heading-less
        // ALPR) draw none (a 30 m disc would bury the map). Wedges default hidden.
        if s.wedge.half_fov_rad < std::f64::consts::PI {
            let wedge = meshes.add(world::wedge_mesh(
                s.wedge.heading_rad as f32,
                s.wedge.half_fov_rad as f32,
                s.wedge.range_m as f32,
                16,
            ));
            commands.spawn((
                Mesh2d(wedge),
                MeshMaterial2d(wedge_mat.clone()),
                Transform::from_translation(world::to_world(s.wedge.apex, 0.5)),
                FovWedge,
            ));
        }
    }
    for (pts, size, icon) in [
        (&cctv_pts, 26.0_f32, "icons/cctv.png"),
        (&alpr_pts, 28.0, "icons/owl.png"),
        (&dot_pts, 28.0, "icons/dot.png"),
    ] {
        if pts.is_empty() {
            continue;
        }
        let mesh = meshes.add(world::merged_icon_quads(pts, size));
        let mat = materials.add(ColorMaterial {
            color: Color::WHITE,
            texture: Some(asset_server.load(icon)),
            ..default()
        });
        commands.spawn((
            Mesh2d(mesh),
            MeshMaterial2d(mat),
            Transform::from_xyz(0.0, 0.0, 1.0),
            BaseMap,
        ));
    }

    info!(
        "loaded {} nodes / {} edges, {} CCTV + {} ALPR + {} DOT cameras, {} ACE segments ({} routes)",
        graph.node_count(),
        graph.edge_count(),
        cctv_count,
        alpr_count,
        dot_count,
        ace_segments.len(),
        ace_routes.len(),
    );
    route.status = "Click the map to set a start point (A).".into();
    info!("equity: diversity ~ detected cameras  r = {:?}", equity_corr);

    // R-tree over camera apexes so the per-frame live walk tally tests only
    // cameras near the walker (not all ~5k). Query radius = max range + margin.
    let cam_index: RTree<GeomWithData<[f64; 2], u64>> = RTree::bulk_load(
        sensors
            .iter()
            .map(|s| GeomWithData::new([s.wedge.apex.x, s.wedge.apex.y], s.id))
            .collect(),
    );
    let cam_max_range = sensors.iter().map(|s| s.wedge.range_m).fold(0.0_f64, f64::max);
    let cam_query_r2 = (cam_max_range + 2.0).powi(2);

    // Pulse-overlay pool: a handful of recycled "seen you" rings that flash on a
    // camera as the live walk passes it (replaces per-camera flash now that dots
    // are a merged static mesh).
    let ping_mesh = meshes.add(Annulus::new(9.0, 14.0));
    let ping_mat = materials.add(Color::srgb_u8(0xa8, 0x54, 0x1f)); // terracotta accent
    let ping_entities: Vec<Entity> = (0..32)
        .map(|_| {
            commands
                .spawn((
                    Mesh2d(ping_mesh.clone()),
                    MeshMaterial2d(ping_mat.clone()),
                    Transform::from_xyz(0.0, 0.0, 3.5),
                    Visibility::Hidden,
                    Ping { life: 0.0 },
                ))
                .id()
        })
        .collect();
    commands.insert_resource(PingPool { entities: ping_entities });

    // Ambient mobile agents: decode the baked vehicle-route pool and spawn the
    // fixed entity pool (vehicles + pedestrians). Routes also live on `Sim` for
    // runtime weighted resampling on recycle.
    let vehicle_routes = vr.0.routes.clone();
    let glasses_icon = asset_server.load("icons/glasses.png");
    let bus_icon = asset_server.load("icons/bus.png");
    let pool = agents::spawn_pool(
        &mut commands, &mut meshes, &mut materials, &vehicle_routes, glasses_icon, bus_icon,
    );
    commands.insert_resource(pool);

    if std::env::var("OURSPACE_SMOKE").is_ok() {
        let _ = std::fs::write(
            "/tmp/ourspace_setup.txt",
            format!(
                "setup_ok nodes={} cameras={} ace_segments={} vehicle_routes={}\n",
                graph.node_count(),
                sensors.len(),
                ace_segments.len(),
                vehicle_routes.len(),
            ),
        );
    }

    commands.insert_resource(Sim {
        graph,
        sensors,
        layer,
        ace_segments,
        ace_routes,
        heatmap,
        equity,
        equity_corr,
        dashcam_field,
        vehicle_routes,
        ace_routes_geom,
        cam_index,
        cam_query_r2,
    });
    handles.built = true;
}

/// Pearson correlation coefficient, or None if undefined.
fn pearson(xs: &[f64], ys: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 2 || n != ys.len() {
        return None;
    }
    let nf = n as f64;
    let mx = xs.iter().sum::<f64>() / nf;
    let my = ys.iter().sum::<f64>() / nf;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for i in 0..n {
        let (dx, dy) = (xs[i] - mx, ys[i] - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    let denom = (sxx * syy).sqrt();
    if denom <= f64::EPSILON {
        None
    } else {
        Some(sxy / denom)
    }
}

// ----------------------------------------------------------------- systems ----

/// Drag (left or right mouse) to pan, scroll to zoom, WASD/arrows to pan.
#[allow(clippy::too_many_arguments)]
fn camera_control(
    mut scroll: MessageReader<MouseWheel>,
    mut cursor: MessageReader<CursorMoved>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    wants: Res<EguiWants>,
    mut drag: ResMut<DragState>,
    mut q: Query<&mut Transform, With<Camera2d>>,
) {
    let Ok(mut t) = q.single_mut() else {
        return;
    };

    // Zoom (scroll). Normalize wheel "lines" vs trackpad "pixels" so both feel
    // the same, then apply gentle exponential zoom (multiplicative, smooth).
    let mut notches = 0.0f32;
    for e in scroll.read() {
        notches += match e.unit {
            MouseScrollUnit::Line => e.y,
            MouseScrollUnit::Pixel => e.y / ZOOM_PIXEL_DIVISOR,
        };
    }
    if notches != 0.0 && !wants.pointer {
        // Per-frame factor clamped as a rail against multi-event bursts.
        let factor = (-notches * ZOOM_PER_NOTCH).exp().clamp(0.86, 1.16);
        t.scale = (t.scale * Vec3::new(factor, factor, 1.0))
            .clamp(Vec3::splat(ZOOM_MIN), Vec3::splat(ZOOM_MAX));
    }

    // Reset drag distance at the start of a press.
    if buttons.just_pressed(MouseButton::Left) {
        drag.moved_px = 0.0;
    }
    let panning = (buttons.pressed(MouseButton::Left) || buttons.pressed(MouseButton::Right)) && !wants.pointer;

    // Pan by cursor delta (works on web without pointer-lock, unlike MouseMotion).
    for ev in cursor.read() {
        if let Some(last) = drag.last_cursor {
            let delta = ev.position - last;
            if panning {
                drag.moved_px += delta.length();
                t.translation.x -= delta.x * t.scale.x;
                t.translation.y += delta.y * t.scale.y;
            }
        }
        drag.last_cursor = Some(ev.position);
    }

    // Keyboard pan (WASD / arrows).
    if !wants.keyboard {
        let mut dir = Vec2::ZERO;
        if keys.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
            dir.y += 1.0;
        }
        if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
            dir.y -= 1.0;
        }
        if keys.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
            dir.x -= 1.0;
        }
        if keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
            dir.x += 1.0;
        }
        if dir != Vec2::ZERO {
            let step = dir.normalize() * 700.0 * t.scale.x * time.delta_secs();
            t.translation.x += step.x;
            t.translation.y += step.y;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_click(
    buttons: Res<ButtonInput<MouseButton>>,
    wants: Res<EguiWants>,
    drag: Res<DragState>,
    params: Res<Params>,
    windows: Query<&Window>,
    cam_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    sim: Option<Res<Sim>>,
    mut route: ResMut<RouteState>,
    mut walkshed_state: ResMut<WalkshedState>,
    mut walk_live: ResMut<WalkLive>,
    route_vis: Query<Entity, With<RouteVis>>,
    walkshed_vis: Query<Entity, With<WalkshedVis>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    // Place a point on click-release — but not if the cursor was dragged (pan).
    if wants.pointer || !buttons.just_released(MouseButton::Left) || drag.moved_px > 6.0 {
        return;
    }
    let Some(sim) = sim else { return };
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else { return };
    let Ok((cam, cam_t)) = cam_q.single() else { return };
    let Ok(world_pt) = cam.viewport_to_world_2d(cam_t, cursor) else { return };
    let enu = Enu::new(world_pt.x as f64, world_pt.y as f64);

    match params.mode {
        Mode::Walkshed => {
            for e in &walkshed_vis {
                commands.entity(e).despawn();
            }
            let Some(node) = sim.graph.snap_nearest(enu) else { return };
            let ws = sim.graph.walkshed(node, WALKSHED_SECONDS, WALK_SPEED);
            let recall = 1.0 / sim.layer.recall.unwrap_or(1.0);
            let summary = sim_core::walkshed_exposure(&sim.graph, &ws, &sim.sensors, &[], recall);

            // Reachable streets light up warm gold.
            let mut pos = Vec::new();
            for &ei in &ws.edges {
                for w in sim.graph.asset().edges[ei as usize].polyline.windows(2) {
                    pos.push([w[0][0] as f32, w[0][1] as f32, 0.12]);
                    pos.push([w[1][0] as f32, w[1][1] as f32, 0.12]);
                }
            }
            if !pos.is_empty() {
                let mesh = meshes.add(world::line_list_mesh(pos));
                let mat = materials.add(Color::srgb_u8(0xc9, 0x89, 0x2f));
                commands.spawn((Mesh2d(mesh), MeshMaterial2d(mat), Transform::from_xyz(0.0, 0.0, 0.12), WalkshedVis));
            }
            // In-shed cameras: emphasized cold rings.
            let ring = meshes.add(Circle::new(20.0));
            let ring_mat = materials.add(Color::srgba(0.16, 0.30, 0.50, 0.85));
            for p in &summary.camera_points {
                commands.spawn((
                    Mesh2d(ring.clone()),
                    MeshMaterial2d(ring_mat.clone()),
                    Transform::from_translation(world::to_world(*p, 1.6)),
                    WalkshedVis,
                ));
            }
            // Center marker (where you're standing).
            let center = meshes.add(Circle::new(22.0));
            let center_mat = materials.add(Color::srgb_u8(0x4e, 0x66, 0x38));
            commands.spawn((
                Mesh2d(center),
                MeshMaterial2d(center_mat),
                Transform::from_translation(world::to_world(enu, 3.0)),
                WalkshedVis,
            ));
            walkshed_state.summary = Some(summary);
            return;
        }
        Mode::Route => {}
    }

    // --- Route mode (A then B) ---
    if route.a.is_none() || route.b.is_some() {
        for e in &route_vis {
            commands.entity(e).despawn();
        }
        *route = RouteState {
            a: Some(enu),
            status: "Click again to set the destination (B).".into(),
            ..default()
        };
        *walk_live = WalkLive::default();
        spawn_marker(&mut commands, &mut meshes, &mut materials, enu, Color::srgb_u8(0x4e, 0x66, 0x38)); // A: lichen
        return;
    }

    route.b = Some(enu);
    *walk_live = WalkLive::default();
    spawn_marker(&mut commands, &mut meshes, &mut materials, enu, Color::srgb_u8(0x6e, 0x2f, 0x12)); // B: deep terracotta

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
        Some(&sim.dashcam_field),
    ) {
        Ok((r, summary)) => {
            let line = meshes.add(world::line_strip_mesh(&r.points, 2.0));
            let line_mat = materials.add(Color::srgb_u8(0xa8, 0x54, 0x1f)); // route: terracotta ink
            commands.spawn((Mesh2d(line), MeshMaterial2d(line_mat), Transform::default(), RouteVis));

            let walker = meshes.add(Circle::new(16.0));
            let walker_mat = materials.add(Color::srgb_u8(0x7a, 0x3b, 0x14)); // walker: burnt sienna
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

/// Animated walk: pulse each camera as the looping walker enters its view, and
/// keep a live "captured this pass" tally (resets each loop).
#[allow(clippy::too_many_arguments)]
fn walk_capture_events(
    params: Res<Params>,
    route: Res<RouteState>,
    sim: Option<Res<Sim>>,
    ping_pool: Res<PingPool>,
    mut walk_live: ResMut<WalkLive>,
    walker_q: Query<&Walker>,
    mut pings: Query<(&mut Ping, &mut Transform, &mut Visibility)>,
) {
    if params.mode != Mode::Route {
        return;
    }
    let Some(sim) = sim else { return };
    let Some(r) = &route.route else { return };
    let Ok(walker) = walker_q.single() else { return };

    // Detect loop wrap (progress reset to ~0) and start the pass fresh.
    if walker.progress_m + 1.0 < walk_live.last_progress {
        walk_live.seen.clear();
        walk_live.count = 0;
        walk_live.mobile_vehicle = 0;
        walk_live.mobile_glasses = 0;
        walk_live.mobile_bus = 0;
    }
    walk_live.last_progress = walker.progress_m;

    let pos = r.position_at(walker.progress_m);
    // Spatial cull: test only cameras within max range of the walker, not all ~5k.
    for cand in sim.cam_index.locate_within_distance([pos.x, pos.y], sim.cam_query_r2) {
        let id = cand.data;
        if walk_live.seen.contains(&id) {
            continue;
        }
        let s = &sim.sensors[id as usize];
        if sim_core::captures(&s.wedge, pos, &[]) && walk_live.seen.insert(id) {
            walk_live.count += 1;
            activate_ping(&ping_pool, &mut pings, s.wedge.apex);
        }
    }
}

/// Light up a free pulse ring at `at` (an idle pool slot). No-op if all busy.
fn activate_ping(
    pool: &PingPool,
    pings: &mut Query<(&mut Ping, &mut Transform, &mut Visibility)>,
    at: sim_core::Vec2,
) {
    for &e in &pool.entities {
        if let Ok((mut ping, mut tf, mut vis)) = pings.get_mut(e) {
            if ping.life <= 0.0 {
                ping.life = 1.0;
                tf.translation = world::to_world(at, 3.5);
                tf.scale = Vec3::splat(0.6);
                *vis = Visibility::Inherited;
                return;
            }
        }
    }
}

/// Decay active pulse rings: expand + retire (recycle the slot when life hits 0).
fn decay_pings(time: Res<Time>, mut q: Query<(&mut Ping, &mut Transform, &mut Visibility)>) {
    let dt = time.delta_secs();
    for (mut ping, mut tf, mut vis) in &mut q {
        if ping.life <= 0.0 {
            continue;
        }
        ping.life = (ping.life - dt * 2.0).max(0.0);
        tf.scale = Vec3::splat(0.6 + (1.0 - ping.life) * 1.4);
        if ping.life <= 0.0 {
            *vis = Visibility::Hidden;
        }
    }
}

/// Clear route/walkshed visuals + state when the interaction mode changes.
#[allow(clippy::too_many_arguments)]
fn sync_mode(
    params: Res<Params>,
    mut last: Local<Option<Mode>>,
    route_vis: Query<Entity, With<RouteVis>>,
    walkshed_vis: Query<Entity, With<WalkshedVis>>,
    mut route: ResMut<RouteState>,
    mut walkshed_state: ResMut<WalkshedState>,
    mut walk_live: ResMut<WalkLive>,
    mut commands: Commands,
) {
    if *last == Some(params.mode) {
        return;
    }
    let first = last.is_none();
    *last = Some(params.mode);
    if first {
        return; // don't wipe on startup
    }
    for e in &route_vis {
        commands.entity(e).despawn();
    }
    for e in &walkshed_vis {
        commands.entity(e).despawn();
    }
    *route = RouteState {
        status: match params.mode {
            Mode::Route => "Click to set start (A), then destination (B).".into(),
            Mode::Walkshed => "Click a point to map its 10-minute walkshed.".into(),
        },
        ..default()
    };
    walkshed_state.summary = None;
    *walk_live = WalkLive::default();
}

/// Re-evaluate the existing route when scenario sliders / hour change.
/// Recompute the analytical route summary only when an input that actually
/// affects it changes. We can't rely on `params.is_changed()`: `ui_panel` holds
/// `ResMut<Params>` and egui widgets bind `&mut params.field` every frame, which
/// trips change-detection unconditionally — so `is_changed()` is true *every*
/// frame, and `summarize` (a full `simulate_full` over ~900 route ticks ×
/// ~5,236 sensors) would run per frame, tanking FPS during a walk. Instead we
/// snapshot the summary-affecting inputs and recompute only on a real delta.
fn recompute_on_change(
    params: Res<Params>,
    sim: Option<Res<Sim>>,
    mut route: ResMut<RouteState>,
    mut last: Local<Option<SummarySig>>,
) {
    let Some(sim) = sim else { return };
    let Some(r) = route.route.clone() else {
        *last = None; // route cleared; next route forces a recompute
        return;
    };
    let sig = SummarySig {
        route_len_bits: r.total_m.to_bits(),
        route_points: r.points.len(),
        hour_bits: params.departure_hour.to_bits(),
        ace_on: params.ace_on,
        dashcam_on: params.dashcam_on,
        glasses_on: params.glasses_on,
        pen_bits: params.dashcam_penetration.to_bits(),
        per1000_bits: params.glasses_per_1000.to_bits(),
    };
    if *last == Some(sig) {
        return; // nothing summary-relevant changed since the last compute
    }
    *last = Some(sig);

    // Cull fixed sensors to those whose range could ever touch the route (capture
    // requires distance ≤ range_m, so this is exact — no undercount), via the
    // R-tree over dense route samples. Turns the summarize cost from ~5,236 ×
    // ticks into ~(few hundred) × ticks so a slider drag stays snappy.
    let samples = r.sample_over_time(WALK_SPEED, 1.0);
    let mut ids: std::collections::HashSet<u64> = std::collections::HashSet::new();
    for (_, p) in &samples {
        for c in sim.cam_index.locate_within_distance([p.x, p.y], sim.cam_query_r2) {
            ids.insert(c.data);
        }
    }
    let nearby: Vec<sim_core::SensorInstance> =
        ids.iter().map(|&id| sim.sensors[id as usize]).collect();

    let mobile = build_mobile(&params, &sim);
    let summary = sim_core::summarize(
        &r, &nearby, &[], &mobile, sim_params(&sim), params.departure_hour as f64,
        Some(&sim.dashcam_field),
    );
    route.summary = Some(summary);
}

/// Signature of the inputs that change the analytical route summary; recompute
/// fires only when this differs (see [`recompute_on_change`]).
#[derive(PartialEq, Eq, Clone, Copy)]
struct SummarySig {
    route_len_bits: u64,
    route_points: usize,
    hour_bits: u32,
    ace_on: bool,
    dashcam_on: bool,
    glasses_on: bool,
    pen_bits: u32,
    per1000_bits: u32,
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
        w.progress_m += WALK_SPEED * ANIM_SPEEDUP * time.delta_secs_f64();
        if w.progress_m > r.total_m {
            w.progress_m = 0.0;
        }
        t.translation = world::to_world(r.position_at(w.progress_m), 4.0);
    }
}

fn set_vis<F: bevy::ecs::query::QueryFilter>(q: &mut Query<&mut Visibility, F>, on: bool) {
    let target = if on { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in q.iter_mut() {
        if *v != target {
            *v = target;
        }
    }
}

/// In heatmap mode the base map / cameras / wedges / ACE lines are hidden so the
/// colored exposure overlay reads cleanly; otherwise FOV and ACE follow toggles.
fn sync_visibility(
    params: Res<Params>,
    mut base: Query<&mut Visibility, (With<BaseMap>, Without<FovWedge>, Without<AceVis>, Without<agents::MobileVis>)>,
    mut fov: Query<&mut Visibility, (With<FovWedge>, Without<BaseMap>, Without<AceVis>, Without<agents::MobileVis>)>,
    mut ace: Query<&mut Visibility, (With<AceVis>, Without<BaseMap>, Without<FovWedge>, Without<agents::MobileVis>)>,
    mut mobile: Query<(&mut Visibility, &agents::MobileAgent), (With<agents::MobileVis>, Without<BaseMap>, Without<FovWedge>, Without<AceVis>)>,
) {
    let hm = params.heatmap_on;
    set_vis(&mut base, !hm);
    set_vis(&mut fov, params.show_fov && !hm);
    set_vis(&mut ace, params.show_ace && !hm);
    // Agents: hidden in heatmap mode or when toggled off; otherwise each follows
    // its own `active` flag (population scaling), so only active ones show.
    let show_agents = params.show_agents && !hm;
    for (mut vis, agent) in &mut mobile {
        let target = if show_agents && agent.active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != target {
            *vis = target;
        }
    }
}

/// Heat gradient from low exposure (warm parchment-ochre) to high (cold slate) —
/// exposure literally drains warmth from the page.
const HEAT_COLORS: [Color; 6] = [
    Color::srgb_u8(0xdc, 0xcc, 0xa4),
    Color::srgb_u8(0xcb, 0xa9, 0x68),
    Color::srgb_u8(0xb8, 0x8a, 0x3e),
    Color::srgb_u8(0x9c, 0x7c, 0x6e),
    Color::srgb_u8(0x5e, 0x6f, 0x8c),
    Color::srgb_u8(0x2c, 0x47, 0x63),
];

/// Rebuild the colored heatmap meshes when the mode/class changes.
#[allow(clippy::type_complexity)]
/// Signature of the inputs that change the heatmap field; recompute fires only
/// when this differs (heatmap evaluation is a one-shot grid sweep, not per-frame).
#[derive(PartialEq, Eq, Clone, Copy)]
struct HeatSig {
    on: bool,
    class: u8,
    hour_bits: u32,
    pen_bits: u32,
    per1000_bits: u32,
    ace_on: bool,
    dashcam_on: bool,
    glasses_on: bool,
}

/// Map a normalized intensity (0..1) to an RGBA byte color along `HEAT_COLORS`,
/// with an alpha ramp so empty space stays clear and exposure glows.
fn heat_rgba(norm: f64) -> [u8; 4] {
    if norm <= 1e-6 {
        return [0, 0, 0, 0];
    }
    let t = norm.clamp(0.0, 1.0);
    let pos = (t * 5.0) as f32;
    let i = (pos.floor() as usize).min(4);
    let f = pos - i as f32;
    let a = HEAT_COLORS[i].to_srgba();
    let b = HEAT_COLORS[i + 1].to_srgba();
    let lerp = |x: f32, y: f32| x + (y - x) * f;
    let alpha = (0.20 + 0.62 * t as f32).min(0.82);
    [
        (lerp(a.red, b.red) * 255.0) as u8,
        (lerp(a.green, b.green) * 255.0) as u8,
        (lerp(a.blue, b.blue) * 255.0) as u8,
        (alpha * 255.0) as u8,
    ]
}

/// Render the exposure heatmap as a continuous **spatial field**: sample
/// `exposure_rates_per_minute` over a grid covering the Manhattan extent, map it
/// through the heat gradient with alpha, and paint it as one translucent textured
/// quad. Far more legible than per-street lines, and it reads as coverage over
/// *space* (not just where streets happen to run). Recomputed only when the
/// class / hour / sliders change (see `HeatSig`).
/// Debounce state for the heatmap: don't rebuild on every changed frame — wait
/// until the controls settle, so dragging a slider/hour coalesces into one grid
/// sweep instead of a hitch per step.
#[derive(Default)]
struct HeatDebounce {
    pending: Option<HeatSig>,
    stable_frames: u32,
    built: Option<HeatSig>,
}

/// Frames the inputs must hold steady before a (potentially expensive) rebuild.
const HEAT_DEBOUNCE_FRAMES: u32 = 6;

#[allow(clippy::too_many_arguments)]
fn rebuild_heatmap(
    params: Res<Params>,
    sim: Option<Res<Sim>>,
    mut deb: Local<HeatDebounce>,
    existing: Query<Entity, With<HeatmapVis>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let cur = HeatSig {
        on: params.heatmap_on,
        class: params.heatmap_class as u8,
        hour_bits: params.departure_hour.to_bits(),
        pen_bits: params.dashcam_penetration.to_bits(),
        per1000_bits: params.glasses_per_1000.to_bits(),
        ace_on: params.ace_on,
        dashcam_on: params.dashcam_on,
        glasses_on: params.glasses_on,
    };
    // Debounce: a new value restarts the settle counter; only once it has held
    // steady (and differs from what we last built) do we rebuild.
    if deb.pending != Some(cur) {
        deb.pending = Some(cur);
        deb.stable_frames = 0;
        return;
    }
    if deb.built == Some(cur) {
        return; // already built this exact field
    }
    deb.stable_frames += 1;
    if deb.stable_frames < HEAT_DEBOUNCE_FRAMES {
        return; // still settling
    }
    deb.built = Some(cur);

    for e in &existing {
        commands.entity(e).despawn();
    }
    if !params.heatmap_on {
        return;
    }
    let Some(sim) = sim else { return };

    // Extent = graph node bounding box (the actual island), padded a touch.
    let nodes = &sim.graph.asset().nodes;
    if nodes.is_empty() {
        return;
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) =
        (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for n in nodes {
        min_x = min_x.min(n.x);
        min_y = min_y.min(n.y);
        max_x = max_x.max(n.x);
        max_y = max_y.max(n.y);
    }
    let pad = 60.0;
    min_x -= pad; min_y -= pad; max_x += pad; max_y += pad;
    let (w_m, h_m) = (max_x - min_x, max_y - min_y);

    // Grid sized to ~45 m cells, capped so a toggle is a brief one-shot sweep.
    let cell = 45.0_f64;
    let gw = ((w_m / cell).round() as usize).clamp(48, 280);
    let gh = ((h_m / cell).round() as usize).clamp(48, 360);
    let (dx, dy) = (w_m / gw as f64, h_m / gh as f64);

    let mobile = build_mobile(&params, &sim);
    let recall = 1.0 / sim.layer.recall.unwrap_or(1.0);
    let need_ace = params.ace_on
        && matches!(params.heatmap_class, HeatClass::Ace | HeatClass::Total)
        && !sim.ace_segments.is_empty();

    // Densified ACE point index (only when the ACE class contributes).
    let ace_tree: Option<RTree<[f64; 2]>> = if need_ace {
        let mut pts = Vec::new();
        for [a, b] in &sim.ace_segments {
            pts.push([a.x, a.y]);
            pts.push([b.x, b.y]);
            pts.push([(a.x + b.x) * 0.5, (a.y + b.y) * 0.5]);
        }
        Some(RTree::bulk_load(pts))
    } else {
        None
    };
    let ace_r2 = {
        let reach = mobile.ace.as_ref().map_or(20.0, |a| a.capture_range_m);
        (reach + dy.max(dx)).powi(2)
    };

    // Sweep the grid (row 0 = north / max_y).
    let mut values = vec![0.0_f64; gw * gh];
    let mut scratch: Vec<sim_core::SensorInstance> = Vec::new();
    let mut max_v = 1e-9_f64;
    for row in 0..gh {
        let y = max_y - (row as f64 + 0.5) * dy;
        for col in 0..gw {
            let x = min_x + (col as f64 + 0.5) * dx;
            let p = Enu::new(x, y);

            scratch.clear();
            for c in sim.cam_index.locate_within_distance([x, y], sim.cam_query_r2) {
                scratch.push(sim.sensors[c.data as usize]);
            }
            let near_ace = ace_tree
                .as_ref()
                .is_some_and(|t| t.locate_within_distance([x, y], ace_r2).next().is_some());

            let r = sim_core::exposure_rates_per_minute(
                p, params.departure_hour as f64, &scratch, &[], near_ace, &mobile, recall,
                Some(&sim.dashcam_field),
            );
            let v = match params.heatmap_class {
                HeatClass::Total => r.total(),
                HeatClass::Fixed => r.fixed,
                HeatClass::Ace => r.ace,
                HeatClass::Dashcam => r.dashcam,
            };
            values[row * gw + col] = v;
            if v > max_v {
                max_v = v;
            }
        }
    }

    // Build the RGBA texture from the field.
    let mut data = vec![0u8; gw * gh * 4];
    for (i, &v) in values.iter().enumerate() {
        let px = heat_rgba(v / max_v);
        data[i * 4..i * 4 + 4].copy_from_slice(&px);
    }
    let mut image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: gw as u32,
            height: gh as u32,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::linear(); // smooth the field
    let tex = images.add(image);

    // One quad over the extent (north = max_y maps to v = 0 / top texel row).
    let (x0, x1, y0, y1) = (min_x as f32, max_x as f32, min_y as f32, max_y as f32);
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[x0, y1, 0.0], [x1, y1, 0.0], [x1, y0, 0.0], [x0, y0, 0.0]],
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
    );
    mesh.insert_indices(bevy::mesh::Indices::U32(vec![0, 2, 1, 0, 3, 2]));
    let mesh = meshes.add(mesh);
    let mat = materials.add(ColorMaterial {
        color: Color::WHITE,
        texture: Some(tex),
        ..default()
    });
    commands.spawn((
        Mesh2d(mesh),
        MeshMaterial2d(mat),
        Transform::from_xyz(0.0, 0.0, 0.15),
        HeatmapVis,
    ));
}

#[allow(clippy::too_many_arguments)]
fn apply_reset(
    mut reset: ResMut<ResetRequested>,
    params: Res<Params>,
    mut route: ResMut<RouteState>,
    mut walkshed_state: ResMut<WalkshedState>,
    mut walk_live: ResMut<WalkLive>,
    route_vis: Query<Entity, With<RouteVis>>,
    walkshed_vis: Query<Entity, With<WalkshedVis>>,
    mut commands: Commands,
) {
    if !reset.0 {
        return;
    }
    reset.0 = false;
    for e in &route_vis {
        commands.entity(e).despawn();
    }
    for e in &walkshed_vis {
        commands.entity(e).despawn();
    }
    *route = RouteState {
        status: match params.mode {
            Mode::Route => "Click to set start (A), then destination (B).".into(),
            Mode::Walkshed => "Click a point to map its 10-minute walkshed.".into(),
        },
        ..default()
    };
    walkshed_state.summary = None;
    *walk_live = WalkLive::default();
}

/// Rebuild the block-group diversity choropleth when toggled on/off.
fn rebuild_equity(
    params: Res<Params>,
    sim: Option<Res<Sim>>,
    mut last: Local<Option<bool>>,
    existing: Query<Entity, With<EquityVis>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if *last == Some(params.equity_on) {
        return;
    }
    *last = Some(params.equity_on);
    for e in &existing {
        commands.entity(e).despawn();
    }
    if !params.equity_on {
        return;
    }
    let Some(sim) = sim else { return };
    let Some(eq) = &sim.equity else { return };

    for bg in &eq.block_groups {
        if bg.population == 0 {
            continue;
        }
        let Some(mesh) = world::filled_polygon_mesh(&bg.exterior, -0.3) else {
            continue;
        };
        // Diversity ramp: washed clay (homogeneous) -> warm lichen (diverse),
        // translucent. The warm/diverse ground is precisely what the cold
        // surveillance light bleaches and clusters on (the Dahir thesis).
        let t = (bg.entropy / MAX_ENTROPY).clamp(0.0, 1.0) as f32;
        let lo = [0xcd, 0xb9, 0x8f]; // washed clay
        let hi = [0x4e, 0x66, 0x38]; // lichen
        let ch = |i: usize| (lo[i] as f32 + (hi[i] as f32 - lo[i] as f32) * t) / 255.0;
        let color = Color::srgba(ch(0), ch(1), ch(2), 0.55);
        commands.spawn((
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(materials.add(color)),
            Transform::from_xyz(0.0, 0.0, -0.3),
            EquityVis,
        ));
    }
}

/// In `OURSPACE_SMOKE` mode, exit after a few rendered frames so headless runs
/// can confirm the render loop ticked without panicking.
fn smoke_exit(
    mut frames: Local<u32>,
    mut exit: MessageWriter<AppExit>,
    mut params: ResMut<Params>,
    sim: Option<Res<Sim>>,
) {
    if std::env::var("OURSPACE_SMOKE").is_err() {
        return;
    }
    if sim.is_none() {
        return; // wait until the world is built (async asset load)
    }
    *frames += 1;
    // Exercise the heatmap + equity render paths before exiting. Hold each
    // overlay on long enough to clear the heatmap debounce (HEAT_DEBOUNCE_FRAMES).
    if *frames == 3 {
        params.heatmap_on = true;
    }
    if *frames == 14 {
        params.heatmap_on = false;
        params.equity_on = true;
    }
    if *frames == 20 {
        params.equity_on = false;
        params.mode = Mode::Walkshed; // exercise mode switch + walkshed panel
    }
    if *frames == 28 {
        let _ = std::fs::write("/tmp/ourspace_frames.txt", format!("frames_ok={}\n", *frames));
        exit.write(AppExit::Success);
    }
}
