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
mod coverage;
mod geocode;
mod loading;
mod movable;
mod operators;
mod storymap;
mod theme;
mod ui;
mod world;

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::math::primitives::Circle;
use bevy::prelude::*;
use bevy::mesh::VertexAttributeValues;
use bevy::window::CursorMoved;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};

use bevy::math::primitives::Annulus;
use rstar::primitives::GeomWithData;
use rstar::RTree;
use agents::AgentPool;
use loading::{
    AceRes, AlprRes, BoroughRes, BusDayRes, CamerasRes, CctvRes, DashcamFieldRes, DotRes, EquityRes,
    FacilitiesRes, FootprintsRes, GraphAssetRes, HeatmapRes, LandmarkRes, LinkNycRes, LoadingHandles,
    NeighborhoodRes, ParksRes, PlazaRes, RobotabilityRes, TaxiDayRes, TeslaFieldRes,
    VehicleRoutesRes,
};
use operators::{OperatorCol, OperatorMesh, OperatorsLayout, OperatorsView};
use sim_core::assets::{
    BusDayLayer, DashcamFieldLayer, EquityLayer, FixedSensorLayer, HeatmapLayer, RobotabilityField,
    TaxiDayLayer, TeslaField,
};
use sim_core::simulation::SimParams;
use sim_core::{
    AceConfig, DashcamConfig, EnuProjection, FixedCameraDefaults, GlassesConfig,
    MobileScenario, RobotConfig, Route, RouteSummary, StreetGraph, TeslaConfig, Vec2 as Enu,
};

const WALK_SPEED: f64 = sim_core::graph::DEFAULT_WALK_SPEED_MPS;

/// NYC's default citywide speed limit (25 mph ≈ 11.176 m/s) — the cruise cap for the
/// replayed dashcam-vehicle pace.
const NYC_SPEED_LIMIT_MPS: f64 = 11.176;
/// Sharpest turns are taken at this fraction of the limit (decelerate through corners).
const TURN_SPEED_FRAC: f64 = 0.3;

/// World time-lapse rate (sim-seconds per real-second): the master [`SimClock`]
/// advances `rate` sim-seconds each real second and the ambient agents move the
/// same `rate`, so the day and the traffic stay congruent. Tweaked by the time
/// panel's speed control. Default ≈ a real 24 h day every 40 min (`86400/(36·60)`) — a
/// calmer pace so the time-lapse reads as ambient motion rather than a fast-forward.
pub(crate) const DEFAULT_SIM_RATE: f64 = 36.0;
pub(crate) const SIM_RATE_MIN: f64 = 15.0; // calm: ~96 min / day
pub(crate) const SIM_RATE_MAX: f64 = 1440.0; // fast time-lapse: ~60 s / day

/// Granularity at which the route summary + heatmap re-evaluate as the clock runs
/// (2 → every 30 sim-min): the headline *steps* cleanly instead of recomputing the
/// expensive `summarize()` every frame and flickering. The live scene (agents,
/// sun/moon) still moves continuously.
pub(crate) const CLOCK_RECOMPUTE_STEPS_PER_HOUR: f64 = 2.0;

/// The clock's current recompute step (a stable signature key while the minute
/// hand sweeps within a 30-min bucket).
pub(crate) fn clock_hour_step(time_of_day: f64) -> i64 {
    (time_of_day * CLOCK_RECOMPUTE_STEPS_PER_HOUR).floor() as i64
}

/// The master simulation clock — the live time-of-day plus the world time-lapse
/// rate and play/pause. Replaces the old frozen `Params.departure_hour`; both the
/// ambient agents and the analytical estimate now read it.
#[derive(Resource)]
pub struct SimClock {
    /// Time of day in hours, `[0, 24)` — cycles back to 0 at 24.
    pub time_of_day: f64,
    /// World time-lapse: sim-seconds per real-second (see [`DEFAULT_SIM_RATE`]).
    pub rate: f64,
    /// When false the whole world (clock + agents) freezes.
    pub playing: bool,
}
impl Default for SimClock {
    fn default() -> Self {
        SimClock {
            time_of_day: 17.0,
            rate: DEFAULT_SIM_RATE,
            playing: true,
        }
    }
}

// Zoom feel: gentle multiplicative zoom per normalized scroll notch.
const ZOOM_PER_NOTCH: f32 = 0.06;
const ZOOM_PIXEL_DIVISOR: f32 = 160.0;
const ZOOM_MIN: f32 = 0.4;
const ZOOM_MAX: f32 = 30.0;
/// Address fly-in tuning: target zoom (m/px) for a single point, and the ease time.
const STORY_OVERVIEW_ZOOM: f32 = 9.0; // StoryMap "overview" — wide view of the island
const FLY_AREA_ZOOM: f32 = 2.6; // walkshed center — shows ~the 10-minute reach
const FLY_POINT_ZOOM: f32 = 1.6; // a single route endpoint — street level
const FLY_DUR: f32 = 0.7; // seconds

// Paths relative to the AssetServer root (`assets/`); works native + web.
// Distinct extensions disambiguate the per-type postcard loaders.
const GRAPH_PATH: &str = "processed/graph_manhattan.osgraph";
/// Manhattan drive network (CSCL, incl. the FDR/highways; carries road class in
/// segment_id). Taxi routes are baked over it, and the coverage overlay snaps to it.
const GRAPH_DRIVE_PATH: &str = "processed/graph_manhattan_drive.osgraph";
const CAMERAS_PATH: &str = "processed/cameras_fixed.oscctv";
/// Automated photo-enforcement cameras (DOT signage), a `FixedSensorLayer`.
const ENFORCEMENT_PATH: &str = "processed/enforcement.oscam";
/// Radius (m) within which fixed cameras from different sources are treated as one
/// physical camera (multiply-attested) for the de-duplicated headline count.
const FIXED_GROUP_RADIUS_M: f64 = 15.0;
const ACE_PATH: &str = "processed/ace_corridors.osace";
const HEATMAP_PATH: &str = "processed/heatmap.osheat";
const EQUITY_PATH: &str = "processed/equity.osequity";
const DASHCAM_FIELD_PATH: &str = "processed/dashcam_field.osfield";
const ALPR_PATH: &str = "processed/alpr.osalpr";
const DOT_PATH: &str = "processed/dot_cameras.osdot";
const VEHICLE_ROUTES_PATH: &str = "processed/vehicle_routes.osroutes";
const NEIGHBORHOODS_PATH: &str = "processed/neighborhoods.osneigh";
// Real-day trip replay (one consistent date: Tue 2026-04-21).
const BUS_DAY_PATH: &str = "processed/bus_day_20260421.osbusday";
const TAXI_DAY_PATH: &str = "processed/taxi_day_20260421.ostaxiday";
/// Citywide rideshare: a real day of all-five-borough TLC HVFHV trips, routed on the
/// citywide graph (vs the Manhattan-only `TAXI_DAY_PATH`). Baked from 2024-06-25.
const TAXI_DAY_PATH_NYC: &str = "processed/taxi_day_nyc.ostaxiday";
/// Robotability Score field (IRL-CT) for weighted delivery-robot spawning.
const ROBOT_FIELD_PATH: &str = "processed/robotability.osrobot";
/// Tesla-camera field (private NYS DMV Tesla registrations by ZIP).
const TESLA_FIELD_PATH: &str = "processed/teslas.osteslas";
/// Borough outlines (NYC DCP boundaries) — the coastline frame. The Manhattan-focus
/// default build shares the all-five-borough outline (`borough_nyc.osboro`): Manhattan
/// (incl. Roosevelt / Randalls / Governors islands) is the detailed focus, and the
/// other four boroughs draw as quiet context frames. `rings[0]` stays Manhattan's main
/// landmass (the offshore-label anchor); the citywide camera clip (`in_manhattan` =
/// point-in-any-ring) is a no-op here since the default census is Manhattan-only.
const BOROUGH_PATH: &str = "processed/borough_nyc.osboro";

// --- Citywide (five-borough) asset set, selected by `?city=nyc` / `OURSPACE_CITY=nyc`.
// Only the layers that differ from the Manhattan build have a `_nyc` variant; the
// rest (enforcement, ALPR, footprints, landmarks, LinkNYC, heatmap, graph, trips)
// stay Manhattan for the static-first MVP. Neighborhoods are already citywide, so
// the same `NEIGHBORHOODS_PATH` is reused.
const CAMERAS_PATH_NYC: &str = "processed/cameras_fixed_nyc.oscctv";
const DOT_PATH_NYC: &str = "processed/dot_cameras_nyc.osdot";
/// Citywide ACE corridors + bus-day (all 5 boroughs, GTFS C6 board, Tue 2026-07-07).
/// The ACE buses are the one dynamic layer carried into the citywide static-first
/// MVP; the other agent layers stay Manhattan-only / off.
const ACE_PATH_NYC: &str = "processed/ace_corridors_nyc.osace";
const BUS_DAY_PATH_NYC: &str = "processed/bus_day_nyc.osbusday";
/// Citywide street network — the five-borough **drive** graph (OSM surface roads,
/// largest connected component spanning all boroughs via their bridges; the walk
/// network would strand Staten Island, which has no pedestrian crossing). Renders the
/// streets across the city and backs citywide routing/walkshed.
const GRAPH_PATH_NYC: &str = "processed/graph_nyc.osgraph";
/// All-five-borough outline (one asset): the five main landmasses first (ring `i` ↔
/// borough region `i` for the footprint loader), then detached islands. Also the
/// runtime camera clip (`in_manhattan` tests point-in-any-ring), so it un-clips the
/// citywide census. Shared with the default build (see `BOROUGH_PATH`).
const BOROUGH_PATH_NYC: &str = "processed/borough_nyc.osboro";
/// Initial camera framing for the citywide overview: ENU center (~8 km south of the
/// Midtown origin, the city's vertical midpoint) and m/px scale to fit ~47 km.
const CITY_INIT_CENTER: Vec3 = Vec3::new(0.0, -9000.0, 0.0);
const CITY_INIT_SCALE: f32 = 40.0;

// --- Lazy per-borough footprints (citywide build). The full citywide building
// fabric is ~51 MB, so it's never in first-load: each borough's footprints load
// only when the camera zooms in past the floor and the viewport overlaps it.
// Order matches the five-borough outline's ring order (Manhattan, Bronx, Brooklyn,
// Queens, Staten Island), so ring `i` gives region `i`'s bbox. Manhattan reuses the
// base `footprints.osbldg`; the rest are per-borough bakes.
const FOOTPRINT_REGIONS: &[(&str, &str)] = &[
    ("Manhattan", "processed/footprints.osbldg"),
    ("Bronx", "processed/footprints_bronx.osbldg"),
    ("Brooklyn", "processed/footprints_brooklyn.osbldg"),
    ("Queens", "processed/footprints_queens.osbldg"),
    ("Staten Island", "processed/footprints_statenisland.osbldg"),
];
/// Zoomed out past this camera scale (m/px), footprints are sub-pixel clutter, so no
/// region is loaded — the citywide overview pays zero footprint cost.
const FOOTPRINT_ZOOM_FLOOR_MPP: f32 = 8.0;

/// Zoomed out past this scale (m/px) the neighborhood count-labels are illegible, so
/// they're hidden and their per-frame de-collision is skipped (keeps citywide panning
/// smooth with the choropleth on). They reappear as you zoom into a borough.
const NEIGHBORHOOD_LABEL_MAX_MPP: f32 = 22.0;

/// Runtime state for one lazily-loaded footprint region (a borough).
struct FootprintRegionState {
    label: &'static str,
    path: &'static str,
    bbox: [f64; 4], // ENU [min_x, min_y, max_x, max_y]
    handle: Option<Handle<FootprintsRes>>,
    entity: Option<Entity>,
}

/// The lazy footprint-region set + the shared fabric material (citywide only).
#[derive(Resource, Default)]
struct FootprintRegions {
    regions: Vec<FootprintRegionState>,
    material: Option<Handle<ColorMaterial>>,
}

/// Marks a lazily-spawned footprint-region mesh — distinct from the eager
/// `BuildingVis` fabric the Manhattan build uses, so it's outside that toggle and
/// governed solely by the zoom-floor + viewport loader.
#[derive(Component)]
struct FootprintRegionMesh;

/// Axis-aligned bbox of an ENU ring as `[min_x, min_y, max_x, max_y]`.
fn ring_bbox(ring: &[[f64; 2]]) -> [f64; 4] {
    let (mut a, mut b, mut c, mut d) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in ring {
        a = a.min(p[0]);
        b = b.min(p[1]);
        c = c.max(p[0]);
        d = d.max(p[1]);
    }
    [a, b, c, d]
}
/// Half-width (m) of the coastline outline band. Constant world width, so it reads
/// as a thin hairline at the city overview and gains presence as you zoom in.
const OUTLINE_HALF_WIDTH_M: f32 = 6.0;
/// Building footprints (NYC DCP) — the flat ground fabric under the streets.
const FOOTPRINTS_PATH: &str = "processed/footprints.osbldg";
/// Parks (NYC Parks Properties) — flat green polygons under the streets. Manhattan-
/// clipped for the default build; the citywide build swaps the all-borough set.
const PARKS_PATH: &str = "processed/parks.ospark";
const PARKS_PATH_NYC: &str = "processed/parks_nyc.ospark";
/// Painter z for the parks fabric — below the building footprints (−0.1) so a park's
/// interior buildings still draw over the green, and above the choropleth fill (−0.2).
const PARKS_Z: f32 = -0.16;
/// Pedestrian plazas (NYC DOT) — paved public spaces, one asset for both builds.
const PLAZAS_PATH: &str = "processed/plazas.osplaza";
/// Plaza fill sits above the footprints (−0.1) — a plaza is carved from roadway, not
/// a building — with its hatch a hair above the fill, both below the streets (0.0).
const PLAZA_FILL_Z: f32 = -0.08;
const PLAZA_HATCH_Z: f32 = -0.07;
/// Curated landmark LoD2 massings (NYC 3D Building Model) — 2.5D orientation aids.
const LANDMARKS_PATH: &str = "processed/landmarks.oslmk";
/// Iconic NYC bridge massings (decks + towers + cables) — generated from CSCL
/// centerlines, shares the landmark schema + oblique renderer. Citywide-relevant
/// (the East River spans are visible from Manhattan), so loaded in both builds.
const BRIDGES_PATH: &str = "processed/bridges.oslmk";
/// LinkNYC Wi-Fi/phone kiosks — a fixed point layer (not cameras).
const LINKNYC_PATH: &str = "processed/linknyc.oslink";
/// Institutions (schools + libraries) — Manhattan-only default vs all five boroughs.
const FACILITIES_PATH: &str = "processed/facilities.osfac";
const FACILITIES_PATH_NYC: &str = "processed/facilities_nyc.osfac";
/// Vertical extrusion scale for the landmark massing: world-meters of screen-up
/// shift per meter of building height. Pure vertical (no sideways skew) so every
/// building stands straight up; depth + recognizability come from per-face shading,
/// not lean. Tunable — bigger = taller-looking.
const LANDMARK_HEIGHT: f32 = 0.5;
/// Painter z for the landmark massing — **above** every mobile agent (≤ `Z_BUS` 2.7)
/// and the camera icons (1.0), so a building's 2.5D silhouette occludes vehicles,
/// peds, and cameras that are screen-behind it. Stays below the A/B markers (3.0),
/// walker (4.0), and labels (5.0) so the user's own route stays readable.
pub(crate) const LANDMARK_MASSING_Z: f32 = 2.8;
/// Painter z for the bridges, rendered as quiet flat footprints (not 3D massing). It
/// must sit ABOVE the coastline outline so the boundary stroke never shows atop a span
/// crossing the water. NOTE the outline's *effective* z is 0.30, not 0.15: its mesh
/// (`stroke_ring_mesh`) bakes z=0.15 into the verts AND its transform adds another 0.15.
/// The ACE ribbons likewise double to ~0.4. So this is set well above both (0.5) — the
/// earlier 0.16/0.185 were below the outline's true 0.30 and the boundary line cut
/// through. Still below the camera icons (1.0) and mobile agents (≤ 2.7) so they draw
/// over the deck (no occlusion). Iconic bridges carry no ACE bus-lane corridors, so
/// sitting above the ACE ribbons is harmless.
const BRIDGE_FLAT_Z: f32 = 0.5;
/// Light direction for the landmark Lambert shading: from the upper south-east
/// (east+, south−, up+), so roofs read bright, south/east faces lit, west faces dark.
const LANDMARK_LIGHT: Vec3 = Vec3::new(0.38, -0.48, 0.79);

/// The real calendar day the simulation replays (matches the baked day-assets).
#[derive(Resource, Clone)]
pub struct SimDate {
    /// Display label, e.g. "Tuesday, April 21, 2026".
    pub label: String,
    /// `YYYYMMDD`, the baked service date.
    pub ymd: u32,
}
impl Default for SimDate {
    fn default() -> Self {
        SimDate { label: "Tuesday, April 21, 2026".into(), ymd: 20260421 }
    }
}

/// Max Shannon entropy over 5 groups = ln(5), for normalizing the choropleth.
const MAX_ENTROPY: f64 = 1.6094379;

/// Interaction mode: route between two points, a one-point walkshed, or the
/// neighborhood camera-density overview (a read-only mode driven by hovering).
/// `None` is the startup state — nothing is selected, the map is inert, and the
/// panel prompts the visitor to pick how to explore.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    None,
    Route,
    Walkshed,
    Neighborhoods,
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

/// Per-neighborhood camera aggregation, computed at world-build from the loaded
/// neighborhood polygons + every fixed sensor (point-in-polygon). Drives the
/// neighborhood choropleth + the hover breakdown.
pub struct NeighborhoodStat {
    pub name: String,
    pub borough: String,
    /// Exterior ring in ENU meters (for the fill + outline meshes).
    pub exterior: Vec<[f64; 2]>,
    /// `[min_x, min_y, max_x, max_y]` ENU bounds (fast point prefilter).
    pub bbox: [f64; 4],
    /// Label anchor (ENU centroid).
    pub centroid: Enu,
    pub area_km2: f64,
    pub cctv: u32,
    pub dot: u32,
    pub alpr: u32,
    pub total: u32,
    /// Cameras per km² (the choropleth value).
    pub density: f64,
}

impl NeighborhoodStat {
    /// Ray-cast point-in-polygon (bbox-prefiltered) for hover picking.
    pub(crate) fn contains(&self, p: Enu) -> bool {
        if p.x < self.bbox[0] || p.x > self.bbox[2] || p.y < self.bbox[1] || p.y > self.bbox[3] {
            return false;
        }
        let r = &self.exterior;
        let n = r.len();
        if n < 3 {
            return false;
        }
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = (r[i][0], r[i][1]);
            let (xj, yj) = (r[j][0], r[j][1]);
            if ((yi > p.y) != (yj > p.y)) && (p.x < (xj - xi) * (p.y - yi) / (yj - yi) + xi) {
                inside = !inside;
            }
            j = i;
        }
        inside
    }
}

/// The loaded simulation world (routing graph + placed sensors + ACE corridors).
#[derive(Resource)]
pub struct Sim {
    pub graph: StreetGraph,
    /// Drive network (CSCL, incl. highways) the roving-coverage overlay snaps onto.
    pub drive_graph: StreetGraph,
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
    /// Per-neighborhood camera aggregation (all boroughs; app renders Manhattan by default).
    pub neighborhoods: Vec<NeighborhoodStat>,
    /// Real-day ACE bus schedule (replayed by the clock).
    pub bus_day: BusDayLayer,
    /// Real-day rideshare trips (replayed by the clock).
    pub taxi_day: TaxiDayLayer,
    /// `Route` per bus shape (for `position_at` during replay), indexed by `BusTrip.shape_idx`.
    pub bus_routes: Vec<Route>,
    /// `Route` per taxi O-D pool entry, indexed by `TaxiTrip.route_idx`.
    pub taxi_routes: Vec<Route>,
    /// Turn-aware, speed-limit-capped pace per taxi route (same index as `taxi_routes`)
    /// so replayed dashcam vehicles brake into turns and cruise on straights instead of
    /// gliding the whole trip at one constant speed.
    pub taxi_paces: Vec<sim_core::PaceProfile>,
    /// Real per-minute ACE headways + rideshare volume for the baked day, feeding the
    /// analytical headline so the cited number tracks the real timetable/trips by minute.
    pub real_rates: Option<sim_core::mobile::RealDayRates>,
    /// NYC Robotability Score field (IRL-CT) for the speculative delivery-robot layer.
    pub robot_field: RobotabilityField,
    /// Cumulative robotability weight per graph node (ends ~1.0) for weighted
    /// delivery-robot spawning — robots cluster where the score is high.
    pub robot_node_cumulative: Vec<f32>,
    /// Tesla-camera field (private DMV Tesla registration density by ZIP).
    pub tesla_field: TeslaField,
    /// Cumulative Tesla-density weight per graph node for weighted Tesla spawning.
    pub tesla_node_cumulative: Vec<f32>,
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
    /// Show the MapLibre street basemap behind the sim layers (web only; off by
    /// default — the sim reads on its own near-black ground, the basemap is opt-in).
    pub basemap_on: bool,
    /// Draw the Manhattan borough coastline as a frame around the street network.
    pub outline_on: bool,
    /// Draw the building-footprint ground fabric under the streets.
    pub buildings_on: bool,
    /// Draw the parks as flat green polygons under the streets.
    pub parks_on: bool,
    /// Draw the pedestrian plazas (concrete fill + hatch) under the streets.
    pub plazas_on: bool,
    /// Draw the curated landmark buildings as 2.5D massing (orientation aids).
    pub landmarks_on: bool,
    /// Draw the LinkNYC kiosks (Wi-Fi hubs). Off by default — a dense supplementary
    /// layer the visitor opts into.
    pub linknyc_on: bool,
    pub ace_on: bool,
    pub dashcam_on: bool,
    /// The "In 5 years…" speculative-future mode: smart glasses + delivery robots, the
    /// two not-yet-deployed surveillance layers, grouped behind one switch (off by
    /// default). `set_future` keeps `glasses_on`/`robots_on` in lockstep with it.
    pub future_on: bool,
    /// AI smart glasses on pedestrians (speculative; driven by `future_on`).
    pub glasses_on: bool,
    /// Speculative sidewalk delivery robots (driven by `future_on`; not yet legal in NYC).
    pub robots_on: bool,
    /// Tesla cameras (Sentry/Autopilot), density from private DMV registrations.
    pub tesla_on: bool,
    pub dashcam_penetration: f32,
    pub glasses_per_1000: f32,
    /// Robots/min passing in a top-robotability spot at peak (Robotability scales it down).
    pub robots_density: f32,
    /// Tesla "passes"/min in a typical ZIP (the Tesla field scales it spatially).
    pub tesla_density: f32,
    pub heatmap_on: bool,
    pub heatmap_class: HeatClass,
    pub equity_on: bool,
    /// Neighborhoods mode: draw the hoverable neighborhood boundaries (the on-hover
    /// breakdown browser). The camera-density choropleth itself rides `choropleth_on`.
    pub neighborhoods_on: bool,
    /// Show the camera-density **choropleth** (the per-neighborhood density fill + count
    /// labels — "the heatmap"). An opt-in Explore toggle within Neighborhoods mode, kept
    /// off the bare hover-browser so the default citywide view stays light.
    pub choropleth_on: bool,
    /// Include non-Manhattan boroughs in the neighborhood layer (default: Manhattan only).
    pub neighborhoods_all: bool,
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
            // Routes (the corridor ribbons) off by default — the moving ACE buses
            // (`ace_on`) carry the layer; the ribbons are a power-user overlay in
            // "More layers". Off keeps the default map from being dominated by blue.
            show_ace: false,
            basemap_on: false,
            outline_on: true,
            buildings_on: true,
            parks_on: true,
            plazas_on: true,
            landmarks_on: true,
            linknyc_on: false,
            ace_on: true,
            dashcam_on: true,
            future_on: false,
            glasses_on: false,
            robots_on: false,
            tesla_on: false,
            dashcam_penetration: 0.40,
            glasses_per_1000: 10.0,
            robots_density: 2.0,
            tesla_density: 4.0,
            heatmap_on: false,
            heatmap_class: HeatClass::Fixed,
            equity_on: false,
            neighborhoods_on: false,
            choropleth_on: false,
            neighborhoods_all: false,
            // No mode is selected at startup: the visitor first chooses how to
            // explore (their area, an A→B walk, or neighborhoods). The map stays
            // inert until then — no seeded example, no accidental clicks.
            mode: Mode::None,
            show_agents: true,
            exposure_mode: ExposureMode::Analytical,
        }
    }
}

impl Params {
    /// Enter/leave the "In 5 years…" speculative future: smart glasses + sidewalk
    /// delivery robots appear together (or both vanish). One switch for both layers —
    /// the "In 5 years…" button and StoryMaps both go through here.
    pub fn set_future(&mut self, on: bool) {
        self.future_on = on;
        self.glasses_on = on;
        self.robots_on = on;
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

/// Set once `setup_theme` has applied the dark visuals + installed the display
/// fonts; `ui_panel` holds off drawing until then so the poster font family exists.
#[derive(Resource, Default)]
pub struct ThemeReady(pub bool);

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

/// The neighborhood under the cursor (index into `Sim.neighborhoods`), for the
/// panel breakdown. `None` when the layer is off or the cursor is off-map.
#[derive(Resource, Default)]
pub struct NeighborhoodPick(pub Option<usize>);

/// One ALPR reader's clickable metadata, built at world load (Manhattan-filtered).
/// `pos` (world ENU) is hit-tested in `handle_click`; `lat`/`lon` + `osm_id` drive the
/// DeFlock / OpenStreetMap deep-links in the modal.
#[derive(Clone)]
pub(crate) struct AlprPin {
    pub pos: Vec2,
    pub osm_id: u64,
    pub manufacturer: Option<String>,
    pub operator: Option<String>,
    pub heading_deg: Option<f64>,
    pub lat: f64,
    pub lon: f64,
    /// Other fixed-camera sources that map this same physical camera (co-located, merged
    /// into one group by `group_sensors`). Empty = single-source. Drives the modal's
    /// "cross-source confirmed" note. See [`group_attestations`].
    pub also_sources: Vec<&'static str>,
}
/// All Manhattan ALPR readers + metadata, for click-picking + the modal.
#[derive(Resource, Default)]
pub(crate) struct AlprDirectory(pub Vec<AlprPin>);
/// The ALPR the user clicked (index into `AlprDirectory`); `Some` shows the modal.
#[derive(Resource, Default)]
pub(crate) struct SelectedAlpr(pub Option<usize>);
/// ALPR reader counts per manufacturer (descending), for the Operators-view maker
/// legend that names the strata banding the ALPR tower.
#[derive(Resource, Default)]
pub(crate) struct AlprMakerBreakdown(pub Vec<(operators::Maker, usize)>);
/// Click within this many CSS px of an ALPR/CCTV marker to open its metadata modal.
const ALPR_PICK_PX: f32 = 16.0;

// ---- Institutions (schools + libraries) explore view ----
/// Radius (m) around an institution within which deduplicated fixed cameras are
/// counted as "watching" it — its surveillance-exposure score. ~2–3 blocks.
const FACILITY_SCAN_M: f64 = 200.0;
/// Click within this many CSS px of an institution marker to open its modal.
const FACILITY_PICK_PX: f32 = 14.0;

/// One institution's clickable record (runtime; ENU + metadata + nearby-camera score).
#[derive(Clone)]
pub(crate) struct FacilityPin {
    pub pos: Vec2,
    pub name: String,
    pub kind: sim_core::assets::FacilityKind,
    pub subtype: String,
    pub lat: f64,
    pub lon: f64,
    /// Deduplicated fixed cameras within [`FACILITY_SCAN_M`] (one per physical camera).
    pub cameras_near: u32,
}
/// All institutions + a surveillance-ranked index, for the left panel + click-picking.
#[derive(Resource, Default)]
pub(crate) struct FacilityDirectory {
    pub pins: Vec<FacilityPin>,
    /// Indices into `pins`, most-watched first (computed once, at world build).
    pub ranked: Vec<usize>,
}
/// The institution the user selected (index into `FacilityDirectory.pins`); `Some`
/// shows the modal + highlights its marker.
#[derive(Resource, Default)]
pub(crate) struct SelectedFacility(pub Option<usize>);

/// The "Institutions" explore view: a toggle + a slide-in transition for the left
/// ranking panel, plus the per-class visibility filters. Activated from the EXPLORE
/// section of the right panel.
#[derive(Resource)]
pub(crate) struct InstitutionsView {
    pub active: bool,
    /// Slide-in progress for the left panel (0 = off-screen, 1 = docked); eased.
    pub t: f32,
    pub show_schools: bool,
    pub show_libraries: bool,
}
impl Default for InstitutionsView {
    fn default() -> Self {
        InstitutionsView { active: false, t: 0.0, show_schools: true, show_libraries: true }
    }
}
/// Rank facility indices most-watched first (nearby-camera count desc, then name asc
/// for stable ties). Pulled out of `build_world` so it's unit-testable.
fn rank_facilities(pins: &[FacilityPin]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..pins.len()).collect();
    idx.sort_by(|&a, &b| {
        pins[b]
            .cameras_near
            .cmp(&pins[a].cameras_near)
            .then_with(|| pins[a].name.cmp(&pins[b].name))
    });
    idx
}

/// On-map institution marker (one merged mesh per class); visibility follows the view.
#[derive(Component)]
pub(crate) struct FacilityMarker {
    pub kind: sim_core::assets::FacilityKind,
}
/// The single reusable ring that highlights the selected institution.
#[derive(Component)]
pub(crate) struct FacilityHighlight;
/// Handle to the [`FacilityHighlight`] entity (moved onto the selected institution).
#[derive(Resource)]
pub(crate) struct FacilityHighlightEntity(pub Entity);
/// World size (m) of an institution marker square, and its painter z (above cameras
/// so the subjects read clearly while the view is up).
const FACILITY_MARK_SIZE: f32 = 26.0;
const FACILITY_MARK_Z: f32 = 1.6;
/// Camera-marker modals (ALPR/CCTV) only open at/below this zoom (camera m/px). Zoomed
/// further out the markers are tiny + dense and a click is almost always meant for the
/// area/route, so the modals stay out of the way of "my area" / the walk.
const MODAL_ZOOM_MAX_MPP: f32 = 4.0;

/// One fixed-CCTV camera's clickable provenance for the modal (Manhattan-filtered).
/// Dahir-sourced cameras carry a Street View `panoid` + capture date; all carry lat/lon
/// for a Google Street View deep-link.
#[derive(Clone)]
pub(crate) struct CctvPin {
    pub pos: Vec2,
    pub source: sim_core::assets::CctvSource,
    pub panoid: Option<String>,
    pub year: Option<u16>,
    pub month: Option<u8>,
    pub heading_deg: Option<f64>,
    pub lat: f64,
    pub lon: f64,
    /// Other fixed-camera sources that map this same physical camera (co-located, merged
    /// into one group by `group_sensors`). Empty = single-source. Drives the modal's
    /// "cross-source confirmed" note. See [`group_attestations`].
    pub also_sources: Vec<&'static str>,
}
/// All Manhattan CCTV cameras + provenance, for click-picking + the modal.
#[derive(Resource, Default)]
pub(crate) struct CctvDirectory(pub Vec<CctvPin>);
/// The CCTV the user clicked (index into `CctvDirectory`); `Some` shows the modal.
#[derive(Resource, Default)]
pub(crate) struct SelectedCctv(pub Option<usize>);

/// Short human label for a fixed-camera source, used to name the *other* layers that
/// attest a merged camera in the click modal. Returns `None` for the mobile/speculative
/// kinds, which are never part of a fixed-camera group.
pub(crate) fn fixed_source_label(kind: sim_core::SourceKind) -> Option<&'static str> {
    use sim_core::SourceKind::*;
    Some(match kind {
        FixedCctv => "CCTV census",
        Alpr => "DeFlock ALPR",
        DotLiveView => "NYC DOT camera",
        EnforcementCamera => "DOT enforcement",
        _ => return None,
    })
}

/// Mobile sensors currently inside one neighborhood, by class — sampled live from
/// the moving agents as the day clock runs.
#[derive(Clone, Copy, Default)]
pub struct MobileCount {
    pub rideshare: u32,
    pub bus: u32,
    pub tesla: u32,
    pub robot: u32,
    pub glasses: u32,
}
impl MobileCount {
    pub fn total(&self) -> u32 {
        self.rideshare + self.bus + self.tesla + self.robot + self.glasses
    }
}

/// Live per-neighborhood mobile-sensor tallies (aligned to `Sim.neighborhoods`),
/// refreshed on a throttle by `sample_neighborhood_mobile` so the choropleth +
/// labels breathe with the day's real traffic. Empty until the first sample.
#[derive(Resource, Default)]
pub struct NeighborhoodLive {
    pub by_nbhd: Vec<MobileCount>,
}
impl NeighborhoodLive {
    pub fn get(&self, i: usize) -> MobileCount {
        self.by_nbhd.get(i).copied().unwrap_or_default()
    }
}

/// Tags a choropleth fill mesh + its count label with the neighborhood index they
/// belong to, so the live sampler can recolor / relabel them in place.
#[derive(Component, Clone, Copy)]
pub struct NeighborhoodId(pub usize);

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
    /// Narrative mode: stochastic delivery-robot encounters this pass.
    pub mobile_robot: u32,
    /// Narrative mode: stochastic Tesla-camera encounters this pass.
    pub mobile_tesla: u32,
    pub last_progress: f64,
}

/// The current one-point walkshed result (for the panel).
#[derive(Resource, Default)]
pub struct WalkshedState {
    pub summary: Option<sim_core::WalkshedSummary>,
    /// A gentle one-off hint shown with the result (e.g. the seeded SoHo example);
    /// cleared the moment the user places their own walkshed.
    pub status: Option<String>,
}

// --------------------------------------------------------------- components ---

#[derive(Component)]
pub(crate) struct BaseMap; // streets (cameras now carry OperatorMesh); hidden in heatmap mode
#[derive(Component)]
pub(crate) struct FovWedge;
#[derive(Component)]
pub(crate) struct AceVis;
/// Holds the ACE corridor segments + base width so `scale_ace_corridors` can rebuild
/// the ribbon mesh to a zoom-floored on-screen width.
#[derive(Component)]
struct AceCorridors {
    segments: Vec<[Enu; 2]>,
    base_half: f32,
    mesh: Handle<Mesh>,
}
/// LinkNYC kiosk markers — a fixed Wi-Fi-hub layer, toggled by `linknyc_on`.
#[derive(Component)]
pub(crate) struct LinkNycVis;
/// Points + base size for a merged-quad marker layer (LinkNYC kiosks) that
/// `scale_markers` keeps a legible minimum on-screen size, like the camera icons.
#[derive(Component)]
struct ScaledMarkers {
    pts: Vec<Enu>,
    base_half: f32,
    mesh: Handle<Mesh>,
}
/// Manhattan coastline outline mesh — a geographic frame, toggled by `outline_on`.
#[derive(Component)]
pub(crate) struct OutlineVis;
/// Building-footprint ground-fabric mesh, toggled by `buildings_on`.
#[derive(Component)]
pub(crate) struct BuildingVis;
/// Parks ground-fabric mesh (flat green polygons), toggled by `parks_on`.
#[derive(Component)]
pub(crate) struct ParksVis;
/// Pedestrian-plaza meshes (concrete fill + hatch overlay), toggled by `plazas_on`.
#[derive(Component)]
pub(crate) struct PlazasVis;
/// Landmark 2.5D massing mesh, toggled by `landmarks_on`.
#[derive(Component)]
pub(crate) struct LandmarkVis;
/// Iconic bridge massing mesh (decks + towers + cables). Shares the landmark
/// renderer; visibility follows `landmarks_on` (no off-water labels — bridges
/// name themselves by silhouette).
#[derive(Component)]
pub(crate) struct BridgeVis;
/// Landmark name label (world-anchored `Text2d`), kept a constant on-screen size by
/// `size_landmark_labels`; follows `landmarks_on`. Floated off-island over the water
/// with a leader line back to the building (see [`offshore_label_anchors`]).
#[derive(Component)]
pub(crate) struct LandmarkLabel;
/// Hairline leader lines tying off-island labels back to their objects. One merged
/// mesh per layer; the width is zoom-floored by `scale_leader_lines`.
#[derive(Component)]
pub(crate) struct LeaderLines {
    segments: Vec<[sim_core::Vec2; 2]>,
    base_half: f32,
    mesh: Handle<Mesh>,
}
/// Marks the landmark leader-line mesh (visibility follows `landmarks_on`, like the labels).
#[derive(Component)]
pub(crate) struct LandmarkLeader;
#[derive(Component)]
struct HeatmapVis;
#[derive(Component)]
struct EquityVis;
/// Neighborhood-layer fill + outline meshes (cleared/rebuilt on toggle).
#[derive(Component)]
struct NeighborhoodVis;
/// Neighborhood name labels (Text2d), cleared/rebuilt with the layer.
#[derive(Component)]
struct NeighborhoodLabel;
/// Transient hover-emphasis overlay (wash + bold ring) for the neighborhood under the
/// cursor; rebuilt only when the hovered region changes (see [`highlight_neighborhood`]).
#[derive(Component)]
struct NeighborhoodHighlight;
/// Dark translucent backing plate behind a neighborhood label (a child of the label),
/// sized to the text by [`size_neighborhood_label_plates`] so the counts stay readable
/// over the warm choropleth.
#[derive(Component)]
struct NeighborhoodLabelPlate;
#[derive(Component)]
pub(crate) struct RouteVis;
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
            title: "our-space · Manhattan sensing exposure".into(),
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
    // Transparent on web (the white #stage shows through); white paper on native.
    .insert_resource(ClearColor(if cfg!(target_arch = "wasm32") {
        Color::NONE
    } else {
        theme::map::ZINC_950 // white paper ground
    }))
    .init_resource::<RouteState>()
    .init_resource::<geocode::Geocoder>()
    .init_resource::<CameraFly>()
    .init_resource::<storymap::StoryMap>()
    .init_resource::<Params>()
    .init_resource::<EguiWants>()
    .init_resource::<DragState>()
    .init_resource::<ResetRequested>()
    .init_resource::<WalkLive>()
    .init_resource::<WalkshedState>()
    .init_resource::<PingPool>()
    .init_resource::<OperatorsView>()
    .init_resource::<OperatorsLayout>()
    .init_resource::<NeighborhoodPick>()
    .init_resource::<AlprDirectory>()
    .init_resource::<AlprMakerBreakdown>()
    .init_resource::<SelectedAlpr>()
    .init_resource::<CctvDirectory>()
    .init_resource::<SelectedCctv>()
    .init_resource::<FacilityDirectory>()
    .init_resource::<SelectedFacility>()
    .init_resource::<InstitutionsView>()
    .init_resource::<NeighborhoodLive>()
    .init_resource::<SimClock>()
    .init_resource::<SimDate>()
    .init_resource::<ThemeReady>()
    .init_resource::<agents::ReplayState>()
    .init_resource::<movable::MovablePanels>()
    .init_resource::<coverage::CoverageView>()
    .init_resource::<coverage::Coverage>()
    .init_resource::<coverage::EdgeGrid>()
    .init_resource::<FootprintRegions>()
    .insert_resource(AgentPool::empty())
    .add_systems(Startup, (start_loading, init_reduced_motion))
    .add_systems(
        Update,
        (
            (build_world, manage_footprint_regions),
            advance_clock,
            (camera_control, fly_camera).chain(),
            (handle_click, geocode::geocode_tick, apply_geocode, storymap_tick, storymap_autostart),
            recompute_on_change,
            animate_walker,
            (walk_capture_events, agents::mobile_capture_events).chain(),
            decay_pings,
            agents::replay_agents,
            agents::scale_agent_population,
            agents::animate_agents,
            (scale_camera_icons, scale_ace_corridors, scale_markers, scale_leader_lines),
            (
                operators::operators_drive,
                operators::operators_layout,
                operators::operators_snapshot,
                operators::operators_animate_fixed,
                operators::operators_animate_mobile,
                operators::operators_chip_material,
                operators::operators_mobile_material,
                operators::operators_headers,
                operators::operators_fade_scene,
            )
                .chain(),
            (
                sync_mode,
                (sync_visibility, sync_building_visibility, sync_leader_visibility),
                (
                    coverage::coverage_drive,
                    coverage::coverage_accumulate,
                    coverage::coverage_render,
                    coverage::coverage_scene,
                )
                    .chain()
                    .after(agents::animate_agents)
                    .after(sync_visibility)
                    .after(sync_building_visibility),
            ),
            rebuild_heatmap,
            rebuild_equity,
            rebuild_neighborhoods,
            (
                size_neighborhood_labels,
                size_neighborhood_label_plates,
                declutter_neighborhood_labels,
                size_landmark_labels,
                pick_neighborhood,
                highlight_neighborhood,
                sample_neighborhood_mobile,
            ),
            (
                institutions_tick,
                facility_markers_visibility,
                facility_highlight_follow,
            ),
            (ui::setup_theme, apply_reset, smoke_exit),
        ),
    )
    .add_systems(EguiPrimaryContextPass, (ui::ui_panel, ui::citywide_nudge, ui::coverage_overlay, ui::storymap_ui, ui::alpr_modal, ui::cctv_modal, ui::institutions_panel));
    // Web: keep the MapLibre basemap synced to the Bevy camera each frame.
    #[cfg(target_arch = "wasm32")]
    app.add_systems(Update, basemap::sync_basemap);
    // Native dev-only: `OURSPACE_SHOT=1` saves a screenshot of the Operators view.
    #[cfg(not(target_arch = "wasm32"))]
    app.add_systems(Update, shot_capture);
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
        robots: if params.robots_on {
            Some(RobotConfig {
                robots_per_min_peak: params.robots_density as f64,
                ..Default::default()
            })
        } else {
            None
        },
        tesla: if params.tesla_on {
            Some(TeslaConfig {
                teslas_per_min_peak: params.tesla_density as f64,
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

/// Read the OS/page reduced-motion preference once; the Operators view snaps
/// (no fly animation) when set, and the day clock starts paused (no auto-advancing
/// sun — the user can still scrub it). Native honors `OURSPACE_REDUCED_MOTION`.
fn init_reduced_motion(mut ov: ResMut<OperatorsView>, mut clock: ResMut<SimClock>) {
    #[cfg(target_arch = "wasm32")]
    {
        ov.reduced_motion = basemap::prefers_reduced_motion();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        ov.reduced_motion = std::env::var("OURSPACE_REDUCED_MOTION").is_ok();
    }
    if ov.reduced_motion {
        clock.playing = false;
    }
}

/// Ray-casting point-in-polygon on an ENU ring (for the runtime Manhattan trim).
fn point_in_ring(p: Enu, ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (ring[i][0], ring[i][1]);
        let (xj, yj) = (ring[j][0], ring[j][1]);
        if ((yi > p.y) != (yj > p.y)) && (p.x < (xj - xi) * (p.y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// True if ENU point `p` lies inside the Manhattan boundary (any of its rings) —
/// used to trim off-island geometry (e.g. floating cameras) at runtime against the
/// borough-outline asset we already load.
fn in_manhattan(p: Enu, rings: &[Vec<[f64; 2]>]) -> bool {
    rings.iter().any(|ring| point_in_ring(p, ring))
}

/// Whether to load the citywide (five-borough) asset set + framing instead of the
/// Manhattan build. Web: `?city=nyc` query param. Native: `OURSPACE_CITY=nyc` env
/// var (dev screenshots). Mirrors `url_story_is`'s web-sys read; the env check is
/// inert on the web (returns `Err`) and the window read is inert on native.
fn citywide_scope() -> bool {
    if std::env::var("OURSPACE_CITY")
        .map(|v| v.eq_ignore_ascii_case("nyc"))
        .unwrap_or(false)
    {
        return true;
    }
    #[cfg(target_arch = "wasm32")]
    {
        return web_sys::window()
            .and_then(|w| w.location().search().ok())
            .map(|s| s.contains("city=nyc"))
            .unwrap_or(false);
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

/// Set once at startup so downstream systems can branch on the citywide build
/// without re-reading the URL/env.
#[derive(Resource, Clone, Copy)]
pub struct CityScope {
    pub citywide: bool,
}

/// Spawn the camera and request the baked layers via the AssetServer.
fn start_loading(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut route: ResMut<RouteState>,
    mut params: ResMut<Params>,
    mut sim_date: ResMut<SimDate>,
) {
    let citywide = citywide_scope();
    commands.insert_resource(CityScope { citywide });
    // Widen the address geocoder's accept-box to all five boroughs in the citywide
    // build, so a Brooklyn/Queens map click reverse-geocodes to its real address.
    geocode::set_citywide(citywide);

    // Citywide carries two real-day dynamic layers now: the ACE buses (citywide GTFS)
    // and the rideshare/taxi vehicles (citywide TLC HVFHV, routed on the citywide
    // graph — see `TAXI_DAY_PATH_NYC`). The remaining agent layers (smart glasses,
    // robots, Teslas) still replay Manhattan-only fields, so they stay off. Open on the
    // all-borough camera choropleth. The citywide bus + taxi days are a current-board
    // Tuesday (the baked Manhattan real-day date is Manhattan-only).
    if citywide {
        params.mode = Mode::Neighborhoods;
        params.neighborhoods_all = true;
        params.ace_on = true; // ACE buses citywide
        params.dashcam_on = true; // rideshare/taxi vehicles citywide (TLC HVFHV)
        params.glasses_on = false;
        params.robots_on = false;
        params.tesla_on = false;
        *sim_date = SimDate {
            label: "Tuesday, July 7, 2026".into(),
            ymd: 20260707,
        };
    }

    let (init_xy, init_scale) = if citywide {
        (CITY_INIT_CENTER, CITY_INIT_SCALE)
    } else {
        (Vec3::ZERO, 6.0)
    };
    commands.spawn((
        Camera2d,
        Transform::from_translation(init_xy).with_scale(Vec3::splat(init_scale)),
    ));

    // Only the layers with a citywide variant are swapped; the rest stay Manhattan
    // for the static-first MVP (footprints lazy per-borough; landmarks/ALPR/
    // enforcement/LinkNYC Manhattan-only).
    let (graph_path, cameras_path, dot_path, borough_path, ace_path, bus_day_path) = if citywide {
        (GRAPH_PATH_NYC, CAMERAS_PATH_NYC, DOT_PATH_NYC, BOROUGH_PATH_NYC, ACE_PATH_NYC, BUS_DAY_PATH_NYC)
    } else {
        (GRAPH_PATH, CAMERAS_PATH, DOT_PATH, BOROUGH_PATH, ACE_PATH, BUS_DAY_PATH)
    };
    // Parks: Manhattan-clipped default, all five boroughs citywide (small either way).
    let parks_path = if citywide { PARKS_PATH_NYC } else { PARKS_PATH };
    // Institutions (schools + libraries): Manhattan-only default, all five citywide.
    let facilities_path = if citywide { FACILITIES_PATH_NYC } else { FACILITIES_PATH };
    // Rideshare/taxi: all-borough HVFHV on the citywide graph, vs Manhattan-only.
    let taxi_day_path = if citywide { TAXI_DAY_PATH_NYC } else { TAXI_DAY_PATH };
    // Coverage snaps to a drive graph with highways; citywide CSCL already has them.
    let drive_graph_path = if citywide { GRAPH_PATH_NYC } else { GRAPH_DRIVE_PATH };

    commands.insert_resource(LoadingHandles {
        graph: asset_server.load(graph_path),
        drive_graph: asset_server.load(drive_graph_path),
        cameras: asset_server.load(cameras_path),
        enforcement: asset_server.load(ENFORCEMENT_PATH),
        ace: asset_server.load(ace_path),
        heatmap: asset_server.load(HEATMAP_PATH),
        equity: asset_server.load(EQUITY_PATH),
        dashcam: asset_server.load(DASHCAM_FIELD_PATH),
        alpr: asset_server.load(ALPR_PATH),
        dot: asset_server.load(dot_path),
        vehicle_routes: asset_server.load(VEHICLE_ROUTES_PATH),
        neighborhoods: asset_server.load(NEIGHBORHOODS_PATH),
        bus_day: asset_server.load(bus_day_path),
        taxi_day: asset_server.load(taxi_day_path),
        robotability: asset_server.load(ROBOT_FIELD_PATH),
        teslas: asset_server.load(TESLA_FIELD_PATH),
        borough: asset_server.load(borough_path),
        footprints: asset_server.load(FOOTPRINTS_PATH),
        parks: asset_server.load(parks_path),
        plazas: asset_server.load(PLAZAS_PATH),
        landmarks: asset_server.load(LANDMARKS_PATH),
        bridges: asset_server.load(BRIDGES_PATH),
        linknyc: asset_server.load(LINKNYC_PATH),
        facilities: asset_server.load(facilities_path),
        built: false,
    });
    route.status = if citywide {
        "Loading citywide (5-borough) map data…".into()
    } else {
        "Loading Manhattan map data…".into()
    };
}

/// Lazily load/unload per-borough footprint meshes for the citywide build, driven
/// by the camera viewport + a zoom-floor. Self-initializes each region's bbox from
/// the loaded five-borough outline (ring `i` ↔ region `i`). A no-op for the
/// Manhattan build (whose footprints are the eager single mesh in `build_world`).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn manage_footprint_regions(
    city: Option<Res<CityScope>>,
    handles: Option<Res<LoadingHandles>>,
    mut state: ResMut<FootprintRegions>,
    borough_assets: Res<Assets<BoroughRes>>,
    fps_assets: Res<Assets<FootprintsRes>>,
    cam: Query<(&Camera, &Transform), With<Camera2d>>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut commands: Commands,
) {
    let Some(city) = city else { return };
    if !city.citywide {
        return;
    }
    // Wait until the world (hence the borough outline) is up.
    let Some(handles) = handles else { return };
    if !handles.built {
        return;
    }

    // One-time init: a region per borough-outline ring, in ring order.
    if state.regions.is_empty() {
        let Some(boro) = borough_assets.get(&handles.borough) else { return };
        if boro.0.rings.len() < FOOTPRINT_REGIONS.len() {
            return;
        }
        for (i, (label, path)) in FOOTPRINT_REGIONS.iter().enumerate() {
            state.regions.push(FootprintRegionState {
                label,
                path,
                bbox: ring_bbox(&boro.0.rings[i]),
                handle: None,
                entity: None,
            });
        }
        // light-gray building fabric on white (matches the Manhattan eager fabric)
        state.material = Some(materials.add(theme::map::ZINC_800));
    }

    let Ok((cam, cam_t)) = cam.single() else { return };
    let scale = cam_t.scale.x; // m/px
    let zoomed_in = scale <= FOOTPRINT_ZOOM_FLOOR_MPP;

    // Viewport world bbox (+ half-frame margin so a region pre-loads just before it
    // scrolls into view).
    let center = cam_t.translation.truncate();
    let (vmin, vmax) = if let Some(vp) = cam.logical_viewport_size() {
        let half = Vec2::new(vp.x, vp.y) * scale * 0.75; // 0.5 frame + 0.25 margin each side
        (
            [(center.x - half.x) as f64, (center.y - half.y) as f64],
            [(center.x + half.x) as f64, (center.y + half.y) as f64],
        )
    } else {
        ([f64::MIN, f64::MIN], [f64::MAX, f64::MAX])
    };

    let mat = state.material.clone();
    for r in state.regions.iter_mut() {
        // Keep a region built while we stay zoomed in (avoids re-triangulating big
        // boroughs when panning across them); only the zoom-out tier frees memory.
        let wanted = zoomed_in
            && r.bbox[0] <= vmax[0]
            && r.bbox[2] >= vmin[0]
            && r.bbox[1] <= vmax[1]
            && r.bbox[3] >= vmin[1];

        if wanted {
            if r.handle.is_none() {
                r.handle = Some(asset_server.load(r.path));
            }
            if r.entity.is_none() {
                if let Some(fp) = r.handle.as_ref().and_then(|h| fps_assets.get(h)) {
                    if !fp.0.polygons.is_empty() {
                        let mesh = meshes.add(world::merged_footprints_mesh(&fp.0.polygons));
                        let e = commands
                            .spawn((
                                Mesh2d(mesh),
                                MeshMaterial2d(mat.clone().expect("region material set in init")),
                                Transform::from_xyz(0.0, 0.0, -0.1),
                                FootprintRegionMesh,
                            ))
                            .id();
                        r.entity = Some(e);
                    }
                }
            }
        } else if !zoomed_in {
            // Zoomed back out to the overview: free meshes + drop handles so the
            // ~51 MB of citywide fabric isn't held resident.
            if let Some(e) = r.entity.take() {
                commands.entity(e).despawn();
            }
            r.handle = None;
        }
    }
}

/// Once all baked layers have loaded, build the simulation world + map meshes.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn build_world(
    mut handles: ResMut<LoadingHandles>,
    graphs: Res<Assets<GraphAssetRes>>,
    cams: Res<Assets<CamerasRes>>,
    cctvs: Res<Assets<CctvRes>>,
    aces: Res<Assets<AceRes>>,
    heatmaps: Res<Assets<HeatmapRes>>,
    equities: Res<Assets<EquityRes>>,
    // Bundled into one tuple param to stay under Bevy's 16-param-per-system cap.
    #[allow(clippy::type_complexity)]
    (dashcams, alprs, dots, vroutes, neighborhoods_asset, bus_days, taxi_days, robots_field_asset, tesla_field_asset, borough_asset, footprints_asset, parks_asset, plazas_asset, landmark_asset, linknyc_asset, facilities_asset): (
        Res<Assets<DashcamFieldRes>>,
        Res<Assets<AlprRes>>,
        Res<Assets<DotRes>>,
        Res<Assets<VehicleRoutesRes>>,
        Res<Assets<NeighborhoodRes>>,
        Res<Assets<BusDayRes>>,
        Res<Assets<TaxiDayRes>>,
        Res<Assets<RobotabilityRes>>,
        Res<Assets<TeslaFieldRes>>,
        Res<Assets<BoroughRes>>,
        Res<Assets<FootprintsRes>>,
        Res<Assets<ParksRes>>,
        Res<Assets<PlazaRes>>,
        Res<Assets<LandmarkRes>>,
        Res<Assets<LinkNycRes>>,
        Res<Assets<FacilitiesRes>>,
    ),
    asset_server: Res<AssetServer>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut route: ResMut<RouteState>,
    city: Option<Res<CityScope>>,
) {
    if handles.built {
        return;
    }
    // Citywide build: footprints load lazily per borough (`manage_footprint_regions`).
    let citywide = city.map(|c| c.citywide).unwrap_or(false);
    let (
        Some(g),
        Some(dg),
        Some(c),
        Some(enf),
        Some(a),
        Some(h),
        Some(e),
        Some(df),
        Some(al),
        Some(dot),
        Some(vr),
        Some(nb),
        Some(bd),
        Some(td),
        Some(rf),
        Some(tf),
        Some(boro),
        Some(fps),
        Some(parks),
        Some(plazas),
        Some(lmk),
        Some(brg),
        Some(link),
        Some(fac),
    ) = (
        graphs.get(&handles.graph),
        graphs.get(&handles.drive_graph),
        cctvs.get(&handles.cameras),
        cams.get(&handles.enforcement),
        aces.get(&handles.ace),
        heatmaps.get(&handles.heatmap),
        equities.get(&handles.equity),
        dashcams.get(&handles.dashcam),
        alprs.get(&handles.alpr),
        dots.get(&handles.dot),
        vroutes.get(&handles.vehicle_routes),
        neighborhoods_asset.get(&handles.neighborhoods),
        bus_days.get(&handles.bus_day),
        taxi_days.get(&handles.taxi_day),
        robots_field_asset.get(&handles.robotability),
        tesla_field_asset.get(&handles.teslas),
        borough_asset.get(&handles.borough),
        footprints_asset.get(&handles.footprints),
        parks_asset.get(&handles.parks),
        plazas_asset.get(&handles.plazas),
        landmark_asset.get(&handles.landmarks),
        landmark_asset.get(&handles.bridges),
        linknyc_asset.get(&handles.linknyc),
        facilities_asset.get(&handles.facilities),
    ) else {
        return; // still loading
    };

    let graph = StreetGraph::from_asset(g.0.clone());
    // The drive network (incl. highways) the coverage overlay snaps onto.
    let drive_graph = StreetGraph::from_asset(dg.0.clone());
    // CCTV now carries per-camera provenance (`CctvCameraLayer`); project to the shared
    // sensor type for the exposure model.
    let layer = c.0.to_fixed_layer();
    // Combine fixed-camera layers: Dahir CCTV + DeFlock ALPRs + NYC DOT traffic
    // cameras, re-indexing ids to the combined-vector position (used as the
    // distinct-device key + dot index). DOT cams use monitoring defaults
    // (omnidirectional, wider reach, low frame rate).
    // Trim every fixed-camera source to the boundary. The data is NOT Manhattan-only —
    // the DeFlock ALPR census (and some DOT/CCTV) carry outer-borough cameras — so the
    // Manhattan build clips to Manhattan's main landmass alone (`rings[0]`), keeping the
    // outer boroughs empty of sensing layers; the citywide build keeps all five boroughs'
    // rings so the census un-clips across the city.
    let man: &[Vec<[f64; 2]>] = if citywide { &boro.0.rings } else { &boro.0.rings[..1] };
    let keep = |s: &sim_core::SensorInstance| in_manhattan(s.wedge.apex, man);
    let mut sensors: Vec<_> = sim_core::sensors_from_layer(&layer, FixedCameraDefaults::default())
        .into_iter().filter(&keep).collect();
    let cctv_count = sensors.len();
    // ALPRs now carry per-reader metadata (`AlprReaderLayer`); project to the shared
    // sensor type for the exposure model. Directional readers keep their heading, so
    // they render a 70° FOV wedge like any directional camera.
    sensors.extend(
        sim_core::sensors_from_layer(&al.0.to_fixed_layer(), FixedCameraDefaults::default())
            .into_iter()
            .filter(&keep),
    );
    let alpr_count = sensors.len() - cctv_count;
    sensors.extend(
        sim_core::sensors_from_layer(&dot.0, FixedCameraDefaults::dot_monitoring()).into_iter().filter(&keep),
    );
    let dot_count = sensors.len() - cctv_count - alpr_count;
    // Automated photo-enforcement cameras (DOT signage) — omnidirectional monitoring.
    sensors.extend(
        sim_core::sensors_from_layer(&enf.0, FixedCameraDefaults::dot_monitoring()).into_iter().filter(&keep),
    );
    let enf_count = sensors.len() - cctv_count - alpr_count - dot_count;
    for (i, s) in sensors.iter_mut().enumerate() {
        s.id = i as u64;
    }
    // ALPR click-modal directory: Manhattan readers + DeFlock/OSM metadata, hit-tested
    // in `handle_click`. lat/lon precomputed for the DeFlock deep-link.
    let alpr_proj = EnuProjection::default();
    let mut alpr_pins: Vec<AlprPin> = al
        .0
        .readers
        .iter()
        .filter(|r| in_manhattan(Enu::new(r.x, r.y), man))
        .map(|r| {
            let (lat, lon) = alpr_proj.to_wgs84(Enu::new(r.x, r.y));
            AlprPin {
                pos: Vec2::new(r.x as f32, r.y as f32),
                osm_id: r.osm_id,
                manufacturer: r.manufacturer.clone(),
                operator: r.operator.clone(),
                heading_deg: r.heading_deg,
                lat,
                lon,
                also_sources: Vec::new(), // filled below, after `group_sensors`
            }
        })
        .collect();
    // Per-reader maker (parallel to the Manhattan ALPR sensors, same filter+order) →
    // both the tower banding (item: stratify by maker) and the panel legend.
    let alpr_makers_seq: Vec<operators::Maker> = al
        .0
        .readers
        .iter()
        .filter(|r| in_manhattan(Enu::new(r.x, r.y), man))
        .map(|r| operators::Maker::classify(r.manufacturer.as_deref()))
        .collect();
    {
        let mut counts = vec![0usize; operators::MAKERS.len()];
        for m in &alpr_makers_seq {
            if let Some(i) = operators::MAKERS.iter().position(|x| x == m) {
                counts[i] += 1;
            }
        }
        let mut breakdown: Vec<(operators::Maker, usize)> = operators::MAKERS
            .iter()
            .copied()
            .zip(counts)
            .filter(|(_, c)| *c > 0)
            .collect();
        breakdown.sort_by(|a, b| b.1.cmp(&a.1));
        commands.insert_resource(AlprMakerBreakdown(breakdown));
    }
    // CCTV click-modal directory: Manhattan cameras + Amnesty/Dahir provenance.
    let mut cctv_pins: Vec<CctvPin> = c
        .0
        .cameras
        .iter()
        .filter(|cam| in_manhattan(Enu::new(cam.x, cam.y), man))
        .map(|cam| {
            let (lat, lon) = alpr_proj.to_wgs84(Enu::new(cam.x, cam.y));
            CctvPin {
                pos: Vec2::new(cam.x as f32, cam.y as f32),
                source: cam.source,
                panoid: cam.panoid.clone(),
                year: cam.year,
                month: cam.month,
                heading_deg: cam.heading_deg,
                lat,
                lon,
                also_sources: Vec::new(), // filled below, after `group_sensors`
            }
        })
        .collect();
    // Group co-located fixed cameras across sources into one physical node, so a
    // camera attested by several layers (CCTV census + DOT + ALPR + enforcement)
    // counts once in the headline (multiply-attested, not multiply-counted).
    let n_fixed = sensors.len();
    let n_nodes = sim_core::group_sensors(&mut sensors, FIXED_GROUP_RADIUS_M);
    info!(
        "grouped {} fixed sensors → {} distinct camera nodes ({} co-located merges)",
        n_fixed,
        n_nodes,
        n_fixed - n_nodes
    );
    // Cross-source attestation for the click modals: a camera merged into a group with
    // members from other layers (e.g. a DeFlock ALPR co-located with a NYC DOT cam) lists
    // those *other* sources, so the popup can say "also mapped here by …". `group_sensors`
    // already set each sensor's `group`; we read it back per pin by position (the pin's
    // `pos` is the same `Vec2` as its sensor's `apex`, so a 0.5 m grid key matches exactly).
    {
        use std::collections::{HashMap, HashSet};
        // `apex` is `sim_core::Vec2`, the pins' `pos` is `bevy::Vec2` — distinct types,
        // same f32 `.x`/`.y`. Key the grid on the raw components so both match.
        let mut group_kinds: HashMap<u32, HashSet<sim_core::SourceKind>> = HashMap::new();
        let mut pos_group: HashMap<(i64, i64), u32> = HashMap::new();
        let cell = |x: f64, y: f64| ((x * 2.0).round() as i64, (y * 2.0).round() as i64);
        for s in &sensors {
            group_kinds.entry(s.group).or_default().insert(s.kind);
            pos_group.insert(cell(s.wedge.apex.x, s.wedge.apex.y), s.group);
        }
        let others_for = |x: f64, y: f64, self_kind: sim_core::SourceKind| -> Vec<&'static str> {
            let Some(g) = pos_group.get(&cell(x, y)) else { return Vec::new() };
            let Some(kinds) = group_kinds.get(g) else { return Vec::new() };
            let mut v: Vec<&'static str> = kinds
                .iter()
                .filter(|&&k| k != self_kind)
                .filter_map(|&k| fixed_source_label(k))
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        for p in &mut alpr_pins {
            p.also_sources =
                others_for(p.pos.x as f64, p.pos.y as f64, sim_core::SourceKind::Alpr);
        }
        for p in &mut cctv_pins {
            p.also_sources =
                others_for(p.pos.x as f64, p.pos.y as f64, sim_core::SourceKind::FixedCctv);
        }
        let alpr_x = alpr_pins.iter().filter(|p| !p.also_sources.is_empty()).count();
        let cctv_x = cctv_pins.iter().filter(|p| !p.also_sources.is_empty()).count();
        info!(
            "cross-source attestations: {} ALPR + {} CCTV clickable pins multiply-attested",
            alpr_x, cctv_x
        );
    }
    commands.insert_resource(AlprDirectory(alpr_pins));
    commands.insert_resource(CctvDirectory(cctv_pins));
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

    // Parks — flat green polygons under the streets (and under the footprints, so a
    // park's interior buildings still read). One merged mesh, in BOTH builds (the
    // citywide set is all five boroughs; small enough to load eagerly). Green ground
    // context that also fills the otherwise-bare outer boroughs at citywide zoom.
    if !parks.0.polygons.is_empty() {
        let park_mesh = meshes.add(world::merged_footprints_mesh(&parks.0.polygons));
        let park_mat = materials.add(theme::map::ca(0x86, 0xa8, 0x66, 0.92)); // muted sage parkland
        commands.spawn((
            Mesh2d(park_mesh),
            MeshMaterial2d(park_mat),
            Transform::from_xyz(0.0, 0.0, PARKS_Z),
            ParksVis,
        ));
    }

    // Pedestrian plazas — paved public spaces carved from the roadway. A warm
    // concrete fill (above the footprints, below the streets) with a 45° hatch
    // clipped to each polygon (`world::hatch_lines_mesh`) reading as paving/tiling.
    // The asset is citywide; the Manhattan-only build clips it to the island so no
    // plaza floats off in Brooklyn/Queens (a polygon is kept if any vertex is in
    // Manhattan's main landmass).
    let plaza_polys_owned;
    let plaza_polys: &[Vec<[f32; 2]>] = if citywide {
        &plazas.0.polygons
    } else {
        let man0 = &boro.0.rings[0];
        plaza_polys_owned = plazas
            .0
            .polygons
            .iter()
            .filter(|poly| {
                poly.iter()
                    .any(|v| point_in_ring(Enu::new(v[0] as f64, v[1] as f64), man0))
            })
            .cloned()
            .collect::<Vec<_>>();
        &plaza_polys_owned
    };
    if !plaza_polys.is_empty() {
        let fill = meshes.add(world::merged_footprints_mesh(plaza_polys));
        let fill_mat = materials.add(theme::map::ca(0xd7, 0xd1, 0xc6, 0.95)); // warm concrete
        commands.spawn((
            Mesh2d(fill),
            MeshMaterial2d(fill_mat),
            Transform::from_xyz(0.0, 0.0, PLAZA_FILL_Z),
            PlazasVis,
        ));
        let hatch = meshes.add(world::hatch_lines_mesh(plaza_polys, 6.0, 0.35, PLAZA_HATCH_Z));
        let hatch_mat = materials.add(theme::map::ca(0x8f, 0x88, 0x7b, 0.55)); // darker warm-gray paving lines
        commands.spawn((
            Mesh2d(hatch),
            MeshMaterial2d(hatch_mat),
            Transform::from_xyz(0.0, 0.0, PLAZA_HATCH_Z),
            PlazasVis,
        ));
    }

    // Building footprints — the flat ground fabric, a quiet dark fill *under* the
    // streets so blocks read between the linework. One merged mesh. Citywide skips
    // this: footprints stream per borough via `manage_footprint_regions`.
    if !citywide && !fps.0.polygons.is_empty() {
        let fp_mesh = meshes.add(world::merged_footprints_mesh(&fps.0.polygons));
        let fp_mat = materials.add(theme::map::ZINC_800); // light-gray building fabric on white
        commands.spawn((
            Mesh2d(fp_mesh),
            MeshMaterial2d(fp_mat),
            Transform::from_xyz(0.0, 0.0, -0.1),
            BuildingVis,
        ));
    }

    // Streets.
    let street_mesh = meshes.add(world::line_list_mesh(world::street_line_positions(graph.asset())));
    let street_mat = materials.add(theme::map::ZINC_500); // medium-gray ink linework on paper
    commands.spawn((
        Mesh2d(street_mesh),
        MeshMaterial2d(street_mat),
        Transform::from_xyz(0.0, 0.0, 0.0),
        BaseMap,
    ));

    // Manhattan coastline outline — a quiet light-zinc frame around the street
    // network (just above streets, below the transit + marker layers). Geometry
    // only; a visual frame, not part of the exposure model.
    for ring in &boro.0.rings {
        if let Some(mesh) = world::stroke_ring_mesh(ring, OUTLINE_HALF_WIDTH_M, 0.15) {
            let outline_mesh = meshes.add(mesh);
            // Opaque light-zinc so the Operators-view fade (absolute alpha) recedes
            // it cleanly with the rest of the map.
            let outline_mat = materials.add(theme::map::ZINC_400);
            commands.spawn((
                Mesh2d(outline_mesh),
                MeshMaterial2d(outline_mat),
                Transform::from_xyz(0.0, 0.0, 0.15),
                OutlineVis,
            ));
        }
    }

    // Landmark 2.5D massing — notable buildings extruded straight up and per-face
    // Lambert-shaded so their real shape (One WTC's facets, the Empire State's
    // setbacks, Columbia's dome) reads for orientation. One depth-sorted mesh with
    // per-vertex colors (white material multiplies them). Drawn ABOVE the camera +
    // agent layers (`LANDMARK_MASSING_Z`) so a building's silhouette occludes the
    // vehicles / peds / cameras that are screen-behind it.
    {
        let mesh = meshes.add(world::landmark_massing_mesh(
            &lmk.0.landmarks,
            LANDMARK_HEIGHT,
            LANDMARK_LIGHT,
        ));
        let mat = materials.add(Color::WHITE);
        commands.spawn((
            Mesh2d(mesh),
            MeshMaterial2d(mat),
            Transform::from_xyz(0.0, 0.0, LANDMARK_MASSING_Z),
            LandmarkVis,
        ));

        // Float each landmark's name OFF the island, out over the water, with a hairline
        // leader back to the building — so the names stop cluttering the dense, informative
        // interior. Each goes to its nearest shore, spread apart so the midtown cluster
        // doesn't overlap. `size_landmark_labels` keeps the text a constant screen size and
        // `scale_leader_lines` keeps the leaders hairline-thin at any zoom.
        let lm_pts: Vec<Vec2> = lmk
            .0
            .landmarks
            .iter()
            .map(|lm| Vec2::new(lm.anchor[0], lm.anchor[1]))
            .collect();
        let placed = offshore_label_anchors(&lm_pts, &boro.0.rings[0], LABEL_OFFSHORE_M, LABEL_GAP_M);
        let mut leaders: Vec<[Enu; 2]> = Vec::with_capacity(placed.len());
        for (lm, p) in lmk.0.landmarks.iter().zip(&placed) {
            // Text grows into the water (away from the island), anchored on its
            // island-facing edge so the leader meets the label cleanly.
            let (anchor, justify) = if p.left {
                (bevy::sprite::Anchor::CENTER_RIGHT, Justify::Right)
            } else {
                (bevy::sprite::Anchor::CENTER_LEFT, Justify::Left)
            };
            commands.spawn((
                Text2d::new(lm.name.clone()),
                TextFont { font_size: 34.0, ..default() },
                TextColor(theme::map::ca(0x18, 0x18, 0x1b, 0.96)), // ink on paper
                TextLayout::new_with_justify(justify),
                anchor,
                Transform::from_xyz(p.pos.x, p.pos.y, 5.0),
                LandmarkLabel,
            ));
            leaders.push([
                Enu::new(p.obj.x as f64, p.obj.y as f64),
                Enu::new(p.pos.x as f64, p.pos.y as f64),
            ]);
        }
        if !leaders.is_empty() {
            let lmesh = meshes.add(world::thick_line_list_mesh(&leaders, LEADER_BASE_HALF, 0.0));
            commands.spawn((
                Mesh2d(lmesh.clone()),
                MeshMaterial2d(materials.add(theme::map::ca(0x18, 0x18, 0x1b, 0.34))), // faint ink hairline
                Transform::from_xyz(0.0, 0.0, 3.0),
                LandmarkLeader,
                LeaderLines { segments: leaders, base_half: LEADER_BASE_HALF, mesh: lmesh },
            ));
        }
    }

    // Iconic bridges as quiet flat footprints — the deck slab + tower/pier caps from
    // the CSCL centerlines (see `tools/generate_bridges.py`), flattened to the ground
    // plane (`landmark_flat_mesh` drops the vertical cables/truss/walls). Styled in the
    // same light zinc as the building footprints and drawn low (`BRIDGE_FLAT_Z`), so a
    // span reads as context that connects the street grids — not a bold 3D structure
    // that occludes the traffic crossing it. Follows `landmarks_on`.
    // The bridges asset carries all nine NYC spans; the Manhattan-only build keeps just
    // the ones that touch the island (a bridge is in if any surface vertex falls in
    // Manhattan's main landmass) so Verrazzano / Throgs Neck / Whitestone don't float
    // off in the far boroughs. The citywide build renders them all.
    let bridges_owned;
    let bridges: &[sim_core::assets::Landmark] = if citywide {
        &brg.0.landmarks
    } else {
        let man0 = &boro.0.rings[0];
        bridges_owned = brg
            .0
            .landmarks
            .iter()
            .filter(|lm| {
                lm.surfaces.iter().any(|s| {
                    s.verts
                        .iter()
                        .any(|v| point_in_ring(Enu::new(v[0] as f64, v[1] as f64), man0))
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        &bridges_owned
    };
    if !bridges.is_empty() {
        let mesh = meshes.add(world::landmark_flat_mesh(bridges));
        // One notch up from the footprint fabric (ZINC_800) so the deck stays legible
        // over the white water + green parkland where no buildings frame it — still
        // quiet, flat, and recessive. Sits at `BRIDGE_FLAT_Z`, clearly above the
        // coastline outline (z 0.15) so a span crossing the water reads continuously
        // rather than being nicked by the boundary stroke where it leaves the shore.
        let mat = materials.add(theme::map::ZINC_700);
        commands.spawn((
            Mesh2d(mesh),
            MeshMaterial2d(mat),
            Transform::from_xyz(0.0, 0.0, BRIDGE_FLAT_Z),
            BridgeVis,
        ));
    }

    // ACE corridors (transit blue) as thick ribbons, above streets. GL lines are stuck
    // at 1px under WebGPU, so the bus-lane routes were nearly invisible — especially
    // zoomed out. `scale_ace_corridors` floors the on-screen width so they read at any
    // zoom; here we seed them at the intrinsic world width.
    if !ace_segments.is_empty() {
        let ace_mesh = meshes.add(world::thick_line_list_mesh(&ace_segments, ACE_BASE_HALF, 0.2));
        let ace_mat = materials.add(theme::map::BLUE);
        commands.spawn((
            Mesh2d(ace_mesh.clone()),
            MeshMaterial2d(ace_mat),
            Transform::from_xyz(0.0, 0.0, 0.2),
            AceVis,
            AceCorridors {
                segments: ace_segments.clone(),
                base_half: ACE_BASE_HALF,
                mesh: ace_mesh,
            },
        ));
    }

    // LinkNYC kiosks — Wi-Fi/phone hubs as a fixed point layer (solid cyan squares,
    // distinct from the textured camera icons). NOT cameras: a kiosk surveils you only
    // when you connect to its Wi-Fi (the panel note makes the point). Drawn below the
    // camera icons; zoom-floored to a legible size by `scale_markers`.
    if !link.0.kiosks.is_empty() {
        let pts: Vec<Enu> = link.0.kiosks.iter().map(|k| Enu::new(k.x, k.y)).collect();
        let mesh = meshes.add(world::merged_icon_quads(&pts, LINKNYC_BASE_HALF * 2.0));
        let mat = materials.add(theme::map::ca(0x02, 0x84, 0xc7, 0.95)); // LinkNYC sky-blue (infrastructure)
        commands.spawn((
            Mesh2d(mesh.clone()),
            MeshMaterial2d(mat),
            Transform::from_xyz(0.0, 0.0, 0.9),
            LinkNycVis,
            ScaledMarkers { pts, base_half: LINKNYC_BASE_HALF, mesh },
        ));
    }

    // Camera markers: one MERGED textured-quad mesh per class, each painting a
    // recognizable icon (CCTV camera, owl for Flock/ALPR, traffic-cam for DOT)
    // via a single ColorMaterial+texture — 3 draw calls for ~5k cameras, not one
    // entity each. FOV wedges stay per-camera (directional only, hidden unless
    // toggled, so cheap).
    let mut cctv_pts: Vec<Enu> = Vec::new();
    let mut alpr_pts: Vec<Enu> = Vec::new();
    let mut alpr_makers: Vec<operators::Maker> = Vec::new();
    let mut dot_pts: Vec<Enu> = Vec::new();
    let mut enf_pts: Vec<Enu> = Vec::new();
    let wedge_mat = materials.add(theme::map::ca(0xdc, 0x26, 0x26, 0.14)); // what the camera sees: surveillance-red cone
    // One marker (+ one FOV cone) per *physical* camera. A co-located group attested by
    // several layers used to stack a wordmark per source; now each group draws a single
    // primary marker. Priority prefers a source the click modal can open — ALPR, then CCTV
    // — matching `handle_click`'s ALPR-before-CCTV hit-test, so the visible icon is the one
    // whose popup opens; DOT/enforcement only win when no clickable member shares the group.
    let group_primary_idx: std::collections::HashMap<u32, usize> = {
        let prio = |k: sim_core::SourceKind| match k {
            sim_core::SourceKind::Alpr => 0u8,
            sim_core::SourceKind::FixedCctv => 1,
            sim_core::SourceKind::DotLiveView => 2,
            sim_core::SourceKind::EnforcementCamera => 3,
            _ => 4,
        };
        let mut m: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        for (i, s) in sensors.iter().enumerate() {
            let e = m.entry(s.group).or_insert(i);
            if prio(s.kind) < prio(sensors[*e].kind) {
                *e = i;
            }
        }
        m
    };
    let mut alpr_ord = 0usize;
    for (i, s) in sensors.iter().enumerate() {
        // ALPR maker is parallel to the ALPR *sensors* (same filter+order as the readers);
        // advance the ordinal for every ALPR even when its marker is a skipped duplicate.
        let alpr_idx = if matches!(s.kind, sim_core::SourceKind::Alpr) {
            let o = alpr_ord;
            alpr_ord += 1;
            o
        } else {
            usize::MAX
        };
        // Only the group's primary member draws; co-located duplicates are the same camera
        // (and are named in that primary's "cross-source confirmed" modal note).
        if group_primary_idx.get(&s.group) != Some(&i) {
            continue;
        }
        match s.kind {
            sim_core::SourceKind::Alpr => {
                alpr_makers.push(alpr_makers_seq.get(alpr_idx).copied().unwrap_or(operators::Maker::Other));
                alpr_pts.push(s.wedge.apex);
            }
            sim_core::SourceKind::DotLiveView => dot_pts.push(s.wedge.apex),
            sim_core::SourceKind::EnforcementCamera => enf_pts.push(s.wedge.apex),
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
    info!(
        "camera markers (one per physical camera): {} CCTV + {} ALPR + {} DOT + {} enforcement = {} (from {} attestations)",
        cctv_pts.len(),
        alpr_pts.len(),
        dot_pts.len(),
        enf_pts.len(),
        cctv_pts.len() + alpr_pts.len() + dot_pts.len() + enf_pts.len(),
        n_fixed,
    );
    // Stratify the ALPR column by reader maker: sort chips so same-maker chips are
    // contiguous (the tower fills bottom-row-first, so they form stacked bands), and
    // precompute per-vertex maker tints (4 per chip). The mesh ships white so the map
    // owls stay uniform; `operators_chip_material` swaps the tints in when stacked.
    {
        let mut order: Vec<usize> = (0..alpr_pts.len()).collect();
        let rank = |m: operators::Maker| operators::MAKERS.iter().position(|&x| x == m).unwrap_or(usize::MAX);
        order.sort_by_key(|&i| rank(alpr_makers[i]));
        alpr_pts = order.iter().map(|&i| alpr_pts[i]).collect();
        alpr_makers = order.iter().map(|&i| alpr_makers[i]).collect();
    }
    let alpr_vertex_colors: Vec<[f32; 4]> =
        alpr_makers.iter().flat_map(|m| std::iter::repeat(m.rgba()).take(4)).collect();

    for (pts, size, icon, col, vcolors) in [
        (&cctv_pts, 26.0_f32, "icons/brand_cctv.png", OperatorCol::Cctv, None),
        (&alpr_pts, 28.0, "icons/brand_alpr.png", OperatorCol::Flock, Some(&alpr_vertex_colors)),
        (&dot_pts, 28.0, "icons/brand_dot.png", OperatorCol::Dot, None),
        (&enf_pts, 26.0, "icons/brand_enforce.png", OperatorCol::Enforcement, None),
    ] {
        if pts.is_empty() {
            continue;
        }
        let mut mesh_data = world::merged_icon_quads(pts, size);
        // Give the stratified column a (white) vertex-color attribute so the shader's
        // VERTEX_COLORS path is live; the tints are written in only when stacked.
        if vcolors.is_some() {
            mesh_data.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0_f32; 4]; pts.len() * 4]);
        }
        let mesh = meshes.add(mesh_data);
        let branded = asset_server.load(icon);
        let material = materials.add(ColorMaterial {
            color: Color::WHITE,
            texture: Some(branded.clone()),
            ..default()
        });
        // Homes parallel the mesh's quad order (chip i owns verts [4i, 4i+3]) so the
        // Operators view can fly each chip from here into its column. These 3 meshes
        // carry no `BaseMap` tag — the Operators systems own them, so the street
        // visibility query never fights the chips.
        let homes: Vec<Vec2> = pts.iter().map(|p| Vec2::new(p.x as f32, p.y as f32)).collect();
        commands.spawn((
            Mesh2d(mesh.clone()),
            MeshMaterial2d(material.clone()),
            Transform::from_xyz(0.0, 0.0, 1.0),
            OperatorMesh {
                col,
                mesh,
                home_half: size * 0.5,
                base_half: size * 0.5,
                homes,
                material,
                branded,
                maker_colors: vcolors.cloned(),
            },
        ));
    }

    info!(
        "loaded {} nodes / {} edges, {} CCTV + {} ALPR + {} DOT + {} enforcement cameras, {} ACE segments ({} routes)",
        graph.node_count(),
        graph.edge_count(),
        cctv_count,
        alpr_count,
        dot_count,
        enf_count,
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

    // ---- Institutions (schools + libraries): clickable directory + surveillance score.
    // Each institution is scored by the count of *deduplicated* fixed cameras within
    // FACILITY_SCAN_M (one per physical camera `group`, matching the walkshed/headline),
    // then ranked most-watched first. Per-class markers are spawned hidden — the
    // Institutions explore view reveals them.
    {
        use sim_core::assets::FacilityKind;
        let fac_proj = EnuProjection::default();
        let scan_r2 = FACILITY_SCAN_M * FACILITY_SCAN_M;
        let mut fac_pins: Vec<FacilityPin> = fac
            .0
            .facilities
            .iter()
            .map(|f| {
                let mut groups = std::collections::HashSet::new();
                for cand in cam_index.locate_within_distance([f.x, f.y], scan_r2) {
                    groups.insert(sensors[cand.data as usize].group);
                }
                let (lat, lon) = fac_proj.to_wgs84(Enu::new(f.x, f.y));
                FacilityPin {
                    pos: Vec2::new(f.x as f32, f.y as f32),
                    name: f.name.clone(),
                    kind: f.kind,
                    subtype: f.subtype.clone(),
                    lat,
                    lon,
                    cameras_near: groups.len() as u32,
                }
            })
            .collect();
        // Surveillance ranking: most cameras nearby first, name-stable on ties.
        let ranked = rank_facilities(&fac_pins);
        // Per-class merged-quad markers in cool ink (subjects, not warm sensors).
        for (kind, color) in [
            (FacilityKind::School, theme::map::ca(0x43, 0x37, 0x8a, 0.95)), // indigo
            (FacilityKind::Library, theme::map::ca(0x0d, 0x6b, 0x66, 0.95)), // teal
        ] {
            let pts: Vec<Enu> = fac_pins
                .iter()
                .filter(|p| p.kind == kind)
                .map(|p| Enu::new(p.pos.x as f64, p.pos.y as f64))
                .collect();
            if pts.is_empty() {
                continue;
            }
            commands.spawn((
                Mesh2d(meshes.add(world::merged_icon_quads(&pts, FACILITY_MARK_SIZE))),
                MeshMaterial2d(materials.add(color)),
                Transform::from_xyz(0.0, 0.0, FACILITY_MARK_Z),
                Visibility::Hidden,
                FacilityMarker { kind },
            ));
        }
        // A single reusable highlight ring, parked off until an institution is selected.
        let highlight = commands
            .spawn((
                Mesh2d(meshes.add(Annulus::new(17.0, 23.0))),
                MeshMaterial2d(materials.add(theme::map::ca(0xc2, 0x41, 0x0c, 0.95))),
                Transform::from_xyz(0.0, 0.0, FACILITY_MARK_Z + 0.2),
                Visibility::Hidden,
                FacilityHighlight,
            ))
            .id();
        commands.insert_resource(FacilityHighlightEntity(highlight));
        info!(
            "institutions: {} loaded ({} schools, {} libraries)",
            fac_pins.len(),
            fac_pins.iter().filter(|p| p.kind == FacilityKind::School).count(),
            fac_pins.iter().filter(|p| p.kind == FacilityKind::Library).count(),
        );
        commands.insert_resource(FacilityDirectory { pins: std::mem::take(&mut fac_pins), ranked });
    }

    // Pulse-overlay pool: a handful of recycled "seen you" rings that flash on a
    // camera as the live walk passes it (replaces per-camera flash now that dots
    // are a merged static mesh).
    let ping_mesh = meshes.add(Annulus::new(9.0, 14.0));
    let ping_mat = materials.add(theme::map::YELLOW); // "you were seen" pulse
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
    let pool = agents::spawn_pool(&mut commands, &mut meshes, &mut materials, glasses_icon, bus_icon);
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

    // Aggregate every fixed camera into the neighborhood it falls in (point-in-
    // polygon, bbox-prefiltered) so the layer can show per-operator counts +
    // density. One-time at world build; all boroughs (app renders Manhattan first).
    let mut neighborhoods: Vec<NeighborhoodStat> = nb
        .0
        .neighborhoods
        .iter()
        .map(|n| NeighborhoodStat {
            name: n.name.clone(),
            borough: n.borough.clone(),
            exterior: n.exterior.clone(),
            bbox: n.bbox,
            centroid: n.centroid(),
            area_km2: (n.area_m2() / 1_000_000.0).max(1e-9),
            cctv: 0,
            dot: 0,
            alpr: 0,
            total: 0,
            density: 0.0,
        })
        .collect();
    for s in &sensors {
        let p = s.wedge.apex;
        for (i, n) in nb.0.neighborhoods.iter().enumerate() {
            if n.contains(p) {
                match s.kind {
                    sim_core::SourceKind::Alpr => neighborhoods[i].alpr += 1,
                    sim_core::SourceKind::DotLiveView => neighborhoods[i].dot += 1,
                    _ => neighborhoods[i].cctv += 1,
                }
                neighborhoods[i].total += 1;
                break; // a point lands in one neighborhood
            }
        }
    }
    for st in &mut neighborhoods {
        st.density = st.total as f64 / st.area_km2;
    }

    // Real-day trip replay: clone the baked schedules and pre-build a `Route` per
    // bus shape / taxi O-D so the runtime can `position_at` without re-deriving.
    let bus_day = bd.0.clone();
    let taxi_day = td.0.clone();
    let bus_routes: Vec<Route> = bus_day
        .shapes
        .iter()
        .map(|s| Route::from_points(s.iter().map(|p| Enu::new(p[0] as f64, p[1] as f64)).collect()))
        .collect();
    let taxi_routes: Vec<Route> = taxi_day
        .routes
        .iter()
        .map(|r| {
            Route::from_points(r.polyline.iter().map(|p| Enu::new(p[0] as f64, p[1] as f64)).collect())
        })
        .collect();
    // Turn-aware pace per taxi route: brake into corners, cruise (≤ the posted limit)
    // on straights, same trip duration. NYC's default limit is 25 mph; sharp turns are
    // taken down to ~30% of it. Computed once per pooled route (not per trip).
    let taxi_paces: Vec<sim_core::PaceProfile> = taxi_routes
        .iter()
        .map(|r| sim_core::PaceProfile::for_route(r, NYC_SPEED_LIMIT_MPS, TURN_SPEED_FRAC))
        .collect();
    commands.insert_resource(agents::ReplayState::new(taxi_day.trips.len(), bus_day.trips.len()));
    // Real per-minute service levels for the headline (ACE timetable + TLC volume).
    let real_rates = Some(sim_core::mobile::RealDayRates::from_day(&bus_day, &taxi_day));
    info!(
        "real-day replay: {} ACE bus trips ({} shapes), {} taxi trips ({} routes)",
        bus_day.trips.len(),
        bus_routes.len(),
        taxi_day.trips.len(),
        taxi_routes.len(),
    );
    if let Some(rr) = &real_rates {
        let (hw, tm) = (|h| rr.ace_headway_at(h), |h| rr.taxi_mult_at(h));
        info!(
            "real headline rates: ACE headway 08h={:.1}m 14h={:.1}m 03h={:.1}m · taxi vol 08h={:.2} 14h={:.2} 03h={:.2}",
            hw(8.0), hw(14.0), hw(3.0), tm(8.0), tm(14.0), tm(3.0)
        );
    }

    // Robotability field → per-node cumulative spawn weights (score² emphasizes the
    // high-robotability blocks so the speculative delivery robots cluster there).
    let robot_field = rf.0.clone();
    let mut robot_node_cumulative = Vec::with_capacity(graph.node_count());
    let mut racc = 0.0f32;
    for i in 0..graph.node_count() {
        let s = robot_field.score_at(graph.node_pos(i as u32), 0.05).max(0.0);
        racc += s * s;
        robot_node_cumulative.push(racc);
    }
    info!(
        "robotability field: {}x{} grid, {} nodes weighted",
        robot_field.cols,
        robot_field.rows,
        robot_node_cumulative.len()
    );

    // Tesla field → per-node cumulative spawn weights (registration density by ZIP).
    let tesla_field = tf.0.clone();
    let mut tesla_node_cumulative = Vec::with_capacity(graph.node_count());
    let mut tacc = 0.0f32;
    for i in 0..graph.node_count() {
        let d = tesla_field.intensity_at(graph.node_pos(i as u32), 0.05) as f32;
        tacc += d.max(0.0) + 0.02; // small floor so every reachable node can host a Tesla
        tesla_node_cumulative.push(tacc);
    }
    info!("tesla field: {} ZIP zones, {} nodes weighted", tesla_field.zones.len(), tesla_node_cumulative.len());

    commands.insert_resource(Sim {
        graph,
        drive_graph,
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
        neighborhoods,
        bus_day,
        taxi_day,
        taxi_paces,
        bus_routes,
        taxi_routes,
        real_rates,
        robot_field,
        robot_node_cumulative,
        tesla_field,
        tesla_node_cumulative,
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
/// A camera fly-in requested by an address pick (eased pan + zoom). `pending` is set
/// by `apply_geocode`; `fly_camera` captures the current camera as the start and
/// runs the tween. Any manual camera input cancels it.
#[derive(Resource, Default)]
struct CameraFly {
    pending: Option<FlyTo>,
    active: Option<FlyState>,
}

/// Where to fly: a single point at a fixed zoom, or fit a route's extent.
enum FlyTo {
    Point { center: Vec2, scale: f32 },
    Fit { center: Vec2, w: f32, h: f32 },
}

struct FlyState {
    from_c: Vec2,
    from_s: f32,
    to_c: Vec2,
    to_s: f32,
    t: f32,
    dur: f32,
}

impl CameraFly {
    fn request(&mut self, to: FlyTo) {
        self.pending = Some(to);
    }
    fn cancel(&mut self) {
        self.pending = None;
        self.active = None;
    }
}

/// Half the right side-panel's width (px); the fly nudges the camera so the target
/// lands centered in the *visible* map area, not behind the panel.
const PANEL_HALF_PX: f32 = 158.0;

/// Resolve a fly target to (center, scale), fitting a route extent into the usable
/// viewport (window minus the side panel) with margin, and offsetting the center so
/// the target sits in the visible map area rather than under the panel.
fn resolve_fly_target(to: FlyTo, windows: &Query<&Window>) -> (Vec2, f32) {
    let (center, scale) = match to {
        FlyTo::Point { center, scale } => (center, scale.clamp(ZOOM_MIN, ZOOM_MAX)),
        FlyTo::Fit { center, w, h } => {
            let (win_w, win_h) = windows
                .single()
                .map(|win| (win.width(), win.height()))
                .unwrap_or((1280.0, 800.0));
            let usable_w = (win_w - 340.0).max(240.0); // leave room for the side panel
            let usable_h = win_h.max(240.0);
            let sx = (w * 1.25) / (usable_w * 0.9);
            let sy = (h * 1.25) / (usable_h * 0.9);
            (center, sx.max(sy).clamp(ZOOM_MIN, ZOOM_MAX))
        }
    };
    // Shift the camera right by half the panel so the target appears left of it.
    (Vec2::new(center.x + PANEL_HALF_PX * scale, center.y), scale)
}

/// How far past the nearest shore (world m) an off-island label floats out over the water.
const LABEL_OFFSHORE_M: f32 = 420.0;
/// Minimum along-shore spacing (world m) between labels on the same shore, so the
/// clustered midtown landmarks don't stack their names on top of each other.
const LABEL_GAP_M: f32 = 380.0;
/// Leader-line intrinsic half-width (world m)…
const LEADER_BASE_HALF: f32 = 1.0;
/// …floored to this hairline on screen so the leaders never thicken into clutter.
const MIN_LEADER_HALF_PX: f32 = 0.7;

/// An off-island label placement: the object it points at, the floated label anchor
/// (out over the water), and which shore it went to (for text alignment).
struct OffshoreLabel {
    obj: Vec2,
    pos: Vec2,
    left: bool,
}

/// Float object points off the island for decluttered labeling: push each past the
/// nearest point of the coastline `ring` by `offshore_m`, then spread labels on the same
/// shore apart vertically so they keep `gap_m` of separation. Pure geometry — the caller
/// spawns the text + leader lines from the result.
fn offshore_label_anchors(
    points: &[Vec2],
    ring: &[[f64; 2]],
    offshore_m: f32,
    gap_m: f32,
) -> Vec<OffshoreLabel> {
    let mut out: Vec<OffshoreLabel> = points
        .iter()
        .map(|&obj| {
            let shore = world::nearest_on_ring(obj, ring);
            let dir = (shore - obj).normalize_or_zero();
            let dir = if dir == Vec2::ZERO { Vec2::NEG_X } else { dir };
            // East-shore labels point across the *narrow* East River toward Brooklyn /
            // Queens, so the full offshore reach lands their names on the far borough.
            // Float them a shorter distance to sit in the channel (over the water),
            // out of the Brooklyn/Queens fabric; west-shore labels keep the full reach
            // out into the wide Hudson.
            let reach = if dir.x > 0.0 { offshore_m * 0.5 } else { offshore_m };
            OffshoreLabel { obj, pos: shore + dir * reach, left: dir.x < 0.0 }
        })
        .collect();
    // De-collide within each shore: sort along y, push later ones up to keep the gap.
    for left in [true, false] {
        let mut idx: Vec<usize> = (0..out.len()).filter(|&i| out[i].left == left).collect();
        idx.sort_by(|&i, &j| out[i].pos.y.total_cmp(&out[j].pos.y));
        for k in 1..idx.len() {
            let prev_y = out[idx[k - 1]].pos.y;
            if out[idx[k]].pos.y - prev_y < gap_m {
                out[idx[k]].pos.y = prev_y + gap_m;
            }
        }
    }
    out
}

/// On-screen half-size floor (CSS px) for a fixed-camera marker when you're zoomed
/// *in* to read it. The wordmark stays this legible down at street level…
const ICON_FLOOR_NEAR_PX: f32 = 8.0;
/// …and tapers to this faint tick at the city overview, so ~5k merged wordmarks read
/// as fine texture instead of burying the basemap when zoomed out.
const ICON_FLOOR_FAR_PX: f32 = 2.75;
/// Camera zoom (m/px) at/below which the near floor holds (street/area legibility)…
const ICON_TAPER_NEAR: f32 = 2.5;
/// …and at/above which the far floor holds (island overview). Default zoom (6.0)
/// sits between, so the markers are already calmer than the old flat 10 px floor.
const ICON_TAPER_FAR: f32 = 13.0;

/// On-screen pixel floor for a fixed marker at camera `scale`, smoothstepped from the
/// legible near floor (zoomed in) down to the faint far floor (zoomed out).
fn icon_floor_px(scale: f32) -> f32 {
    let t = ((scale - ICON_TAPER_NEAR) / (ICON_TAPER_FAR - ICON_TAPER_NEAR)).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t); // smoothstep
    ICON_FLOOR_NEAR_PX + (ICON_FLOOR_FAR_PX - ICON_FLOOR_NEAR_PX) * t
}

/// On-map half-size (world m) for a camera icon at camera `scale` (m/px): the intrinsic
/// `base_half`, but never smaller on screen than `icon_floor_px(scale)`. Zoomed in,
/// `base_half` wins (icons grow on screen with the map); zoomed out, the tapering pixel
/// floor holds so the dense fixed layers shrink toward ticks instead of polluting.
fn icon_half_for(base_half: f32, scale: f32) -> f32 {
    base_half.max(icon_floor_px(scale) * scale)
}

/// Keep the fixed-camera icons a legible minimum on-screen size by rebuilding each
/// merged-quad mesh to `icon_half_for(base_half, scale)` when the zoom changes. Runs
/// only while the Operators view is fully at rest in map view (`ov.idle`) — during the
/// chip-fly, `operators_animate_fixed` owns those vertices (lerping from `home_half`,
/// which we keep current here so the fly starts at the on-screen size).
fn scale_camera_icons(
    ov: Res<OperatorsView>,
    cam: Query<&Transform, With<Camera2d>>,
    mut q: Query<&mut OperatorMesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut last_scale: Local<f32>,
    mut initialized: Local<bool>,
) {
    if !ov.idle || q.is_empty() {
        return;
    }
    let Ok(cam_t) = cam.single() else { return };
    let scale = cam_t.scale.x;
    // Resize on first run (icons just spawned) and on any meaningful zoom change.
    if *initialized && (scale - *last_scale).abs() < 1e-3 {
        return;
    }
    *last_scale = scale;
    *initialized = true;
    for mut om in &mut q {
        let half = icon_half_for(om.base_half, scale);
        om.home_half = half;
        let Some(mesh) = meshes.get_mut(&om.mesh) else {
            continue;
        };
        let Some(VertexAttributeValues::Float32x3(pos)) =
            mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
        else {
            continue;
        };
        // Same corner winding as `world::merged_icon_quads` (TL, TR, BR, BL).
        for (i, c) in om.homes.iter().enumerate() {
            let b = i * 4;
            pos[b] = [c.x - half, c.y + half, 0.0];
            pos[b + 1] = [c.x + half, c.y + half, 0.0];
            pos[b + 2] = [c.x + half, c.y - half, 0.0];
            pos[b + 3] = [c.x - half, c.y - half, 0.0];
        }
    }
}

/// Intrinsic half-width (world m) of an ACE corridor ribbon — ~half a bus lane.
const ACE_BASE_HALF: f32 = 4.0;
/// Minimum on-screen half-width (CSS px) for an ACE ribbon, so the bus-lane routes
/// stay noticeable when zoomed out instead of thinning toward invisibility.
const MIN_ACE_HALF_PX: f32 = 2.5;

/// On-map half-width (world m) of an ACE ribbon at camera `scale` (m/px): the intrinsic
/// width, floored so it never falls below `MIN_ACE_HALF_PX` on screen.
fn ace_half_for(base_half: f32, scale: f32) -> f32 {
    base_half.max(MIN_ACE_HALF_PX * scale)
}

/// Keep the ACE corridor ribbons a legible minimum on-screen width by rebuilding their
/// mesh to `ace_half_for(base_half, scale)` when the zoom changes (mirrors
/// `scale_camera_icons`). They carry no animation, so this can always own the vertices.
fn scale_ace_corridors(
    cam: Query<&Transform, With<Camera2d>>,
    q: Query<&AceCorridors>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut last_scale: Local<f32>,
    mut initialized: Local<bool>,
) {
    if q.is_empty() {
        return;
    }
    let Ok(cam_t) = cam.single() else { return };
    let scale = cam_t.scale.x;
    if *initialized && (scale - *last_scale).abs() < 1e-3 {
        return;
    }
    *last_scale = scale;
    *initialized = true;
    for ac in &q {
        let half = ace_half_for(ac.base_half, scale);
        let Some(mesh) = meshes.get_mut(&ac.mesh) else {
            continue;
        };
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            world::ribbon_positions(&ac.segments, half, 0.2),
        );
    }
}

/// Keep the off-island label leader lines a hairline-thin constant width on screen by
/// rebuilding each merged leader mesh on zoom change (mirrors `scale_ace_corridors`).
fn scale_leader_lines(
    cam: Query<&Transform, With<Camera2d>>,
    q: Query<&LeaderLines>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut last_scale: Local<f32>,
    mut initialized: Local<bool>,
) {
    if q.is_empty() {
        return;
    }
    let Ok(cam_t) = cam.single() else { return };
    let scale = cam_t.scale.x;
    if *initialized && (scale - *last_scale).abs() < 1e-3 {
        return;
    }
    *last_scale = scale;
    *initialized = true;
    for ll in &q {
        let half = ll.base_half.max(MIN_LEADER_HALF_PX * scale);
        let Some(mesh) = meshes.get_mut(&ll.mesh) else {
            continue;
        };
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            world::ribbon_positions(&ll.segments, half, 0.0),
        );
    }
}

/// Off-island leader lines follow their layer's label visibility (landmarks: hide in
/// heatmap mode + the Operators view, like the labels themselves).
fn sync_leader_visibility(
    params: Res<Params>,
    ov: Res<OperatorsView>,
    mut leaders: Query<&mut Visibility, With<LandmarkLeader>>,
) {
    set_vis(&mut leaders, params.landmarks_on && !params.heatmap_on && !ov.active);
}

/// Intrinsic half-size (world m) of a LinkNYC kiosk marker quad (a touch smaller than
/// the camera icons — a secondary layer).
const LINKNYC_BASE_HALF: f32 = 7.0;

/// Keep merged-quad marker layers (LinkNYC kiosks) a legible minimum on-screen size by
/// rebuilding their mesh to `icon_half_for(base_half, scale)` on zoom change. Same floor
/// as the camera icons, but these carry no Operators coupling so it can always run.
fn scale_markers(
    cam: Query<&Transform, With<Camera2d>>,
    q: Query<&ScaledMarkers>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut last_scale: Local<f32>,
    mut initialized: Local<bool>,
) {
    if q.is_empty() {
        return;
    }
    let Ok(cam_t) = cam.single() else { return };
    let scale = cam_t.scale.x;
    if *initialized && (scale - *last_scale).abs() < 1e-3 {
        return;
    }
    *last_scale = scale;
    *initialized = true;
    for sm in &q {
        let half = icon_half_for(sm.base_half, scale);
        let Some(mesh) = meshes.get_mut(&sm.mesh) else {
            continue;
        };
        let Some(VertexAttributeValues::Float32x3(pos)) =
            mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
        else {
            continue;
        };
        for (i, p) in sm.pts.iter().enumerate() {
            let (x, y) = (p.x as f32, p.y as f32);
            let b = i * 4;
            pos[b] = [x - half, y + half, 0.0];
            pos[b + 1] = [x + half, y + half, 0.0];
            pos[b + 2] = [x + half, y - half, 0.0];
            pos[b + 3] = [x - half, y - half, 0.0];
        }
    }
}

/// Surveillance *evidence* for the ACE corridors (NYC Open Parking & Camera Violations,
/// Socrata `nc67-uf89`, `county='NY'`, queried 2026-06; snapshot under
/// `data/snapshots/violations/`): the enforcement output of the bus lanes the ACE camera
/// corridors patrol. Bus-lane class only — the clearest camera-linked violation in the
/// dataset. The dataset carries no coordinates, so this surfaces as a narrative stat
/// tied to the ACE layer rather than a geographic map layer.
pub(crate) const ACE_BUS_LANE_VIOLATIONS: u32 = 242_742;
pub(crate) const ACE_BUS_LANE_FINES_USD: f64 = 27_911_985.0;

/// "242742" → "242,742" (thousands separators, for the evidence callout).
pub(crate) fn group_thousands(n: u32) -> String {
    let s = n.to_string();
    let len = s.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 27_911_985.0 → "$27.9M" (compact currency for the evidence callout).
pub(crate) fn compact_usd(n: f64) -> String {
    if n >= 1e9 {
        format!("${:.1}B", n / 1e9)
    } else if n >= 1e6 {
        format!("${:.1}M", n / 1e6)
    } else if n >= 1e3 {
        format!("${:.0}K", n / 1e3)
    } else {
        format!("${n:.0}")
    }
}

/// On the web, deep-link a StoryMap: `?story=tutorial` auto-plays the tutorial once the
/// world has loaded. No-op on native and when the param is absent.
fn storymap_autostart(
    mut done: Local<bool>,
    sim: Option<Res<Sim>>,
    mut story: ResMut<storymap::StoryMap>,
) {
    if *done || sim.is_none() {
        return; // run once, and only after the world has built
    }
    *done = true;
    #[cfg(target_arch = "wasm32")]
    {
        if url_story_is("longitudinal") {
            story.start("A decade of watching", storymap::longitudinal());
        } else if url_story_is("tutorial") {
            story.start("Tutorial", storymap::tutorial());
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = &mut story;
}

/// Whether the page URL carries `?story=<name>` (web only).
#[cfg(target_arch = "wasm32")]
fn url_story_is(name: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .map(|s| s.contains(&format!("story={name}")))
        .unwrap_or(false)
}

/// Drive the active StoryMap: advance its clock and, when a step is (re)entered, apply
/// its scene against the live world — camera fly, mode switch, example route/walkshed,
/// Operators view, "In 5 years…" future, or the citywide heatmap. Each step starts from
/// a clean baseline (overlays off) so step order doesn't leak state.
#[allow(clippy::too_many_arguments)]
fn storymap_tick(
    time: Res<Time>,
    mut story: ResMut<storymap::StoryMap>,
    mut params: ResMut<Params>,
    mut fly: ResMut<CameraFly>,
    mut ov: ResMut<OperatorsView>,
    sim: Option<Res<Sim>>,
    clock: Res<SimClock>,
    mut route: ResMut<RouteState>,
    mut walkshed_state: ResMut<WalkshedState>,
    mut walk_live: ResMut<WalkLive>,
    route_vis: Query<Entity, With<RouteVis>>,
    walkshed_vis: Query<Entity, With<WalkshedVis>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if !story.active {
        return;
    }
    story.tick(time.delta_secs());
    // The step's scene is applied exactly once, on entry.
    if !story.active || !std::mem::take(&mut story.apply_pending) {
        return;
    }
    let Some(action) = story.current().map(|s| s.action) else {
        return;
    };
    let proj = EnuProjection::default();
    let sim = sim.as_deref();

    // Baseline: each step opens from a clean scene; its action re-enables what it needs.
    ov.active = false;
    params.heatmap_on = false;
    params.set_future(false);

    use storymap::StepAction as A;
    match action {
        A::Caption => {}
        A::Overview => {
            fly.request(FlyTo::Point { center: Vec2::ZERO, scale: STORY_OVERVIEW_ZOOM });
        }
        A::FlyTo { lat, lon, zoom } => {
            let e = proj.to_enu(lat, lon);
            fly.request(FlyTo::Point { center: Vec2::new(e.x as f32, e.y as f32), scale: zoom });
        }
        A::Route { a, b } => {
            params.mode = Mode::Route;
            let (ea, eb) = (proj.to_enu(a.0, a.1), proj.to_enu(b.0, b.1));
            route.a = Some(ea);
            route.b = Some(eb);
            if let Some(sim) = sim {
                rebuild_route(
                    &mut route, &mut walk_live, sim, &params, clock.time_of_day, &route_vis,
                    &mut commands, &mut meshes, &mut materials,
                );
            }
            request_route_fly(&mut fly, &route, Vec2::new(ea.x as f32, ea.y as f32));
        }
        A::Walkshed { lat, lon } => {
            params.mode = Mode::Walkshed;
            let e = proj.to_enu(lat, lon);
            for ent in &walkshed_vis {
                commands.entity(ent).despawn();
            }
            if let Some(sim) = sim {
                place_walkshed(e, sim, &mut commands, &mut meshes, &mut materials, &mut walkshed_state);
            }
            fly.request(FlyTo::Point { center: Vec2::new(e.x as f32, e.y as f32), scale: FLY_AREA_ZOOM });
        }
        A::Operators => {
            ov.active = true;
        }
        A::Future => {
            params.set_future(true);
        }
        A::Heatmap => {
            params.heatmap_on = true;
            fly.request(FlyTo::Point { center: Vec2::ZERO, scale: STORY_OVERVIEW_ZOOM });
        }
        A::Scene { at, linknyc, future, operators, heatmap } => {
            // Composed era scene: baseline already cleared future/operators/heatmap, so
            // only turn on what this era needs. LinkNYC is set explicitly (then vs now).
            params.linknyc_on = linknyc;
            if future {
                params.set_future(true);
            }
            if operators {
                ov.active = true;
            }
            if heatmap {
                params.heatmap_on = true;
            }
            let (center, scale) = match at {
                Some((lat, lon, z)) => {
                    let e = proj.to_enu(lat, lon);
                    (Vec2::new(e.x as f32, e.y as f32), z)
                }
                None => (Vec2::ZERO, STORY_OVERVIEW_ZOOM),
            };
            fly.request(FlyTo::Point { center, scale });
        }
    }
}

/// Ease the camera toward a requested fly target (smoothstep pan + zoom); snaps when
/// reduced-motion is on. Runs right after `camera_control`, which cancels it on input.
fn fly_camera(
    time: Res<Time>,
    ov: Res<OperatorsView>,
    windows: Query<&Window>,
    mut fly: ResMut<CameraFly>,
    mut cam: Query<&mut Transform, With<Camera2d>>,
) {
    let Ok(mut t) = cam.single_mut() else { return };
    if let Some(to) = fly.pending.take() {
        let (to_c, to_s) = resolve_fly_target(to, &windows);
        fly.active = Some(FlyState {
            from_c: t.translation.truncate(),
            from_s: t.scale.x,
            to_c,
            to_s,
            t: 0.0,
            dur: if ov.reduced_motion { 0.0 } else { FLY_DUR },
        });
    }
    let Some(st) = &mut fly.active else { return };
    st.t += time.delta_secs();
    let f = if st.dur <= 0.0 { 1.0 } else { (st.t / st.dur).clamp(0.0, 1.0) };
    let e = f * f * (3.0 - 2.0 * f); // smoothstep
    let c = st.from_c.lerp(st.to_c, e);
    let s = st.from_s + (st.to_s - st.from_s) * e;
    t.translation.x = c.x;
    t.translation.y = c.y;
    t.scale = Vec3::splat(s);
    if f >= 1.0 {
        fly.active = None;
    }
}

/// Request the right fly for a route edit: fit the whole path once both ends route,
/// else fly to the single endpoint just placed.
fn request_route_fly(fly: &mut CameraFly, route: &RouteState, just_set: Vec2) {
    if let Some(r) = &route.route {
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in &r.points {
            minx = minx.min(p.x as f32);
            miny = miny.min(p.y as f32);
            maxx = maxx.max(p.x as f32);
            maxy = maxy.max(p.y as f32);
        }
        let center = Vec2::new((minx + maxx) * 0.5, (miny + maxy) * 0.5);
        fly.request(FlyTo::Fit { center, w: maxx - minx, h: maxy - miny });
    } else {
        fly.request(FlyTo::Point { center: just_set, scale: FLY_POINT_ZOOM });
    }
}

fn camera_control(
    mut scroll: MessageReader<MouseWheel>,
    mut cursor: MessageReader<CursorMoved>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    wants: Res<EguiWants>,
    ov: Res<OperatorsView>,
    mut drag: ResMut<DragState>,
    mut fly: ResMut<CameraFly>,
    mut q: Query<&mut Transform, With<Camera2d>>,
) {
    // The Operators view freezes the map frame so its world-space column targets
    // stay put for the whole flight.
    if ov.active || ov.t > 0.0 {
        return;
    }
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
        fly.cancel(); // manual zoom interrupts an address fly-in
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
                fly.cancel(); // manual pan interrupts an address fly-in
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
            fly.cancel(); // keyboard pan interrupts an address fly-in
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_click(
    buttons: Res<ButtonInput<MouseButton>>,
    wants: Res<EguiWants>,
    drag: Res<DragState>,
    params: Res<Params>,
    // Bundled into one tuple param: a Bevy system caps at 16 params and this was at 16.
    (clock, ov, mut geo, story, dir, mut sel, cctv_dir, mut cctv_sel, inst, fac_dir, mut fac_sel, mut fly): (Res<SimClock>, Res<OperatorsView>, ResMut<geocode::Geocoder>, Res<storymap::StoryMap>, Res<AlprDirectory>, ResMut<SelectedAlpr>, Res<CctvDirectory>, ResMut<SelectedCctv>, Res<InstitutionsView>, Res<FacilityDirectory>, ResMut<SelectedFacility>, ResMut<CameraFly>),
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
    // The Operators view owns the screen — don't place A/B points behind the towers.
    // A playing StoryMap drives the view too; clicks shouldn't fight the tour.
    if ov.active || ov.t > 0.0 || story.active {
        return;
    }
    // Place a point on click-release — but not if the cursor was dragged (pan).
    if wants.pointer || !buttons.just_released(MouseButton::Left) || drag.moved_px > 6.0 {
        return;
    }
    // The Institutions explore view owns clicks: a click near a marker selects + flies to
    // that institution; otherwise it's inert (the ranking list is the primary interface).
    if inst.active {
        if let (Ok(window), Ok((cam, cam_t))) = (windows.single(), cam_q.single()) {
            if let Some(world_pt) = window
                .cursor_position()
                .and_then(|c| cam.viewport_to_world_2d(cam_t, c).ok())
            {
                let pick_r = FACILITY_PICK_PX * cam_t.scale().x;
                let pick_r2 = pick_r * pick_r;
                if let Some((i, _)) = fac_dir
                    .pins
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i, p.pos.distance_squared(world_pt)))
                    .filter(|(_, d2)| *d2 <= pick_r2)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                {
                    fac_sel.0 = Some(i);
                    fly.request(FlyTo::Point { center: fac_dir.pins[i].pos, scale: FLY_AREA_ZOOM });
                }
            }
        }
        return;
    }
    // Neighborhood-density mode is read by hovering (the live breakdown follows the cursor),
    // so a click is inert here — no A/B points, and no marker modals (the markers are hidden).
    if matches!(params.mode, Mode::Neighborhoods) {
        return;
    }
    let Some(sim) = sim else { return };
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else { return };
    let Ok((cam, cam_t)) = cam_q.single() else { return };
    let Ok(world_pt) = cam.viewport_to_world_2d(cam_t, cursor) else { return };
    let enu = Enu::new(world_pt.x as f64, world_pt.y as f64);
    // Clicking on (or within ~ALPR_PICK_PX of) an ALPR marker opens its metadata modal
    // instead of dropping a route/area point. The world threshold scales with zoom so
    // it stays a constant on-screen radius.
    let pick_r = ALPR_PICK_PX * cam_t.scale().x;
    let pick_r2 = pick_r * pick_r;
    // Marker modals only when zoomed in far enough to be aiming at one marker; zoomed out,
    // the click belongs to the area/route, so skip the hit-tests and fall through to placement.
    if cam_t.scale().x <= MODAL_ZOOM_MAX_MPP {
        if let Some((i, _)) = dir
            .0
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.pos.distance_squared(world_pt)))
            .filter(|(_, d2)| *d2 <= pick_r2)
            .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            sel.0 = Some(i);
            cctv_sel.0 = None;
            return;
        }
        // Otherwise a CCTV marker (Amnesty/Dahir census) → its provenance modal.
        if let Some((i, _)) = cctv_dir
            .0
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.pos.distance_squared(world_pt)))
            .filter(|(_, d2)| *d2 <= pick_r2)
            .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            cctv_sel.0 = Some(i);
            sel.0 = None;
            return;
        }
    }
    // Bidirectional input: a click drops the pin *and* reverse-geocodes the nearest
    // address into the matching box (mirroring how typing an address drops a pin).
    let (click_lat, click_lon) = EnuProjection::default().to_wgs84(enu);

    match params.mode {
        Mode::None => {} // no mode selected — the map is inert until one is chosen
        Mode::Walkshed => {
            for e in &walkshed_vis {
                commands.entity(e).despawn();
            }
            place_walkshed(
                enu,
                &sim,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut walkshed_state,
            );
            walkshed_state.status = None; // a real click clears the example hint
            geo.reverse_lookup(geocode::Field::Walkshed, click_lat, click_lon);
            return;
        }
        Mode::Route => {
            // A click fills the next open endpoint; clicking with both set restarts
            // at A. Editing either endpoint — by click here or by the address search —
            // routes through `rebuild_route`, so the walk recomputes the same way.
            let field = if route.a.is_none() || route.b.is_some() {
                route.a = Some(enu);
                route.b = None;
                geo.dest.clear(); // restarting at A clears the now-stale B box
                geocode::Field::Start
            } else {
                route.b = Some(enu);
                geocode::Field::Dest
            };
            rebuild_route(
                &mut route,
                &mut walk_live,
                &sim,
                &params,
                clock.time_of_day,
                &route_vis,
                &mut commands,
                &mut meshes,
                &mut materials,
            );
            geo.reverse_lookup(field, click_lat, click_lon);
        }
        Mode::Neighborhoods => {} // unreachable (early-returned above); keeps the match total
    }
}

/// (Re)draw the A→B route from whatever `route.a` / `route.b` currently hold: clear
/// the old visuals, re-place the A (yellow) + B (orange) markers, and — when both
/// are set — route, draw the path + walker, and fill the summary. Shared by the
/// map-click handler and the address geocoder so setting/editing either endpoint
/// either way recomputes the walk identically.
#[allow(clippy::too_many_arguments)]
fn rebuild_route(
    route: &mut RouteState,
    walk_live: &mut WalkLive,
    sim: &Sim,
    params: &Params,
    time_of_day: f64,
    route_vis: &Query<Entity, With<RouteVis>>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
) {
    for e in route_vis {
        commands.entity(e).despawn();
    }
    *walk_live = WalkLive::default();
    route.route = None;
    route.summary = None;
    if let Some(a) = route.a {
        spawn_marker(commands, meshes, materials, a, theme::map::YELLOW); // A: start
    }
    if let Some(b) = route.b {
        spawn_marker(commands, meshes, materials, b, theme::map::ORANGE); // B: destination
    }
    match (route.a, route.b) {
        (Some(a), Some(b)) => {
            let mobile = build_mobile(params, sim);
            match sim_core::run_route(
                &sim.graph,
                &sim.sensors,
                &[],
                &mobile,
                a,
                b,
                sim_params(sim),
                time_of_day,
                Some(&sim.dashcam_field),
                Some(&sim.robot_field),
                Some(&sim.tesla_field),
                sim.real_rates.as_ref(),
            ) {
                Ok((r, summary)) => {
                    let line = meshes.add(world::line_strip_mesh(&r.points, 2.0));
                    let line_mat = materials.add(theme::map::YELLOW); // your path, hazard yellow
                    commands.spawn((Mesh2d(line), MeshMaterial2d(line_mat), Transform::default(), RouteVis));
                    let walker = meshes.add(Circle::new(16.0));
                    let walker_mat = materials.add(theme::map::AMBER); // amber dot on the path
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
        (Some(_), None) => route.status = "Set a destination (B).".into(),
        (None, Some(_)) => route.status = "Set a start (A).".into(),
        (None, None) => route.status = String::new(),
    }
}

/// Apply a committed address pick (from the panel's search box) to the same path a
/// click would: convert lat/lon → ENU, then set the walkshed center, route start, or
/// route destination and recompute. Off-island picks are filtered out upstream.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn apply_geocode(
    mut geo: ResMut<geocode::Geocoder>,
    params: Res<Params>,
    clock: Res<SimClock>,
    sim: Option<Res<Sim>>,
    mut route: ResMut<RouteState>,
    mut walkshed_state: ResMut<WalkshedState>,
    mut walk_live: ResMut<WalkLive>,
    route_vis: Query<Entity, With<RouteVis>>,
    walkshed_vis: Query<Entity, With<WalkshedVis>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut fly: ResMut<CameraFly>,
    mut rng: Local<Option<sim_core::rng::WyRand>>,
) {
    let Some(sim) = sim else { return };
    // "Surprise me": drop a random walkshed center / A→B pair on the street graph.
    // Persistent PRNG (seeded once) so every press lands somewhere new.
    let want_rand_walkshed = std::mem::take(&mut geo.random_walkshed);
    let want_rand_route = std::mem::take(&mut geo.random_route);
    if want_rand_walkshed || want_rand_route {
        use sim_core::rng::{RngLike, WyRand};
        let proj = EnuProjection::default();
        let n = sim.graph.node_count();
        if n > 0 {
            let rng = rng.get_or_insert_with(|| WyRand::new(0xA17E_5EED_C0FF_EE11));
            if want_rand_walkshed {
                let enu = sim.graph.node_pos(rng.below(n) as u32);
                for e in &walkshed_vis {
                    commands.entity(e).despawn();
                }
                place_walkshed(enu, &sim, &mut commands, &mut meshes, &mut materials, &mut walkshed_state);
                walkshed_state.status = None;
                let (lat, lon) = proj.to_wgs84(enu);
                geo.reverse_lookup(geocode::Field::Walkshed, lat, lon);
                fly.request(FlyTo::Point { center: Vec2::new(enu.x as f32, enu.y as f32), scale: FLY_AREA_ZOOM });
            } else {
                // Random A→B: two well-separated nodes (700–2500 m apart) for a real walk.
                let a = sim.graph.node_pos(rng.below(n) as u32);
                let mut b = sim.graph.node_pos(rng.below(n) as u32);
                for _ in 0..16 {
                    let d = a.distance(b);
                    if (700.0..=2500.0).contains(&d) {
                        break;
                    }
                    b = sim.graph.node_pos(rng.below(n) as u32);
                }
                route.a = Some(a);
                route.b = Some(b);
                let (a_lat, a_lon) = proj.to_wgs84(a);
                let (b_lat, b_lon) = proj.to_wgs84(b);
                geo.reverse_lookup(geocode::Field::Start, a_lat, a_lon);
                geo.reverse_lookup(geocode::Field::Dest, b_lat, b_lon);
                rebuild_route(&mut route, &mut walk_live, &sim, &params, clock.time_of_day, &route_vis, &mut commands, &mut meshes, &mut materials);
                request_route_fly(&mut fly, &route, Vec2::new(b.x as f32, b.y as f32));
            }
        }
        return;
    }
    // ✕ clear: drop the field's endpoint / walkshed too.
    if let Some(field) = geo.cleared.take() {
        match field {
            geocode::Field::Walkshed => {
                for e in &walkshed_vis {
                    commands.entity(e).despawn();
                }
                walkshed_state.summary = None;
                walkshed_state.status = None;
            }
            geocode::Field::Start => {
                route.a = None;
                rebuild_route(&mut route, &mut walk_live, &sim, &params, clock.time_of_day, &route_vis, &mut commands, &mut meshes, &mut materials);
            }
            geocode::Field::Dest => {
                route.b = None;
                rebuild_route(&mut route, &mut walk_live, &sim, &params, clock.time_of_day, &route_vis, &mut commands, &mut meshes, &mut materials);
            }
        }
    }
    // A↔B swap (Google-Maps reverse): swap endpoints + their labels, then recompute.
    if std::mem::take(&mut geo.swap) {
        {
            let r = &mut *route; // deref once so the two fields can be split-borrowed
            std::mem::swap(&mut r.a, &mut r.b);
        }
        {
            let g = &mut *geo;
            std::mem::swap(&mut g.start.query, &mut g.dest.query);
            std::mem::swap(&mut g.start.resolved, &mut g.dest.resolved);
        }
        rebuild_route(&mut route, &mut walk_live, &sim, &params, clock.time_of_day, &route_vis, &mut commands, &mut meshes, &mut materials);
    }
    let Some((field, result)) = geo.picked.take() else { return };
    let enu = EnuProjection::default().to_enu(result.lat, result.lon);
    let center = Vec2::new(enu.x as f32, enu.y as f32);
    match field {
        geocode::Field::Walkshed => {
            for e in &walkshed_vis {
                commands.entity(e).despawn();
            }
            place_walkshed(enu, &sim, &mut commands, &mut meshes, &mut materials, &mut walkshed_state);
            walkshed_state.status = None;
            fly.request(FlyTo::Point { center, scale: FLY_AREA_ZOOM });
        }
        geocode::Field::Start => {
            route.a = Some(enu);
            rebuild_route(&mut route, &mut walk_live, &sim, &params, clock.time_of_day, &route_vis, &mut commands, &mut meshes, &mut materials);
            request_route_fly(&mut fly, &route, center);
        }
        geocode::Field::Dest => {
            route.b = Some(enu);
            rebuild_route(&mut route, &mut walk_live, &sim, &params, clock.time_of_day, &route_vis, &mut commands, &mut meshes, &mut materials);
            request_route_fly(&mut fly, &route, center);
        }
    }
}

/// The highlight-ring color for an in-shed camera, by its source layer (mirrors the
/// map markers + Operators-view colors): maroon CCTV, amber DOT, red ALPR/Flock,
/// orange enforcement.
fn walkshed_ring_color(kind: sim_core::SourceKind) -> Color {
    use sim_core::SourceKind::*;
    let c = match kind {
        FixedCctv => theme::map::MAROON,
        DotLiveView => theme::map::AMBER_700,
        Alpr => theme::map::RED,
        EnforcementCamera => theme::map::ORANGE_600,
        _ => theme::map::ORANGE_600, // other kinds shouldn't appear among fixed sensors
    };
    c.with_alpha(0.95)
}

/// Convex hull (Andrew's monotone chain) of ENU points, as a CCW ring of `[f64;2]`
/// (no duplicated closing vertex). `None` for fewer than three distinct points.
/// Used to draw the walkshed isochrone — the smallest convex shape that
/// *encompasses* every reachable street.
fn convex_hull(pts: &[Enu]) -> Option<Vec<[f64; 2]>> {
    let mut p: Vec<[f64; 2]> = pts.iter().map(|v| [v.x, v.y]).collect();
    p.sort_by(|a, b| {
        a[0].partial_cmp(&b[0])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal))
    });
    p.dedup();
    if p.len() < 3 {
        return None;
    }
    let cross = |o: &[f64; 2], a: &[f64; 2], b: &[f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for &pt in &p {
        while lower.len() >= 2 && cross(&lower[lower.len() - 2], &lower[lower.len() - 1], &pt) <= 0.0 {
            lower.pop();
        }
        lower.push(pt);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for &pt in p.iter().rev() {
        while upper.len() >= 2 && cross(&upper[upper.len() - 2], &upper[upper.len() - 1], &pt) <= 0.0 {
            upper.pop();
        }
        upper.push(pt);
    }
    lower.pop(); // shared endpoints
    upper.pop();
    lower.extend(upper);
    if lower.len() >= 3 {
        Some(lower)
    } else {
        None
    }
}

/// Compute + draw a 10-minute walkshed centered on `enu`: the isochrone area (a
/// translucent gold hull encompassing the reach), the reachable streets (warm
/// gold), the in-shed cameras (cold rings), and a center marker, plus the panel
/// summary. Shared by the click handler and the seeded example so both paint
/// identically. Caller is responsible for despawning any prior `WalkshedVis`.
fn place_walkshed(
    enu: Enu,
    sim: &Sim,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    walkshed_state: &mut WalkshedState,
) {
    let Some(node) = sim.graph.snap_nearest(enu) else { return };
    let ws = sim.graph.walkshed(node, WALKSHED_SECONDS, WALK_SPEED);
    let recall = 1.0 / sim.layer.recall.unwrap_or(1.0);
    let summary = sim_core::walkshed_exposure(&sim.graph, &ws, &sim.sensors, &[], recall);

    // Isochrone: the convex hull of every reachable node, drawn as a translucent
    // gold wash + a deeper-amber boundary so the 10-minute study area reads as one
    // shape. Beneath the gold streets (z 0.06/0.14 < 0.12) so the network stays crisp.
    let reach_pts: Vec<Enu> = ws.node_time.keys().map(|&id| sim.graph.node_pos(id)).collect();
    if let Some(hull) = convex_hull(&reach_pts) {
        if let Some(fill) = world::filled_polygon_mesh(&hull, 0.06) {
            commands.spawn((
                Mesh2d(meshes.add(fill)),
                MeshMaterial2d(materials.add(theme::map::ca(0xfa, 0xcc, 0x15, 0.07))),
                Transform::from_xyz(0.0, 0.0, 0.06),
                WalkshedVis,
            ));
        }
        if let Some(outline) = world::stroke_ring_mesh(&hull, 5.0, 0.14) {
            commands.spawn((
                Mesh2d(meshes.add(outline)),
                MeshMaterial2d(materials.add(theme::map::ca(0xb4, 0x53, 0x09, 0.75))),
                Transform::from_xyz(0.0, 0.0, 0.14),
                WalkshedVis,
            ));
        }
    }

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
        let mat = materials.add(theme::map::YELLOW); // reachable streets: your 10-min reach
        commands.spawn((Mesh2d(mesh), MeshMaterial2d(mat), Transform::from_xyz(0.0, 0.0, 0.12), WalkshedVis));
    }
    // In-shed cameras: a ring around each, colored by its **layer** — CCTV gray, DOT
    // steel, ALPR/Flock yellow, enforcement orange — so the highlighted state shows
    // *which kinds* of cameras are watching, not just that some are. An annulus (not a
    // filled disc) so the camera icon reads inside its ring.
    let ring = meshes.add(Annulus::new(15.0, 20.0));
    let mut ring_mats: std::collections::HashMap<u8, Handle<ColorMaterial>> = Default::default();
    for (p, kind) in summary.camera_points.iter().zip(summary.camera_kinds.iter()) {
        let mat = ring_mats
            .entry(*kind as u8)
            .or_insert_with(|| materials.add(walkshed_ring_color(*kind)))
            .clone();
        commands.spawn((
            Mesh2d(ring.clone()),
            MeshMaterial2d(mat),
            Transform::from_translation(world::to_world(*p, 1.6)),
            WalkshedVis,
        ));
    }
    // Center marker (where you're standing).
    let center = meshes.add(Circle::new(22.0));
    let center_mat = materials.add(theme::map::YELLOW); // where you stand
    commands.spawn((
        Mesh2d(center),
        MeshMaterial2d(center_mat),
        Transform::from_translation(world::to_world(enu, 3.0)),
        WalkshedVis,
    ));
    walkshed_state.summary = Some(summary);
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
        walk_live.mobile_robot = 0;
        walk_live.mobile_tesla = 0;
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
            Mode::None => String::new(),
            Mode::Route => "Click to set start (A), then destination (B).".into(),
            Mode::Walkshed => "Click a point to map its 10-minute walkshed.".into(),
            Mode::Neighborhoods => "Hover a neighborhood for its camera breakdown.".into(),
        },
        ..default()
    };
    walkshed_state.summary = None;
    walkshed_state.status = None;
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
    clock: Res<SimClock>,
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
        // Quantized clock step (not raw bits): the headline re-evaluates every
        // ~30 sim-min as the day runs, instead of every frame.
        hour_step: clock_hour_step(clock.time_of_day),
        ace_on: params.ace_on,
        dashcam_on: params.dashcam_on,
        glasses_on: params.glasses_on,
        robots_on: params.robots_on,
        tesla_on: params.tesla_on,
        pen_bits: params.dashcam_penetration.to_bits(),
        per1000_bits: params.glasses_per_1000.to_bits(),
        robots_bits: params.robots_density.to_bits(),
        tesla_bits: params.tesla_density.to_bits(),
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
        &r, &nearby, &[], &mobile, sim_params(&sim), clock.time_of_day,
        Some(&sim.dashcam_field),
        Some(&sim.robot_field),
        Some(&sim.tesla_field),
        sim.real_rates.as_ref(),
    );
    route.summary = Some(summary);
}

/// Signature of the inputs that change the analytical route summary; recompute
/// fires only when this differs (see [`recompute_on_change`]).
#[derive(PartialEq, Eq, Clone, Copy)]
struct SummarySig {
    route_len_bits: u64,
    route_points: usize,
    hour_step: i64,
    ace_on: bool,
    dashcam_on: bool,
    glasses_on: bool,
    robots_on: bool,
    tesla_on: bool,
    pen_bits: u32,
    per1000_bits: u32,
    robots_bits: u32,
    tesla_bits: u32,
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

/// Advance the master clock when playing (cycling 0→24→0). Paused by the time
/// panel / reduced motion, and while the Operators view holds the world frozen.
fn advance_clock(
    time: Res<Time>,
    ov: Res<OperatorsView>,
    cov_view: Res<coverage::CoverageView>,
    mut clock: ResMut<SimClock>,
) {
    // The roving-coverage overlay drives its own single-day clock.
    if !clock.playing || ov.active || ov.t > 0.0 || cov_view.active {
        return;
    }
    let dh = clock.rate * time.delta_secs_f64() / 3600.0;
    clock.time_of_day = (clock.time_of_day + dh).rem_euclid(24.0);
}

fn animate_walker(
    time: Res<Time>,
    route: Res<RouteState>,
    ov: Res<OperatorsView>,
    clock: Res<SimClock>,
    mut q: Query<(&mut Transform, &mut Walker)>,
) {
    if ov.active || ov.t > 0.0 {
        return; // walker freezes (faded) while the Operators view is up
    }
    if !clock.playing {
        return; // paused: the walker obeys the master clock like everything else
    }
    let Some(r) = &route.route else { return };
    if r.total_m <= 0.0 {
        return;
    }
    for (mut t, mut w) in &mut q {
        // The walker now rides the master clock rate (unified with the agents).
        w.progress_m += WALK_SPEED * clock.rate * time.delta_secs_f64();
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
#[allow(clippy::type_complexity)]
fn sync_visibility(
    params: Res<Params>,
    ov: Res<OperatorsView>,
    mut base: Query<&mut Visibility, (With<BaseMap>, Without<FovWedge>, Without<AceVis>, Without<OutlineVis>, Without<agents::MobileVis>, Without<OperatorMesh>)>,
    mut fov: Query<&mut Visibility, (With<FovWedge>, Without<BaseMap>, Without<AceVis>, Without<OutlineVis>, Without<agents::MobileVis>, Without<OperatorMesh>)>,
    mut ace: Query<&mut Visibility, (With<AceVis>, Without<BaseMap>, Without<FovWedge>, Without<OutlineVis>, Without<agents::MobileVis>, Without<OperatorMesh>)>,
    mut outline: Query<&mut Visibility, (With<OutlineVis>, Without<BaseMap>, Without<FovWedge>, Without<AceVis>, Without<agents::MobileVis>, Without<OperatorMesh>)>,
    mut opmesh: Query<&mut Visibility, (With<OperatorMesh>, Without<BaseMap>, Without<FovWedge>, Without<AceVis>, Without<OutlineVis>, Without<agents::MobileVis>)>,
    mut mobile: Query<(&mut Visibility, &agents::MobileAgent), (With<agents::MobileVis>, Without<BaseMap>, Without<FovWedge>, Without<AceVis>, Without<OutlineVis>, Without<OperatorMesh>)>,
) {
    let hm = params.heatmap_on;
    set_vis(&mut base, !hm);
    set_vis(&mut fov, params.show_fov && !hm);
    set_vis(&mut ace, params.show_ace && !hm);
    // Coastline frame follows its toggle; hidden in heatmap mode like the base map.
    set_vis(&mut outline, params.outline_on && !hm);
    // Camera-icon meshes: hidden in heatmap mode, but always shown in the Operators
    // view (they're the chips flying into the columns).
    set_vis(&mut opmesh, !hm || ov.active);
    // Agents: hidden in heatmap mode or when toggled off; otherwise each follows
    // its own `active` flag (population scaling). The Operators view forces the
    // active ones visible so every column is populated.
    let show_agents = ov.active || (params.show_agents && !hm);
    for (mut vis, agent) in &mut mobile {
        // Delivery robots are excluded from the operator tower (speculative, not a
        // mapped operator), so hide them while the Operators view is up rather than
        // leaving them frozen on the map. Teslas now have their own column.
        let excluded_in_ov = ov.active && agent.class == agents::AgentClass::DeliveryRobot;
        let target = if show_agents && agent.active && !excluded_in_ov {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != target {
            *vis = target;
        }
    }
}

/// Footprint fabric + landmark massing follow their own toggles (and hide in heatmap
/// mode like the rest of the base map). Kept in its own system so the two new markers
/// don't multiply `sync_visibility`'s mutually-exclusive query set.
#[allow(clippy::type_complexity)]
fn sync_building_visibility(
    params: Res<Params>,
    ov: Res<OperatorsView>,
    mut buildings: Query<&mut Visibility, (With<BuildingVis>, Without<LandmarkVis>, Without<LandmarkLabel>, Without<LinkNycVis>, Without<ParksVis>, Without<PlazasVis>)>,
    mut landmarks: Query<&mut Visibility, (Or<(With<LandmarkVis>, With<BridgeVis>)>, Without<BuildingVis>, Without<LandmarkLabel>, Without<LinkNycVis>, Without<ParksVis>, Without<PlazasVis>)>,
    mut labels: Query<&mut Visibility, (With<LandmarkLabel>, Without<BuildingVis>, Without<LandmarkVis>, Without<LinkNycVis>, Without<ParksVis>, Without<PlazasVis>)>,
    mut linknyc: Query<&mut Visibility, (With<LinkNycVis>, Without<BuildingVis>, Without<LandmarkVis>, Without<LandmarkLabel>, Without<ParksVis>, Without<PlazasVis>)>,
    mut parks: Query<&mut Visibility, (With<ParksVis>, Without<BuildingVis>, Without<LandmarkVis>, Without<LandmarkLabel>, Without<LinkNycVis>, Without<PlazasVis>)>,
    mut plazas: Query<&mut Visibility, (With<PlazasVis>, Without<BuildingVis>, Without<LandmarkVis>, Without<LandmarkLabel>, Without<LinkNycVis>, Without<ParksVis>)>,
) {
    let hm = params.heatmap_on;
    // Only the *footprints* clear out under the neighborhood choropleth: the opaque fabric
    // sits *above* the choropleth fill (z −0.1 > −0.2) and was covering it block-by-block
    // (the "footprints over the heatmap" artifact). The 3D landmarks (z 2.8) stay — they're
    // orientation anchors that read fine over the flat density wash.
    // Footprints clear only under the actual choropleth fill (the heatmap toggle), not
    // the bare hover-browser — there's nothing to cover when the fill is off.
    let nb = params.neighborhoods_on && params.choropleth_on;
    set_vis(&mut buildings, params.buildings_on && !hm && !nb);
    // Parks share the footprints' gating (own toggle): they hide under the heatmap +
    // choropleth so the density wash reads clean, like the building fabric above them.
    set_vis(&mut parks, params.parks_on && !hm && !nb);
    // Plazas (fill + hatch) follow the same ground-fabric gating, own toggle.
    set_vis(&mut plazas, params.plazas_on && !hm && !nb);
    set_vis(&mut landmarks, params.landmarks_on && !hm);
    // Labels clear out of the heatmap overlay and the Operators view (where the map
    // recedes), but stay over the choropleth alongside their landmarks.
    set_vis(&mut labels, params.landmarks_on && !hm && !ov.active);
    // Kiosks hide in heatmap mode and behind the Operators towers (they aren't an
    // operator column), like the labels.
    set_vis(&mut linknyc, params.linknyc_on && !hm && !ov.active);
}

/// Heatmap colormap — matplotlib **inferno**, oriented so density reads as *darkness*
/// (low = pale yellow, high = near-black through orange→red→magenta→purple). Inferno is
/// perceptually uniform in lightness; sampling it at **11 anchors** (every 0.1) keeps the
/// piecewise-linear interpolation close to the true curve, so each density step carries
/// equal visual weight on the white-paper ground (no bunching). Anchors are inferno
/// reversed (1.0 pale → 0.0 ink) so denser blocks read as more ink — a surveillance field.
const HEAT_COLORS: [Color; 11] = [
    Color::srgb_u8(0xfc, 0xff, 0xa4), // inferno 1.0 — pale (lowest density)
    Color::srgb_u8(0xf6, 0xd7, 0x46), // 0.9
    Color::srgb_u8(0xfc, 0xa5, 0x0a), // 0.8
    Color::srgb_u8(0xf3, 0x78, 0x19), // 0.7
    Color::srgb_u8(0xdd, 0x51, 0x3a), // 0.6
    Color::srgb_u8(0xbc, 0x37, 0x54), // 0.5
    Color::srgb_u8(0x93, 0x26, 0x67), // 0.4
    Color::srgb_u8(0x6a, 0x17, 0x6e), // 0.3
    Color::srgb_u8(0x42, 0x0a, 0x68), // 0.2
    Color::srgb_u8(0x16, 0x0b, 0x39), // 0.1
    Color::srgb_u8(0x00, 0x00, 0x04), // 0.0 — near-black (highest density)
];

/// Constant fill opacity for the heatmap. Held constant (not ramped with density) so
/// the *color* alone conveys density — a density-linked alpha would double the lightness
/// gradient and break the colormap's perceptual uniformity. High enough that nothing
/// bleeds through; genuinely-empty cells still go fully clear (see [`heat_rgba`]).
const HEAT_ALPHA: f32 = 0.9;

/// Rebuild the colored heatmap meshes when the mode/class changes.
#[allow(clippy::type_complexity)]
/// Signature of the inputs that change the heatmap field; recompute fires only
/// when this differs (heatmap evaluation is a one-shot grid sweep, not per-frame).
#[derive(PartialEq, Eq, Clone, Copy)]
struct HeatSig {
    on: bool,
    class: u8,
    hour_step: i64,
    pen_bits: u32,
    per1000_bits: u32,
    ace_on: bool,
    dashcam_on: bool,
    glasses_on: bool,
}

/// Map a normalized intensity (0..1) to an RGBA byte color along `HEAT_COLORS`. Empty
/// cells (no exposure) stay fully clear; every non-empty cell uses the same opacity, so
/// density is read from the *color* alone (see [`HEAT_ALPHA`]).
fn heat_rgba(norm: f64) -> [u8; 4] {
    if norm <= 1e-6 {
        return [0, 0, 0, 0];
    }
    let t = norm.clamp(0.0, 1.0);
    let last = HEAT_COLORS.len() - 1;
    let pos = t as f32 * last as f32;
    let i = (pos.floor() as usize).min(last - 1);
    let f = pos - i as f32;
    let a = HEAT_COLORS[i].to_srgba();
    let b = HEAT_COLORS[i + 1].to_srgba();
    let lerp = |x: f32, y: f32| x + (y - x) * f;
    [
        (lerp(a.red, b.red) * 255.0) as u8,
        (lerp(a.green, b.green) * 255.0) as u8,
        (lerp(a.blue, b.blue) * 255.0) as u8,
        (HEAT_ALPHA * 255.0) as u8,
    ]
}

/// Render the exposure heatmap as a continuous **spatial field**: sample
/// `exposure_rates_per_minute` over a grid covering the Manhattan extent, map it
/// through the heat gradient with alpha, and paint it as one translucent textured
/// quad. Far more legible than per-street lines, and it reads as coverage over
/// *space* (not just where streets happen to run). Recomputed only when the
/// class / hour / sliders change (see `HeatSig`).
/// Debounce + amortization state for the heatmap. Don't rebuild on every changed
/// frame (wait for the controls to settle), and don't sweep the whole grid in one
/// frame either — spread it over several so a live time-lapse (which steps the hour,
/// hence the field, every few seconds) never hitches. The currently-shown heatmap
/// stays up until the next field finishes, so the swap is seamless.
#[derive(Default)]
struct HeatDebounce {
    pending: Option<HeatSig>,
    stable_frames: u32,
    built: Option<HeatSig>,
    build: Option<HeatBuild>,
}

/// An in-progress amortized grid sweep: fixed inputs captured at start, plus the
/// row cursor + accumulating field. Advanced a slice of rows per frame.
struct HeatBuild {
    sig: HeatSig,
    gw: usize,
    gh: usize,
    min_x: f64,
    max_y: f64,
    dx: f64,
    dy: f64,
    bounds: [f32; 4], // x0, x1, y0, y1
    mobile: MobileScenario,
    recall: f64,
    ace_tree: Option<RTree<[f64; 2]>>,
    ace_r2: f64,
    hour: f64,
    class: HeatClass,
    row: usize,
    values: Vec<f64>,
    max_v: f64,
}

/// Frames the inputs must hold steady before a (potentially expensive) rebuild.
const HEAT_DEBOUNCE_FRAMES: u32 = 6;
/// Grid rows swept per frame while a heatmap field is (re)building — keeps any one
/// frame's work tiny (≤ this × grid width cells) so the time-lapse stays smooth.
const HEAT_ROWS_PER_FRAME: usize = 8;

#[allow(clippy::too_many_arguments)]
fn rebuild_heatmap(
    params: Res<Params>,
    clock: Res<SimClock>,
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
        // Fixed-camera coverage is hour-invariant, so the Fixed field (the default
        // class) must NOT track the clock — otherwise it rebuilds every hour-step as
        // the day runs for an identical result. Only the mobile-bearing classes do.
        hour_step: if matches!(params.heatmap_class, HeatClass::Fixed) {
            0
        } else {
            clock_hour_step(clock.time_of_day)
        },
        pen_bits: params.dashcam_penetration.to_bits(),
        per1000_bits: params.glasses_per_1000.to_bits(),
        ace_on: params.ace_on,
        dashcam_on: params.dashcam_on,
        glasses_on: params.glasses_on,
    };
    // Toggled off: drop the field immediately (cheap; no settle needed).
    if !params.heatmap_on {
        for e in &existing {
            commands.entity(e).despawn();
        }
        deb.build = None;
        deb.pending = Some(cur);
        deb.built = Some(cur);
        return;
    }
    // Settle: a changed target restarts the counter so dragging a slider/hour
    // coalesces into one sweep instead of starting one per step.
    if deb.pending != Some(cur) {
        deb.pending = Some(cur);
        deb.stable_frames = 0;
        return;
    }
    deb.stable_frames = deb.stable_frames.saturating_add(1);
    let Some(sim) = sim else { return };

    // Start a fresh sweep only once the controls have settled and the target differs
    // from what's shown; an in-progress sweep for the current target keeps advancing.
    let building_cur = deb.build.as_ref().map(|b| b.sig) == Some(cur);
    if !building_cur {
        if deb.built == Some(cur) {
            return; // already showing this exact field
        }
        if deb.stable_frames < HEAT_DEBOUNCE_FRAMES {
            return; // still settling
        }
        match init_heat_build(cur, &params, &sim, clock.time_of_day) {
            Some(b) => deb.build = Some(b),
            None => return, // graph not ready
        }
    }

    // Advance the sweep a slice of rows; the heatmap already on screen stays up until
    // this finishes, so the swap never flickers and never sweeps a whole grid at once.
    let b = deb.build.as_mut().unwrap();
    advance_heat_rows(b, &sim, HEAT_ROWS_PER_FRAME);
    if b.row < b.gh {
        return; // more rows next frame
    }
    let (mesh, mat) = finish_heat_build(b, &mut meshes, &mut materials, &mut images);
    for e in &existing {
        commands.entity(e).despawn();
    }
    commands.spawn((
        Mesh2d(mesh),
        MeshMaterial2d(mat),
        Transform::from_xyz(0.0, 0.0, 0.15),
        HeatmapVis,
    ));
    deb.built = Some(cur);
    deb.build = None;
}

/// Begin an amortized heatmap sweep: capture the island extent, the grid, and the
/// hour-fixed inputs (mobile scenario + ACE index). `None` if the graph isn't loaded.
fn init_heat_build(sig: HeatSig, params: &Params, sim: &Sim, hour: f64) -> Option<HeatBuild> {
    let nodes = &sim.graph.asset().nodes;
    if nodes.is_empty() {
        return None;
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

    // Grid sized to ~45 m cells, capped so the whole sweep stays bounded.
    let cell = 45.0_f64;
    let gw = ((w_m / cell).round() as usize).clamp(48, 280);
    let gh = ((h_m / cell).round() as usize).clamp(48, 360);
    let (dx, dy) = (w_m / gw as f64, h_m / gh as f64);

    let mobile = build_mobile(params, sim);
    let recall = 1.0 / sim.layer.recall.unwrap_or(1.0);
    let need_ace = params.ace_on
        && matches!(params.heatmap_class, HeatClass::Ace | HeatClass::Total)
        && !sim.ace_segments.is_empty();
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

    Some(HeatBuild {
        sig,
        gw,
        gh,
        min_x,
        max_y,
        dx,
        dy,
        bounds: [min_x as f32, max_x as f32, min_y as f32, max_y as f32],
        mobile,
        recall,
        ace_tree,
        ace_r2,
        hour,
        class: params.heatmap_class,
        row: 0,
        values: vec![0.0_f64; gw * gh],
        max_v: 1e-9_f64,
    })
}

/// Sweep up to `rows` more grid rows of the in-progress field (row 0 = north / max_y).
fn advance_heat_rows(b: &mut HeatBuild, sim: &Sim, rows: usize) {
    let mut scratch: Vec<sim_core::SensorInstance> = Vec::new();
    let end = (b.row + rows).min(b.gh);
    while b.row < end {
        let y = b.max_y - (b.row as f64 + 0.5) * b.dy;
        for col in 0..b.gw {
            let x = b.min_x + (col as f64 + 0.5) * b.dx;
            let p = Enu::new(x, y);
            scratch.clear();
            for c in sim.cam_index.locate_within_distance([x, y], sim.cam_query_r2) {
                scratch.push(sim.sensors[c.data as usize]);
            }
            let near_ace = b
                .ace_tree
                .as_ref()
                .is_some_and(|t| t.locate_within_distance([x, y], b.ace_r2).next().is_some());
            let r = sim_core::exposure_rates_per_minute(
                p, b.hour, &scratch, &[], near_ace, &b.mobile, b.recall,
                Some(&sim.dashcam_field),
                Some(&sim.robot_field),
                Some(&sim.tesla_field),
                sim.real_rates.as_ref(),
            );
            let v = match b.class {
                HeatClass::Total => r.total(),
                HeatClass::Fixed => r.fixed,
                HeatClass::Ace => r.ace,
                HeatClass::Dashcam => r.dashcam,
            };
            b.values[b.row * b.gw + col] = v;
            if v > b.max_v {
                b.max_v = v;
            }
        }
        b.row += 1;
    }
}

/// Turn the completed field into a smoothed translucent texture on one quad over the
/// extent (north = max_y maps to v = 0 / top texel row).
fn finish_heat_build(
    b: &HeatBuild,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    images: &mut Assets<Image>,
) -> (Handle<Mesh>, Handle<ColorMaterial>) {
    let mut data = vec![0u8; b.gw * b.gh * 4];
    for (i, &v) in b.values.iter().enumerate() {
        let px = heat_rgba(v / b.max_v);
        data[i * 4..i * 4 + 4].copy_from_slice(&px);
    }
    let mut image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: b.gw as u32,
            height: b.gh as u32,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::linear(); // smooth the field
    let tex = images.add(image);

    let [x0, x1, y0, y1] = b.bounds;
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
    (
        meshes.add(mesh),
        materials.add(ColorMaterial { color: Color::WHITE, texture: Some(tex), ..default() }),
    )
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
            Mode::None => String::new(),
            Mode::Route => "Click to set start (A), then destination (B).".into(),
            Mode::Walkshed => "Click a point to map its 10-minute walkshed.".into(),
            Mode::Neighborhoods => "Hover a neighborhood for its camera breakdown.".into(),
        },
        ..default()
    };
    walkshed_state.summary = None;
    walkshed_state.status = None;
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
        // Diversity ramp: dim zinc (homogeneous) -> hazard yellow (diverse). The
        // diverse ground glows in caution yellow — precisely where surveillance
        // clusters (the Dahir thesis).
        let t = (bg.entropy / MAX_ENTROPY).clamp(0.0, 1.0) as f32;
        let lo = [0xe4, 0xe4, 0xe7]; // light paper (homogeneous)
        let hi = [0xfa, 0xcc, 0x15]; // hazard yellow (diverse)
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

/// (Re)build the neighborhood layer on toggle: a camera-density choropleth fill,
/// boundary outlines, and name labels. Manhattan only unless "all boroughs" is on.
/// Mirrors `rebuild_equity`'s rebuild-on-change shape.
/// Live camera density (fixed + the mobile sensors present right now) per km².
fn live_density(st: &NeighborhoodStat, m: MobileCount) -> f64 {
    (st.total + m.total()) as f64 / st.area_km2.max(1e-9)
}

/// Choropleth ramp — the warm half of matplotlib **inferno** (pale yellow → orange →
/// rose-red). A two-stop gray→orange lerp (the old ramp) isn't perceptually uniform;
/// a contiguous slice of inferno is, so equal density steps read as equal color steps.
/// It stops short of inferno's near-black end so the ink neighborhood labels stay
/// legible. Translucent so the street grid + cameras still read on top.
const CHOROPLETH_COLORS: [Color; 6] = [
    Color::srgb_u8(0xfc, 0xff, 0xa4), // pale (lowest density)
    Color::srgb_u8(0xf6, 0xd7, 0x46),
    Color::srgb_u8(0xfc, 0xa5, 0x0a),
    Color::srgb_u8(0xf3, 0x78, 0x19),
    Color::srgb_u8(0xdd, 0x51, 0x3a),
    Color::srgb_u8(0xbc, 0x37, 0x54), // deep rose-red (highest density)
];
// Translucent over white: the pale low end blends toward white, which (with the linear
// stretch) is what makes the gradient read — too much alpha saturates everyone into a
// uniform rose. 0.62 keeps the spread and still lets the street grid through for context.
const CHOROPLETH_ALPHA: f32 = 0.62;

/// Choropleth fill color for a normalized density `t` (0..1) along `CHOROPLETH_COLORS`.
fn choropleth_color(t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let last = CHOROPLETH_COLORS.len() - 1;
    let pos = t * last as f32;
    let i = (pos.floor() as usize).min(last - 1);
    let f = pos - i as f32;
    let a = CHOROPLETH_COLORS[i].to_srgba();
    let b = CHOROPLETH_COLORS[i + 1].to_srgba();
    let lerp = |x: f32, y: f32| x + (y - x) * f;
    Color::srgba(lerp(a.red, b.red), lerp(a.green, b.green), lerp(a.blue, b.blue), CHOROPLETH_ALPHA)
}

/// Contrast-stretch endpoints (low, high) for the choropleth: the 5th / 95th
/// percentile of the visible neighborhoods' live densities. Dividing by the *peak*
/// (the old approach) left Manhattan's uniformly-high densities bunched in the warm
/// end — nothing reached the pale low. Stretching the actual spread across the full
/// ramp makes relative differences read; clipping at the 5th/95th percentile keeps a
/// single hyper-dense (or empty) neighborhood from flattening everyone else.
fn density_range(sim: &Sim, live: &NeighborhoodLive, all: bool) -> (f64, f64) {
    let mut ds: Vec<f64> = sim
        .neighborhoods
        .iter()
        .enumerate()
        .filter(|(_, st)| all || st.borough == "Manhattan")
        .map(|(i, st)| live_density(st, live.get(i)))
        .collect();
    if ds.len() < 2 {
        let v = ds.first().copied().unwrap_or(1.0);
        return (0.0, v.max(1e-9));
    }
    ds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pct = |p: f64| ds[((p * (ds.len() - 1) as f64).round() as usize).min(ds.len() - 1)];
    let (lo, hi) = (pct(0.05), pct(0.95));
    if hi - lo < 1e-9 {
        (ds[0], ds[ds.len() - 1].max(ds[0] + 1e-9)) // degenerate spread → full min/max
    } else {
        (lo, hi)
    }
}

/// Normalized density (0..1) for the choropleth ramp via the contrast-stretch range.
/// A plain linear stretch between the 5th/95th percentiles reads best here: the moderate
/// majority spreads pale→orange while the dense cores anchor the deep-red end, giving a
/// clean north→south gradient. (A log stretch was tried — it lifted the low end and flattened
/// everyone back into a uniform rose; the pale low end is exactly what makes differences pop.)
fn choropleth_t(density: f64, lo: f64, hi: f64) -> f32 {
    ((density - lo) / (hi - lo).max(1e-9)).clamp(0.0, 1.0) as f32
}

#[allow(clippy::type_complexity)]
fn rebuild_neighborhoods(
    params: Res<Params>,
    sim: Option<Res<Sim>>,
    live: Res<NeighborhoodLive>,
    asset_server: Res<AssetServer>,
    mut last: Local<Option<(bool, bool, bool)>>,
    vis: Query<Entity, With<NeighborhoodVis>>,
    labels: Query<Entity, With<NeighborhoodLabel>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    // The ExtraBold cut (matches the panel's display face) for the live "mobile" count,
    // so the hazard-yellow row reads heavier over the choropleth. Handle is path-cached.
    let bold: Handle<Font> = asset_server.load("fonts/HostGrotesk-ExtraBold.ttf");
    let key = (
        params.neighborhoods_on,
        params.neighborhoods_all,
        params.choropleth_on,
    );
    if *last == Some(key) {
        return;
    }
    *last = Some(key);
    for e in vis.iter().chain(labels.iter()) {
        commands.entity(e).despawn();
    }
    if !params.neighborhoods_on {
        return;
    }
    let Some(sim) = sim else { return };
    if sim.neighborhoods.is_empty() {
        return;
    }
    let show = |st: &NeighborhoodStat| params.neighborhoods_all || st.borough == "Manhattan";
    // The camera-density choropleth (fill + per-neighborhood labels) is the opt-in
    // "heatmap". Bare Neighborhoods mode draws only the hoverable boundaries — cheap
    // citywide, where the fills + ~312 labels (and their O(n²) declutter) were the cost.
    // Only compute the contrast-stretch range when the heatmap is actually on.
    let range = params
        .choropleth_on
        .then(|| density_range(&sim, &live, params.neighborhoods_all));

    for (i, st) in sim.neighborhoods.iter().enumerate() {
        if !show(st) {
            continue;
        }
        // Boundary outline: a zinc cell line above streets so the neighborhood reads and
        // can be hovered. Always drawn in Neighborhoods mode (the hover browser).
        let mut ring: Vec<Enu> = st.exterior.iter().map(|p| Enu::new(p[0], p[1])).collect();
        if let Some(first) = st.exterior.first() {
            ring.push(Enu::new(first[0], first[1]));
        }
        commands.spawn((
            Mesh2d(meshes.add(world::line_strip_mesh(&ring, 0.55))),
            MeshMaterial2d(materials.add(theme::map::ca(0x3f, 0x3f, 0x46, 0.7))),
            Transform::from_xyz(0.0, 0.0, 0.55),
            NeighborhoodVis,
        ));

        // Everything below — the density fill + the count labels — is the heatmap layer.
        let Some((lo, hi)) = range else { continue };
        let m = live.get(i);
        // Contrast-stretch across the visible spread (not density/peak), so the bulk of
        // Manhattan's uniformly-high neighborhoods fan out over the full ramp instead of
        // bunching in the warm end.
        let t = choropleth_t(live_density(st, m), lo, hi);
        if let Some(mesh) = world::filled_polygon_mesh(&st.exterior, -0.2) {
            commands.spawn((
                Mesh2d(meshes.add(mesh)),
                MeshMaterial2d(materials.add(choropleth_color(t))),
                Transform::from_xyz(0.0, 0.0, -0.2),
                NeighborhoodVis,
                NeighborhoodId(i),
            ));
        }
        // Name (paper-white) over two colored count rows: fixed cameras in cool steel (the
        // installed grid) and the live mobile sensors in hazard yellow. Backed by a dark
        // translucent plate (the first child) so all three read over the warm choropleth —
        // ink-on-paper would vanish on the dense rose cells. Kept a constant screen size by
        // `size_neighborhood_labels`; the plate is sized to the text by
        // `size_neighborhood_label_plates`. Only the mobile row carries the `NeighborhoodId`
        // tag, so the sampler relabels it (the fixed row is static).
        commands
            .spawn((
                Text2d::new(format!("{}\n", st.name)),
                TextFont {
                    font_size: 40.0,
                    ..default()
                },
                TextColor(theme::map::ca(0xfa, 0xfa, 0xfa, 0.98)), // paper-white on the plate
                TextLayout::new_with_justify(Justify::Center),
                bevy::sprite::Anchor::CENTER,
                Transform::from_translation(world::to_world(st.centroid, 5.5)),
                NeighborhoodLabel,
                NeighborhoodId(i), // parent carries the id too, for the screen-space declutter
            ))
            .with_child((
                TextSpan::new(format!("{} fixed\n", st.total)),
                TextFont {
                    font_size: 30.0,
                    ..default()
                },
                TextColor(theme::map::ca(0x7d, 0x97, 0xb8, 0.95)), // steel
            ))
            .with_child((
                TextSpan::new(format!("{} mobile", m.total())),
                TextFont {
                    font: bold.clone(),
                    font_size: 32.0,
                    ..default()
                },
                TextColor(theme::map::ca(0xfa, 0xcc, 0x15, 1.0)), // hazard yellow (bold, on the dark plate)
                NeighborhoodId(i),
            ))
            .with_child((
                // Backing plate: a unit quad scaled to the text bounds each frame, parked
                // just behind the glyphs (local −z) and inheriting the label's position,
                // counter-scale, and decluttered visibility. Trails the text spans so the
                // text reader's child walk sees the two `TextSpan`s first.
                Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
                MeshMaterial2d(materials.add(theme::map::ca(0x18, 0x18, 0x1b, 0.6))),
                Transform::from_xyz(0.0, 0.0, -0.3).with_scale(Vec3::new(200.0, 130.0, 1.0)),
                NeighborhoodLabelPlate,
            ));
    }
}

/// Throttled live sample: count the active mobile sensors inside each neighborhood
/// (respecting the per-class toggles) and update the choropleth colors + count
/// labels in place, so the estimates breathe with the day's real traffic.
#[allow(clippy::type_complexity)]
fn sample_neighborhood_mobile(
    time: Res<Time>,
    params: Res<Params>,
    sim: Option<Res<Sim>>,
    mut live: ResMut<NeighborhoodLive>,
    mut acc: Local<f32>,
    agents: Query<&agents::MobileAgent>,
    fills: Query<(&NeighborhoodId, &MeshMaterial2d<ColorMaterial>), Without<TextSpan>>,
    mut spans: Query<(&NeighborhoodId, &mut TextSpan)>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if !params.neighborhoods_on {
        return;
    }
    let Some(sim) = sim else { return };
    if sim.neighborhoods.is_empty() {
        return;
    }
    // Recount on a throttle — the point-in-polygon sweep is the expensive part.
    // The first time (or after the visible set resizes) we sample at once so numbers
    // appear the instant the layer turns on; then a few times a second after that.
    let first = live.by_nbhd.len() != sim.neighborhoods.len();
    *acc += time.delta_secs();
    if first || *acc >= 0.5 {
        *acc = 0.0;
        // Tally active agents into their neighborhood (bbox-prefiltered point-in-poly).
        let mut counts = vec![MobileCount::default(); sim.neighborhoods.len()];
        for a in &agents {
            if !a.active {
                continue;
            }
            use agents::AgentClass::*;
            // Honor the per-class toggles so the count matches the enabled layers.
            let on = match a.class {
                Vehicle => params.dashcam_on,
                Bus => params.ace_on,
                Tesla => params.tesla_on,
                DeliveryRobot => params.robots_on,
                Pedestrian => params.glasses_on,
            };
            if !on {
                continue;
            }
            let p = a.route.position_at(a.progress_m);
            if let Some(i) = sim.neighborhoods.iter().position(|st| st.contains(p)) {
                let c = &mut counts[i];
                match a.class {
                    Vehicle => c.rideshare += 1,
                    Bus => c.bus += 1,
                    Tesla => c.tesla += 1,
                    DeliveryRobot => c.robot += 1,
                    Pedestrian => c.glasses += 1,
                }
            }
        }
        live.by_nbhd = counts;
    }

    // The bare hover-browser has no fills/labels to repaint — the live counts above
    // still feed the hover panel, so skip the per-frame recolor/relabel entirely.
    if !params.choropleth_on {
        return;
    }

    // Repaint + relabel every frame from the cached counts (cheap; ~40 entities), so
    // labels freshly (re)spawned by `rebuild_neighborhoods` reflect the latest sample.
    let (lo, hi) = density_range(&sim, &live, params.neighborhoods_all);
    for (id, mat) in &fills {
        let st = &sim.neighborhoods[id.0];
        let t = choropleth_t(live_density(st, live.get(id.0)), lo, hi);
        if let Some(m) = materials.get_mut(&mat.0) {
            m.color = choropleth_color(t);
        }
    }
    for (id, mut span) in &mut spans {
        span.0 = format!("{} mobile", live.get(id.0).total());
    }
}

/// Keep neighborhood labels a roughly constant on-screen size as the user zooms
/// (world-space text otherwise grows/shrinks with zoom). Cheap (≤ ~40 labels).
fn size_neighborhood_labels(
    params: Res<Params>,
    cam: Query<&Transform, With<Camera2d>>,
    mut labels: Query<&mut Transform, (With<NeighborhoodLabel>, Without<Camera2d>)>,
) {
    if !params.neighborhoods_on {
        return;
    }
    let Ok(cam_t) = cam.single() else { return };
    let scale = (13.0 * cam_t.scale.x / 40.0).max(f32::EPSILON); // ~13 CSS px tall
    for mut tf in &mut labels {
        if (tf.scale.x - scale).abs() > f32::EPSILON {
            tf.scale = Vec3::splat(scale);
        }
    }
}

/// Size each neighborhood label's backing plate (a child quad) to the laid-out text bounds
/// plus a small margin. The plate is a unit `Rectangle`, so its local scale *is* its size in
/// the label's font-pixel space; the parent's counter-scale then renders it at a constant
/// on-screen size, in lockstep with the text. Reads `TextLayoutInfo` (populated once the text
/// pipeline lays the glyphs out), so the plate snaps to the real width — including when the
/// live mobile count grows a digit. Writes only on change.
fn size_neighborhood_label_plates(
    params: Res<Params>,
    labels: Query<(&bevy::text::TextLayoutInfo, &Children), With<NeighborhoodLabel>>,
    mut plates: Query<&mut Transform, With<NeighborhoodLabelPlate>>,
) {
    if !params.neighborhoods_on {
        return;
    }
    for (info, children) in &labels {
        if info.size.x <= 0.0 {
            continue; // glyphs not laid out yet
        }
        let want = Vec3::new(info.size.x + 34.0, info.size.y + 22.0, 1.0); // margin in font px
        for &child in children {
            if let Ok(mut tf) = plates.get_mut(child) {
                if tf.scale != want {
                    tf.scale = want;
                }
            }
        }
    }
}

/// De-clutter the neighborhood labels the way landmarks de-clutter their names: where a
/// landmark floats its name offshore with a leader, neighborhoods are too many to float, so
/// the lower-priority label of any overlapping pair simply yields (hidden) and the larger
/// region keeps its label. Collisions are tested in screen pixels — the labels are
/// counter-scaled to a fixed on-screen size, so as you zoom out more labels collide and drop,
/// and zooming in lets them reappear. Cheap (≤ ~40 Manhattan labels; O(n²) AABB tests).
#[allow(clippy::type_complexity)]
fn declutter_neighborhood_labels(
    params: Res<Params>,
    sim: Option<Res<Sim>>,
    cam: Query<&Transform, With<Camera2d>>,
    mut labels: Query<
        (Entity, &Transform, &NeighborhoodId, &mut Visibility),
        (With<NeighborhoodLabel>, Without<Camera2d>),
    >,
) {
    if !params.neighborhoods_on {
        return;
    }
    let Some(sim) = sim else { return };
    let Ok(cam_t) = cam.single() else { return };
    let mpp = cam_t.scale.x.max(f32::EPSILON); // world metres per screen pixel

    // Past a far citywide zoom the labels are illegible anyway — hide them all and skip the
    // O(n²) de-collision so panning the whole city (312 labels) stays smooth; they reappear
    // as you zoom into a borough.
    if mpp > NEIGHBORHOOD_LABEL_MAX_MPP {
        for (_, _, _, mut vis) in &mut labels {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
        }
        return;
    }

    // Each label's screen box, centred on its centroid. The name line is counter-scaled to
    // ~13 px tall (see `size_neighborhood_labels`); the two count rows add ~21 px below it.
    // Width is generous enough to cover the widest of the name / "NNN mobile" count rows.
    let mut items: Vec<(Entity, Vec2, f32, f32, f64)> = Vec::new();
    for (e, tf, id, _) in labels.iter() {
        let Some(st) = sim.neighborhoods.get(id.0) else { continue };
        let w_px = (st.name.chars().count() as f32 * 7.2).max(70.0);
        let (hw, hh) = ((w_px * 0.5 + 2.0) * mpp, (34.0 * 0.5 + 2.0) * mpp);
        items.push((e, tf.translation.truncate(), hw, hh, st.area_km2));
    }
    // Greedy: keep the larger region's label, drop later labels that overlap a kept one.
    items.sort_by(|a, b| b.4.total_cmp(&a.4)); // larger area first = higher priority
    let boxes: Vec<(Vec2, f32, f32)> = items.iter().map(|&(_, c, hw, hh, _)| (c, hw, hh)).collect();
    let keep = declutter_keep(&boxes);
    let hide: std::collections::HashSet<Entity> = items
        .iter()
        .zip(&keep)
        .filter(|(_, &k)| !k)
        .map(|(it, _)| it.0)
        .collect();
    for (e, _, _, mut vis) in &mut labels {
        let want = if hide.contains(&e) { Visibility::Hidden } else { Visibility::Inherited };
        if *vis != want {
            *vis = want;
        }
    }
}

/// Greedy screen-space label de-collision: walk the boxes in priority order (the caller
/// pre-sorts), keep each one that doesn't overlap an already-kept box, and return a
/// keep-flag per input box. Pure (AABB overlap), so the policy is unit-testable apart from
/// the ECS plumbing.
fn declutter_keep(boxes: &[(Vec2, f32, f32)]) -> Vec<bool> {
    let mut kept: Vec<(Vec2, f32, f32)> = Vec::with_capacity(boxes.len());
    let mut flags = Vec::with_capacity(boxes.len());
    for &(c, hw, hh) in boxes {
        let clash = kept
            .iter()
            .any(|(kc, khw, khh)| (c.x - kc.x).abs() < hw + khw && (c.y - kc.y).abs() < hh + khh);
        flags.push(!clash);
        if !clash {
            kept.push((c, hw, hh));
        }
    }
    flags
}

/// Highlight the neighborhood under the cursor (whichever `pick_neighborhood` resolved):
/// a faint wash to lift the region plus a bold ink ring so it reads regardless of how light
/// its choropleth fill is. Its own overlay entity, rebuilt only when the hovered region
/// changes, so it never fights the per-frame choropleth recolor.
fn highlight_neighborhood(
    pick: Res<NeighborhoodPick>,
    sim: Option<Res<Sim>>,
    mut last: Local<Option<usize>>,
    existing: Query<Entity, With<NeighborhoodHighlight>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if *last == pick.0 {
        return; // hovered region unchanged
    }
    *last = pick.0;
    for e in &existing {
        commands.entity(e).despawn();
    }
    let (Some(sim), Some(i)) = (sim, pick.0) else { return };
    let Some(st) = sim.neighborhoods.get(i) else { return };
    // Faint wash (above the streets, below the cameras) lifts the whole region.
    if let Some(mesh) = world::filled_polygon_mesh(&st.exterior, 0.30) {
        commands.spawn((
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(materials.add(theme::map::ca(0x18, 0x18, 0x1b, 0.10))),
            Transform::from_xyz(0.0, 0.0, 0.30),
            NeighborhoodHighlight,
        ));
    }
    // Bold ink ring traces the boundary so the hover reads on any fill lightness.
    if let Some(mesh) = world::stroke_ring_mesh(&st.exterior, 2.4, 0.62) {
        commands.spawn((
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(materials.add(theme::map::ca(0x18, 0x18, 0x1b, 0.95))),
            Transform::from_xyz(0.0, 0.0, 0.62),
            NeighborhoodHighlight,
        ));
    }
}

/// Keep landmark labels a roughly constant on-screen size as the user zooms (same
/// counter-scale as the neighborhood labels). Cheap (10 labels).
fn size_landmark_labels(
    params: Res<Params>,
    cam: Query<&Transform, With<Camera2d>>,
    mut labels: Query<&mut Transform, (With<LandmarkLabel>, Without<Camera2d>)>,
) {
    if !params.landmarks_on {
        return;
    }
    let Ok(cam_t) = cam.single() else { return };
    let scale = (12.0 * cam_t.scale.x / 38.0).max(f32::EPSILON); // ~12 CSS px tall
    for mut tf in &mut labels {
        if (tf.scale.x - scale).abs() > f32::EPSILON {
            tf.scale = Vec3::splat(scale);
        }
    }
}

/// Hover-pick the neighborhood under the cursor → `NeighborhoodPick` (panel breakdown).
fn pick_neighborhood(
    params: Res<Params>,
    wants: Res<EguiWants>,
    drag: Res<DragState>,
    sim: Option<Res<Sim>>,
    cam_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut pick: ResMut<NeighborhoodPick>,
) {
    if !params.neighborhoods_on {
        if pick.0.is_some() {
            pick.0 = None;
        }
        return;
    }
    let Some(sim) = sim else { return };
    if wants.pointer {
        return; // cursor over the control panel
    }
    let Some(cursor) = drag.last_cursor else { return };
    let Ok((cam, cam_t)) = cam_q.single() else { return };
    let Ok(world) = cam.viewport_to_world_2d(cam_t, cursor) else { return };
    let p = Enu::new(world.x as f64, world.y as f64);
    let show = |st: &NeighborhoodStat| params.neighborhoods_all || st.borough == "Manhattan";
    pick.0 = sim
        .neighborhoods
        .iter()
        .position(|st| show(st) && st.contains(p));
}

/// Seconds for the Institutions left panel to fully slide in / out.
const INSTITUTIONS_FADE_S: f32 = 0.28;

/// Drive the Institutions explore view: ease the left panel's slide-in `t`, yield to
/// the other takeover views (Operators / coverage), and forget the selection once the
/// panel has fully closed.
fn institutions_tick(
    time: Res<Time>,
    ov: Res<OperatorsView>,
    cov: Res<coverage::CoverageView>,
    mut inst: ResMut<InstitutionsView>,
    mut sel: ResMut<SelectedFacility>,
) {
    if inst.active && (ov.active || cov.active) {
        inst.active = false; // a full-screen takeover opened — leave Institutions
    }
    let target = if inst.active { 1.0 } else { 0.0 };
    let step = time.delta_secs() / INSTITUTIONS_FADE_S;
    if inst.t < target {
        inst.t = (inst.t + step).min(target);
    } else if inst.t > target {
        inst.t = (inst.t - step).max(target);
    }
    if !inst.active && inst.t <= 0.0 {
        sel.0 = None;
    }
}

/// Show/hide institution markers with the view + the per-class filters.
fn facility_markers_visibility(
    inst: Res<InstitutionsView>,
    mut markers: Query<(&FacilityMarker, &mut Visibility)>,
) {
    use sim_core::assets::FacilityKind;
    let on = inst.active || inst.t > 0.01;
    for (m, mut vis) in &mut markers {
        let show = on
            && match m.kind {
                FacilityKind::School => inst.show_schools,
                FacilityKind::Library => inst.show_libraries,
            };
        *vis = if show { Visibility::Visible } else { Visibility::Hidden };
    }
}

/// Move the highlight ring onto the selected institution (or hide it).
fn facility_highlight_follow(
    inst: Res<InstitutionsView>,
    sel: Res<SelectedFacility>,
    dir: Res<FacilityDirectory>,
    hl: Option<Res<FacilityHighlightEntity>>,
    mut q: Query<(&mut Transform, &mut Visibility), With<FacilityHighlight>>,
) {
    let Some(hl) = hl else { return };
    let Ok((mut t, mut vis)) = q.get_mut(hl.0) else { return };
    let pin = sel.0.and_then(|i| dir.pins.get(i));
    if let (true, Some(p)) = (inst.active, pin) {
        t.translation.x = p.pos.x;
        t.translation.y = p.pos.y;
        *vis = Visibility::Visible;
    } else {
        *vis = Visibility::Hidden;
    }
}

/// `OURSPACE_SHOT=1` (native, dev-only): snap into the Operators view and save a
/// screenshot, so the stacked layout can be eyeballed from a headless run.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn shot_capture(
    mut frames: Local<u32>,
    mut ov: ResMut<OperatorsView>,
    mut params: ResMut<Params>,
    mut clock: ResMut<SimClock>,
    sim: Option<Res<Sim>>,
    city: Option<Res<CityScope>>,
    mut cam: Query<&mut Transform, With<Camera2d>>,
    mut cov_view: ResMut<coverage::CoverageView>,
    (mut inst, mut fac_sel, fac_dir): (
        ResMut<InstitutionsView>,
        ResMut<SelectedFacility>,
        Res<FacilityDirectory>,
    ),
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if std::env::var("OURSPACE_SHOT").is_err() || sim.is_none() {
        return;
    }
    // Institutions explore view: open the left ranking panel, select the most-watched
    // institution (highlight + fly), and grab a frame. `OURSPACE_INSTITUTIONS=1`.
    if std::env::var("OURSPACE_INSTITUTIONS").is_ok() {
        *frames += 1;
        if *frames == 3 {
            clock.playing = false;
            inst.active = true;
            if let Ok(mut t) = cam.single_mut() {
                t.scale = Vec3::splat(5.0);
                t.translation = Vec3::new(-200.0, 2200.0, t.translation.z);
            }
        }
        if *frames == 24 {
            fac_sel.0 = fac_dir.ranked.first().copied(); // most-watched institution
        }
        if *frames == 40 {
            let shot = |commands: &mut Commands, path: &'static str| {
                commands
                    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(path));
            };
            shot(&mut commands, "/tmp/ourspace_institutions.png");
        }
        if *frames == 48 {
            exit.write(AppExit::Success);
        }
        return;
    }
    let shot = |commands: &mut Commands, path: &'static str| {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
    };
    // Operators-view morph: open the stack at full motion (no reduced_motion) and grab
    // frames spanning the texture→chip dissolve. `OURSPACE_MORPH=1`.
    if std::env::var("OURSPACE_MORPH").is_ok() {
        *frames += 1;
        if *frames == 6 {
            clock.playing = false;
            ov.active = true; // animate the flight (reduced_motion stays false)
        }
        if *frames == 26 {
            shot(&mut commands, "/tmp/ourspace_morph_a.png");
        }
        if *frames == 34 {
            shot(&mut commands, "/tmp/ourspace_morph_b.png");
        }
        if *frames == 42 {
            shot(&mut commands, "/tmp/ourspace_morph_c.png");
        }
        if *frames == 90 {
            shot(&mut commands, "/tmp/ourspace_morph_settled.png");
        }
        if *frames == 100 {
            exit.write(AppExit::Success);
        }
        return;
    }
    // Roving-coverage capture: launch the overlay and let the (sped-up) day replay,
    // grabbing a mid-day and an end-of-day frame. `OURSPACE_COVERAGE=1`.
    if std::env::var("OURSPACE_COVERAGE").is_ok() {
        *frames += 1;
        if *frames == 3 {
            cov_view.active = true; // coverage_drive frames the view + starts the fast day
        }
        if *frames >= 4 {
            clock.rate = 6000.0; // ~14 s/day so the replay fits the shot budget
        }
        if *frames == 165 {
            shot(&mut commands, "/tmp/ourspace_coverage_mid.png");
        }
        // Whole-island framing for the final frame so the shoreline highways
        // (FDR east, West Side Hwy / Henry Hudson west) read as continuous ribbons.
        if *frames == 610 {
            if let Ok(mut t) = cam.single_mut() {
                t.scale = Vec3::splat(13.0);
                t.translation = Vec3::new(-500.0, 600.0, t.translation.z);
            }
        }
        if *frames == 620 {
            shot(&mut commands, "/tmp/ourspace_coverage_full.png");
        }
        if *frames == 680 {
            exit.write(AppExit::Success);
        }
        return;
    }
    // Citywide build: a single five-borough overview shot (the Manhattan phases
    // below assume the island framing), then exit.
    if city.map(|c| c.citywide).unwrap_or(false) {
        *frames += 1;
        if *frames == 2 {
            clock.time_of_day = 14.5;
            clock.playing = false;
            params.mode = Mode::Neighborhoods;
            params.neighborhoods_all = true;
            if let Ok(mut t) = cam.single_mut() {
                t.scale = Vec3::splat(CITY_INIT_SCALE);
                t.translation = Vec3::new(CITY_INIT_CENTER.x, CITY_INIT_CENTER.y, t.translation.z);
            }
        }
        if *frames == 12 {
            // Overview: zoomed out past the footprint floor → no footprints loaded.
            shot(&mut commands, "/tmp/ourspace_city.png");
        }
        // Second citywide frame: a closer Lower-Manhattan / East-River zoom (the
        // bridges read as flat decks connecting the grids here).
        if *frames == 16 {
            params.mode = Mode::Walkshed;
            if let Ok(mut t) = cam.single_mut() {
                t.scale = Vec3::splat(2.2);
                t.translation = Vec3::new(1600.0, -13800.0, t.translation.z);
            }
        }
        if *frames == 95 {
            shot(&mut commands, "/tmp/ourspace_city_zoom.png");
        }
        if *frames == 103 {
            exit.write(AppExit::Success);
        }
        return;
    }
    *frames += 1;
    // Pin a stable mid-afternoon so the time bar shows the sun cleanly in shots,
    // then flip to night for a moon shot at the end.
    if *frames == 2 {
        clock.time_of_day = 14.5;
        clock.playing = false;
    }
    if *frames == 68 {
        ov.active = false; // leave the Operators view so the time bar is visible
    }
    if *frames == 70 {
        clock.time_of_day = 1.5; // deep night → the moon (crescent)
    }
    if *frames == 76 {
        shot(&mut commands, "/tmp/ourspace_timebar_night.png");
    }
    // Phase 1: the neighborhood layer over a wide Manhattan view.
    if *frames == 12 {
        params.mode = Mode::Neighborhoods; // the layer flag follows the mode (set in ui_panel)
        if let Ok(mut t) = cam.single_mut() {
            t.scale = Vec3::splat(14.0);
            t.translation = Vec3::new(0.0, -2000.0, t.translation.z);
        }
    }
    if *frames == 24 {
        shot(&mut commands, "/tmp/ourspace_neighborhoods.png");
    }
    // Phase 2: zoom into a dense area for the on-map branded wordmark markers.
    if *frames == 28 {
        params.mode = Mode::Walkshed; // leave neighborhood mode (clears the layer flag)
        if let Ok(mut t) = cam.single_mut() {
            t.scale = Vec3::splat(0.8);
            t.translation = Vec3::new(0.0, 0.0, t.translation.z);
        }
    }
    if *frames == 38 {
        shot(&mut commands, "/tmp/ourspace_map.png");
    }
    // Phase 3: snap into the stacked Operators view (agents have ramped by now).
    if *frames == 52 {
        ov.reduced_motion = true;
        ov.active = true;
    }
    if *frames == 64 {
        shot(&mut commands, "/tmp/ourspace_operators.png");
    }
    if *frames == 80 {
        exit.write(AppExit::Success);
    }
}

/// In `OURSPACE_SMOKE` mode, exit after a few rendered frames so headless runs
/// can confirm the render loop ticked without panicking.
fn smoke_exit(
    mut frames: Local<u32>,
    mut exit: MessageWriter<AppExit>,
    mut params: ResMut<Params>,
    mut ov: ResMut<OperatorsView>,
    mut clock: ResMut<SimClock>,
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
    if *frames == 22 {
        ov.active = true; // exercise the Operators-view enter + fixed/mobile flight
    }
    if *frames == 32 {
        ov.active = false; // and the home flight back onto the map
    }
    if *frames == 34 {
        params.mode = Mode::Neighborhoods; // exercise the neighborhood rebuild + labels (via mode)
    }
    if *frames == 38 {
        params.neighborhoods_all = true; // all-boroughs rebuild
    }
    if *frames == 40 {
        clock.rate = 8000.0; // crank the day clock to exercise wraparound + density reconcile
    }
    if *frames == 46 {
        clock.playing = false; // exercise pause (freezes clock + agents)
    }
    if *frames == 50 {
        let _ = std::fs::write("/tmp/ourspace_frames.txt", format!("frames_ok={}\n", *frames));
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod facility_tests {
    use super::*;
    use sim_core::assets::FacilityKind;

    fn pin(name: &str, cameras_near: u32, kind: FacilityKind) -> FacilityPin {
        FacilityPin {
            pos: Vec2::ZERO,
            name: name.to_string(),
            kind,
            subtype: String::new(),
            lat: 0.0,
            lon: 0.0,
            cameras_near,
        }
    }

    #[test]
    fn ranks_by_camera_count_then_name() {
        use FacilityKind::*;
        let pins = vec![
            pin("Beta School", 3, School),
            pin("Most Watched", 10, Library),
            pin("Alpha School", 3, School), // ties with Beta on count → name breaks it
            pin("Quiet Library", 0, Library),
        ];
        let ranked = rank_facilities(&pins);
        // Most cameras first; the 3-camera tie resolves alphabetically (Alpha before Beta);
        // the zero-camera entry sorts last.
        let names: Vec<&str> = ranked.iter().map(|&i| pins[i].name.as_str()).collect();
        assert_eq!(names, ["Most Watched", "Alpha School", "Beta School", "Quiet Library"]);
    }
}
