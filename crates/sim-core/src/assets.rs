//! Serde structs for the baked static assets produced by the `data-pipeline`
//! crate and loaded by the app/batch hosts. Kept Bevy-free; (de)serialize with
//! postcard for a compact, WASM-friendly binary.

use crate::exposure::SourceKind;
use crate::math::Vec2;
use crate::projection::GeoOrigin;
use serde::{Deserialize, Serialize};

/// Provenance metadata shipped with every layer so the UI can show an honest
/// "source / date / license" badge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub source: String,
    pub url: String,
    pub license: String,
    /// The data vintage / snapshot date this layer was baked from (ISO-8601).
    pub as_of: String,
    pub notes: String,
}

/// A node (intersection) position in local ENU meters.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NodePoint {
    pub x: f64,
    pub y: f64,
}

/// A walkable segment between two nodes. Bidirectional for pedestrians.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeData {
    pub from: u32,
    pub to: u32,
    pub length_m: f64,
    /// Densified polyline in ENU meters, including both endpoints. Used to
    /// reconstruct position(t) along a routed path.
    pub polyline: Vec<[f64; 2]>,
    /// Source segment id (e.g. LION / OSM way id) for heatmap aggregation.
    pub segment_id: Option<i64>,
}

/// The baked routable pedestrian graph for an area of interest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphAsset {
    pub origin: GeoOrigin,
    pub nodes: Vec<NodePoint>,
    pub edges: Vec<EdgeData>,
    pub provenance: Provenance,
}

impl GraphAsset {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// A single fixed sensor (CCTV / DOT cam) in ENU meters. Heading/FOV/range are
/// model assumptions where the source provides only a location.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FixedSensorData {
    pub x: f64,
    pub y: f64,
    /// Compass heading (deg, 0 = north) if known; `None` => model omnidirectional.
    pub heading_deg: Option<f64>,
    pub kind: SourceKind,
}

/// The baked fixed-sensor layer (e.g. Dahir NYC camera points).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixedSensorLayer {
    pub origin: GeoOrigin,
    pub sensors: Vec<FixedSensorData>,
    /// Detector recall the source counts should be corrected by (e.g. 0.63 for
    /// Dahir). `None` => no correction.
    pub recall: Option<f64>,
    pub provenance: Provenance,
}

impl FixedSensorLayer {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// Which census attested a fixed-CCTV point (for the per-camera modal provenance).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CctvSource {
    /// Amnesty *Decode Surveillance NYC* crowdsourced intersection count.
    Amnesty,
    /// Dahir et al. street-view ML detection (carries a GSV panorama + capture date).
    Dahir,
}

/// One fixed-CCTV camera with the provenance the app surfaces in the click modal.
/// Geometry mirrors [`FixedSensorData`]; Dahir-detected points additionally carry the
/// Google Street View `panoid` + capture year/month they were detected in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CctvCamera {
    pub x: f64,
    pub y: f64,
    /// GSV capture bearing (Dahir) if known; `None` => modeled omnidirectional.
    pub heading_deg: Option<f64>,
    pub source: CctvSource,
    /// Dahir Google Street View panorama id (deep-links to the exact pano).
    pub panoid: Option<String>,
    /// Dahir capture date (the pano the detection came from).
    pub year: Option<u16>,
    pub month: Option<u8>,
}

/// The baked fixed-CCTV layer — the merged Amnesty + Dahir census with per-camera
/// provenance. [`CctvCameraLayer::to_fixed_layer`] projects to the shared sensor type
/// for the exposure model (preserving the census's `recall`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CctvCameraLayer {
    pub origin: GeoOrigin,
    pub cameras: Vec<CctvCamera>,
    pub recall: Option<f64>,
    pub provenance: Provenance,
}

impl CctvCameraLayer {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
    pub fn to_fixed_layer(&self) -> FixedSensorLayer {
        FixedSensorLayer {
            origin: self.origin,
            sensors: self
                .cameras
                .iter()
                .map(|c| FixedSensorData {
                    x: c.x,
                    y: c.y,
                    heading_deg: c.heading_deg,
                    kind: SourceKind::FixedCctv,
                })
                .collect(),
            recall: self.recall,
            provenance: self.provenance.clone(),
        }
    }
}

/// One DeFlock/OSM ALPR reader, carrying the crowdsourced metadata the app surfaces
/// in the per-camera modal + the stack-by-operator stratification. Geometry mirrors
/// [`FixedSensorData`]; the extra fields are the OSM node id (deep-links to
/// openstreetmap.org / deflock.me) and the maker/operator strings where mapped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlprReader {
    pub x: f64,
    pub y: f64,
    /// Compass heading (deg, 0 = north) the reader faces, if mapped (`None` => unknown).
    pub heading_deg: Option<f64>,
    /// OpenStreetMap node id (the DeFlock sync target).
    pub osm_id: u64,
    /// Manufacturer/model (e.g. "Flock Safety", "Leonardo", "Mav (IQ:350XR)").
    pub manufacturer: Option<String>,
    /// Operating agency (e.g. "NYPD", "NYC M.T.A").
    pub operator: Option<String>,
}

/// The baked ALPR layer — DeFlock readers with metadata. Richer than a bare
/// [`FixedSensorLayer`] so the app can show per-camera provenance and group by maker;
/// [`AlprReaderLayer::to_fixed_layer`] projects back to the shared sensor type for the
/// exposure model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlprReaderLayer {
    pub origin: GeoOrigin,
    pub readers: Vec<AlprReader>,
    pub provenance: Provenance,
}

impl AlprReaderLayer {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
    /// Project to the shared fixed-sensor layer for the exposure pipeline (kind = Alpr,
    /// mapped points so `recall = None`). The metadata stays on the [`AlprReader`]s.
    pub fn to_fixed_layer(&self) -> FixedSensorLayer {
        FixedSensorLayer {
            origin: self.origin,
            sensors: self
                .readers
                .iter()
                .map(|r| FixedSensorData {
                    x: r.x,
                    y: r.y,
                    heading_deg: r.heading_deg,
                    kind: SourceKind::Alpr,
                })
                .collect(),
            recall: None,
            provenance: self.provenance.clone(),
        }
    }
}

/// One LinkNYC kiosk (Wi-Fi/phone hub) in ENU meters. Deliberately *not* a
/// `FixedSensorData`: a kiosk isn't an always-on camera — it surveils only when you
/// connect to its Wi-Fi — so it carries no FOV and stays out of the exposure model.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LinkNycKiosk {
    pub x: f64,
    pub y: f64,
    /// Wi-Fi is currently live (vs. installed / under repair).
    pub wifi_live: bool,
}

/// The baked LinkNYC kiosk layer — a fixed map layer of Wi-Fi hubs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkNycLayer {
    pub origin: GeoOrigin,
    pub kiosks: Vec<LinkNycKiosk>,
    pub provenance: Provenance,
}

impl LinkNycLayer {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// One ACE route shape as an ordered ENU polyline — the path a bus drives. Used
/// to animate running buses (the `segments` soup below stays for the analytical
/// curb-distance exposure model). `f32` keeps the bundle small; decorative.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcePolyline {
    /// Route short-name (e.g. "M15-SBS").
    pub route: String,
    pub points: Vec<[f32; 2]>,
}

/// Baked ACE bus-camera corridors: the line segments enforced buses traverse,
/// in ENU meters. A walker within the configured curb reach of any segment can
/// be captured by a passing bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AceCorridorLayer {
    pub origin: GeoOrigin,
    /// `[[x0,y0],[x1,y1]]` ENU segments (drives the analytical exposure model).
    pub segments: Vec<[[f64; 2]; 2]>,
    /// Per-route ordered polylines (drives the animated running buses).
    #[serde(default)]
    pub polylines: Vec<AcePolyline>,
    /// ACE route short-names included (for provenance/UI).
    pub routes: Vec<String>,
    pub provenance: Provenance,
}

impl AceCorridorLayer {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// A citywide exposure heatmap: per-class intensity per graph edge, in the
/// **same order** as the `GraphAsset.edges` it was computed from. Each value is
/// the expected number of devices that would capture you per minute of presence
/// on that segment (at `reference_hour`). Classes are kept separate so a uniform
/// field (dashcams) doesn't wash out the spatial signal of fixed cameras / ACE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatmapLayer {
    pub reference_hour: f64,
    pub fixed: Vec<f64>,
    pub ace: Vec<f64>,
    pub dashcam: Vec<f64>,
    pub glasses: Vec<f64>,
    pub provenance: Provenance,
}

impl HeatmapLayer {
    pub fn len(&self) -> usize {
        self.fixed.len()
    }
    pub fn is_empty(&self) -> bool {
        self.fixed.is_empty()
    }
    /// Total expected devices/min for edge `i` across all classes.
    pub fn total(&self, i: usize) -> f64 {
        self.fixed[i] + self.ace[i] + self.dashcam[i] + self.glasses[i]
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// One census block group: its boundary (ENU exterior ring), Shannon diversity
/// entropy, population, and detected-camera count — for the Dahir equity overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockGroup {
    pub geoid: String,
    /// Exterior boundary ring in ENU meters (no holes; block groups rarely have any).
    pub exterior: Vec<[f64; 2]>,
    /// Shannon entropy over white/Black/Asian/Hispanic/other (0 = homogeneous).
    pub entropy: f64,
    pub population: u32,
    /// Detected fixed cameras whose point falls in this block group.
    pub camera_count: u32,
}

/// The block-group equity overlay (diversity vs. camera density), mirroring
/// Dahir et al. Aggregated at block-group level only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquityLayer {
    pub origin: GeoOrigin,
    pub block_groups: Vec<BlockGroup>,
    pub provenance: Provenance,
}

impl EquityLayer {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// One NYC neighborhood (Pedia Cities boundaries): name, borough, and boundary
/// ring in ENU meters. Camera aggregation is done at runtime by the app (it holds
/// every fixed sensor + an R-tree in memory), so the baked layer is pure geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neighborhood {
    pub name: String,
    pub borough: String,
    /// Exterior boundary ring in ENU meters (exterior only; these have no holes).
    pub exterior: Vec<[f64; 2]>,
    /// `[min_x, min_y, max_x, max_y]` ENU bounds for a fast point prefilter.
    pub bbox: [f64; 4],
}

impl Neighborhood {
    /// True if ENU point `p` lies inside this neighborhood (bbox-prefiltered).
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.bbox[0]
            && p.x <= self.bbox[2]
            && p.y >= self.bbox[1]
            && p.y <= self.bbox[3]
            && point_in_ring(p, &self.exterior)
    }

    /// Polygon area in m² (shoelace over the ENU ring).
    pub fn area_m2(&self) -> f64 {
        let r = &self.exterior;
        let n = r.len();
        if n < 3 {
            return 0.0;
        }
        let mut s = 0.0;
        let mut j = n - 1;
        for i in 0..n {
            s += (r[j][0] + r[i][0]) * (r[j][1] - r[i][1]);
            j = i;
        }
        (s * 0.5).abs()
    }

    /// Area-weighted polygon centroid in ENU meters (for placing the name label);
    /// falls back to the vertex mean if the ring is degenerate.
    pub fn centroid(&self) -> Vec2 {
        let r = &self.exterior;
        let n = r.len();
        if n == 0 {
            return Vec2::new(0.0, 0.0);
        }
        let (mut a, mut cx, mut cy) = (0.0, 0.0, 0.0);
        let mut j = n - 1;
        for i in 0..n {
            let cross = r[j][0] * r[i][1] - r[i][0] * r[j][1];
            a += cross;
            cx += (r[j][0] + r[i][0]) * cross;
            cy += (r[j][1] + r[i][1]) * cross;
            j = i;
        }
        if a.abs() < 1e-9 {
            let mx = r.iter().map(|p| p[0]).sum::<f64>() / n as f64;
            let my = r.iter().map(|p| p[1]).sum::<f64>() / n as f64;
            return Vec2::new(mx, my);
        }
        Vec2::new(cx / (3.0 * a), cy / (3.0 * a))
    }
}

/// The NYC neighborhood layer (Pedia Cities boundaries, all five boroughs).
/// Geometry only — the app aggregates fixed-camera counts per neighborhood at
/// runtime and renders Manhattan by default (other boroughs are toggleable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeighborhoodLayer {
    pub origin: GeoOrigin,
    pub neighborhoods: Vec<Neighborhood>,
    pub provenance: Provenance,
}

impl NeighborhoodLayer {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// A borough boundary as closed exterior rings in ENU meters — geometry only,
/// rendered as an unfilled outline that frames the street network (a visual
/// polish layer, not used by the model). Baked from the NYC borough-boundary
/// dataset; only the borough's main landmass part(s) are kept.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoroughOutline {
    pub origin: GeoOrigin,
    /// Borough name (e.g. `"Manhattan"`).
    pub name: String,
    /// One or more closed exterior rings in ENU meters (largest landmass first).
    pub rings: Vec<Vec<[f64; 2]>>,
    pub provenance: Provenance,
}

impl BoroughOutline {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// Manhattan building footprints — flat exterior rings in ENU meters, rendered as
/// a subtle ground fabric under the street network (context/polish; not part of the
/// exposure model). Clipped to the borough; geometry only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildingFootprints {
    pub origin: GeoOrigin,
    /// Exterior rings in ENU meters (f32 — ample precision at city scale).
    pub polygons: Vec<Vec<[f32; 2]>>,
    pub provenance: Provenance,
}

impl BuildingFootprints {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// One planar LoD2 surface of a landmark building: a polygon ring of ENU points
/// carrying height-above-base in meters (so the renderer can oblique-project it).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandmarkSurface {
    /// 0 = wall, 1 = roof. (Ground surfaces are dropped — hidden under everything.)
    pub kind: u8,
    /// Ring vertices as ENU `[x, y, height_above_base_m]`.
    pub verts: Vec<[f32; 3]>,
}

/// A notable building rendered as recognizable 2.5D massing (oblique-projected in
/// the top-down view) to help orient users — its true LoD2 shape from the NYC 3D
/// Building Model (Empire State's spire, Chrysler's crown, Columbia's dome).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Landmark {
    pub name: String,
    /// Footprint centroid in ENU meters (label anchor + scene depth sort).
    pub anchor: [f32; 2],
    /// Peak height above base, meters (for the label + scaling).
    pub height_m: f32,
    pub surfaces: Vec<LandmarkSurface>,
}

/// The landmark-massing layer: a curated handful of orienting buildings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandmarkMassing {
    pub origin: GeoOrigin,
    pub landmarks: Vec<Landmark>,
    pub provenance: Provenance,
}

impl LandmarkMassing {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// One taxi-zone polygon part carrying its zone's normalized rideshare density.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashcamZone {
    /// Exterior ring in ENU meters.
    pub exterior: Vec<[f64; 2]>,
    /// `[min_x, min_y, max_x, max_y]` ENU bounds for fast point prefilter.
    pub bbox: [f64; 4],
    /// Rideshare density relative to the median Manhattan zone (≈1.0 typical).
    pub intensity: f64,
}

/// Spatial dashcam field: rideshare (for-hire vehicle) density by taxi zone,
/// from NYC TLC High-Volume FHV trip records. Dashcams ride in these vehicles,
/// so exposure follows where Uber/Lyft actually drive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashcamFieldLayer {
    pub origin: GeoOrigin,
    pub zones: Vec<DashcamZone>,
    pub provenance: Provenance,
}

/// Ray-casting point-in-polygon on an ENU ring.
fn point_in_ring(p: Vec2, ring: &[[f64; 2]]) -> bool {
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

impl DashcamFieldLayer {
    /// Relative rideshare density at an ENU point (1.0 ≈ a typical zone). Falls
    /// back to 1.0 outside all zones so the dashcam class never silently vanishes.
    pub fn intensity_at(&self, p: Vec2) -> f64 {
        for z in &self.zones {
            if p.x >= z.bbox[0] && p.x <= z.bbox[2] && p.y >= z.bbox[1] && p.y <= z.bbox[3]
                && point_in_ring(p, &z.exterior)
            {
                return z.intensity;
            }
        }
        1.0
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// Spatial Tesla-camera field: normalized private-Tesla registration density by ZIP
/// (NYS DMV), as polygon zones. Teslas run always-on cameras (Sentry when parked,
/// Autopilot while driving), so exposure follows where Teslas are garaged/driven —
/// a residential pattern distinct from the rideshare-dashcam field. Reuses
/// [`DashcamZone`] (a generic polygon + intensity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeslaField {
    pub origin: GeoOrigin,
    pub zones: Vec<DashcamZone>,
    pub provenance: Provenance,
}

impl TeslaField {
    /// Normalized Tesla density at an ENU point (1.0 ≈ a typical Manhattan ZIP);
    /// `default` outside all zones.
    pub fn intensity_at(&self, p: Vec2, default: f64) -> f64 {
        for z in &self.zones {
            if p.x >= z.bbox[0] && p.x <= z.bbox[2] && p.y >= z.bbox[1] && p.y <= z.bbox[3]
                && point_in_ring(p, &z.exterior)
            {
                return z.intensity;
            }
        }
        default
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// Spatial robotability field: a coarse ENU grid of the NYC "Robotability Score"
/// (0..1, higher = more suitable for sidewalk robots), aggregated from the IRL-CT
/// robotability project's per-sidewalk scores. Drives where speculative sidewalk
/// delivery robots spawn (per-node weight) and how dense their exposure is along a
/// route. A grid keeps lookup O(1) and decoupled from the walk-graph topology.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobotabilityField {
    pub origin: GeoOrigin,
    /// ENU lower-left corner of the grid (meters).
    pub min_x: f64,
    pub min_y: f64,
    /// Square cell size (meters).
    pub cell_m: f64,
    pub cols: u32,
    pub rows: u32,
    /// Row-major scores in [0,1]; a negative value marks a cell with no sidewalk data.
    pub scores: Vec<f32>,
    pub provenance: Provenance,
}

impl RobotabilityField {
    /// Robotability in [0,1] at an ENU point; returns `default` off-grid or where a
    /// cell has no sidewalk data, so the class never silently divides by an empty grid.
    pub fn score_at(&self, p: Vec2, default: f32) -> f32 {
        if self.cell_m <= 0.0 || self.cols == 0 || self.rows == 0 {
            return default;
        }
        let cx = ((p.x - self.min_x) / self.cell_m).floor();
        let cy = ((p.y - self.min_y) / self.cell_m).floor();
        if cx < 0.0 || cy < 0.0 || cx as u32 >= self.cols || cy as u32 >= self.rows {
            return default;
        }
        let v = self.scores[cy as usize * self.cols as usize + cx as usize];
        if v < 0.0 {
            default
        } else {
            v
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

/// One baked taxi/vehicle route: a densified ENU polyline plus its sampling
/// weight (∝ O-D trip volume). Routed **offline** over the pedestrian walk graph
/// (v1 limitation: ignores one-way / turn restrictions — these are decorative
/// agents, not part of the citable exposure estimate). `f32` keeps the bundle
/// small; sub-meter precision is irrelevant for a moving dot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleRoute {
    pub polyline: Vec<[f32; 2]>,
    pub length_m: f32,
    /// Relative sampling weight = O-D trip volume / total (Midtown corridors
    /// carry more cars, tracking the same field the dashcam model integrates).
    pub weight: f32,
}

/// Baked pool of representative vehicle (rideshare) routes for the animated
/// dashcam agents. Sampled with replacement at runtime, weighted by `weight`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleRoutesLayer {
    pub origin: GeoOrigin,
    pub routes: Vec<VehicleRoute>,
    pub provenance: Provenance,
}

impl VehicleRoutesLayer {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

// ----------------------------------------------- real-day replay (buses) ------

/// One scheduled ACE bus trip: its shape (geometry) plus time→arc-length keyframes
/// from the real GTFS stop times. A bus exists during `[start_min, end_min)`
/// (minutes since service midnight; may exceed 1440 for after-midnight trips) and
/// its position is interpolated from the keyframes along `shape_idx`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusTrip {
    pub route_idx: u16,
    pub shape_idx: u32,
    pub start_min: f32,
    pub end_min: f32,
    /// `[time_min, arc_m]` keyframes, monotonic in both (one per served stop).
    pub keyframes: Vec<[f32; 2]>,
}

/// A day's worth of real ACE bus trips (one GTFS service date), for schedule-driven
/// replay. Shapes are de-duplicated; trips reference them by index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusDayLayer {
    pub origin: GeoOrigin,
    /// `YYYYMMDD` service date these trips run on.
    pub service_date: u32,
    /// Route short-names (for color/labels); `BusTrip.route_idx` indexes this.
    pub routes: Vec<String>,
    /// De-duplicated ENU shape polylines; `BusTrip.shape_idx` indexes this.
    pub shapes: Vec<Vec<[f32; 2]>>,
    pub trips: Vec<BusTrip>,
    pub provenance: Provenance,
}

impl BusDayLayer {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

// ----------------------------------------------- real-day replay (taxis) ------

/// One real rideshare trip: it appears at `pu_min` (minutes since midnight) and
/// drives `route_idx` over `dur_min` minutes, then vanishes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TaxiTrip {
    pub pu_min: f32,
    pub route_idx: u32,
    pub dur_min: f32,
}

/// Real per-minute origin→destination trip count (drives the minute-accurate
/// analytical flux; full, not sampled).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TaxiOdMinute {
    pub pu_min: u16,
    pub pu_zone: u16,
    pub do_zone: u16,
    pub trips: u32,
}

/// A day's worth of real rideshare trips (one date): a shared routed-O-D pool, a
/// sampled trip list (for the visuals, capped to the on-screen pool), and the full
/// per-minute O-D aggregate (for the estimate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxiDayLayer {
    pub origin: GeoOrigin,
    pub service_date: u32,
    /// Shared routed O-D polyline pool; `TaxiTrip.route_idx` indexes this.
    pub routes: Vec<VehicleRoute>,
    /// Sampled real trips, sorted ascending by `pu_min`.
    pub trips: Vec<TaxiTrip>,
    /// Full per-minute O-D counts (for the analytical flux).
    pub od_per_minute: Vec<TaxiOdMinute>,
    pub provenance: Provenance,
}

impl TaxiDayLayer {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vehicle_routes_round_trip() {
        let layer = VehicleRoutesLayer {
            origin: GeoOrigin::MANHATTAN,
            routes: vec![VehicleRoute {
                polyline: vec![[0.0, 0.0], [10.0, 5.0]],
                length_m: 11.18,
                weight: 0.5,
            }],
            provenance: Provenance {
                source: "test".into(),
                url: String::new(),
                license: String::new(),
                as_of: "2026-06-16".into(),
                notes: String::new(),
            },
        };
        let back = VehicleRoutesLayer::from_bytes(&layer.to_bytes().unwrap()).unwrap();
        assert_eq!(back.routes.len(), 1);
        assert_eq!(back.routes[0].polyline.len(), 2);
        assert!((back.routes[0].weight - 0.5).abs() < 1e-6);
    }

    #[test]
    fn dashcam_field_intensity_lookup() {
        let layer = DashcamFieldLayer {
            origin: GeoOrigin::MANHATTAN,
            zones: vec![DashcamZone {
                exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                bbox: [0.0, 0.0, 10.0, 10.0],
                intensity: 3.5,
            }],
            provenance: Provenance {
                source: String::new(),
                url: String::new(),
                license: String::new(),
                as_of: String::new(),
                notes: String::new(),
            },
        };
        assert_eq!(layer.intensity_at(Vec2::new(5.0, 5.0)), 3.5); // inside
        assert_eq!(layer.intensity_at(Vec2::new(50.0, 50.0)), 1.0); // outside -> fallback
    }

    #[test]
    fn graph_asset_round_trips_through_postcard() {
        let g = GraphAsset {
            origin: GeoOrigin::MANHATTAN,
            nodes: vec![NodePoint { x: 0.0, y: 0.0 }, NodePoint { x: 10.0, y: 0.0 }],
            edges: vec![EdgeData {
                from: 0,
                to: 1,
                length_m: 10.0,
                polyline: vec![[0.0, 0.0], [10.0, 0.0]],
                segment_id: Some(42),
            }],
            provenance: Provenance {
                source: "OSM".into(),
                url: "https://www.openstreetmap.org".into(),
                license: "ODbL".into(),
                as_of: "2026-06-14".into(),
                notes: "test".into(),
            },
        };
        let bytes = g.to_bytes().unwrap();
        let back = GraphAsset::from_bytes(&bytes).unwrap();
        assert_eq!(back.nodes.len(), 2);
        assert_eq!(back.edges[0].segment_id, Some(42));
    }

    #[test]
    fn neighborhood_round_trip_geometry() {
        let n = Neighborhood {
            name: "Test".into(),
            borough: "Manhattan".into(),
            exterior: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            bbox: [0.0, 0.0, 100.0, 100.0],
        };
        assert!(n.contains(Vec2::new(50.0, 50.0)));
        assert!(!n.contains(Vec2::new(150.0, 50.0))); // outside bbox
        assert!((n.area_m2() - 10_000.0).abs() < 1e-6);
        let c = n.centroid();
        assert!((c.x - 50.0).abs() < 1e-6 && (c.y - 50.0).abs() < 1e-6);

        let layer = NeighborhoodLayer {
            origin: GeoOrigin::MANHATTAN,
            neighborhoods: vec![n],
            provenance: Provenance {
                source: "test".into(),
                url: String::new(),
                license: String::new(),
                as_of: String::new(),
                notes: String::new(),
            },
        };
        let back = NeighborhoodLayer::from_bytes(&layer.to_bytes().unwrap()).unwrap();
        assert_eq!(back.neighborhoods.len(), 1);
        assert_eq!(back.neighborhoods[0].name, "Test");
        assert_eq!(back.neighborhoods[0].borough, "Manhattan");
    }

    #[test]
    fn bus_taxi_day_round_trip() {
        let prov = || Provenance {
            source: "t".into(),
            url: String::new(),
            license: String::new(),
            as_of: String::new(),
            notes: String::new(),
        };
        let bus = BusDayLayer {
            origin: GeoOrigin::MANHATTAN,
            service_date: 20260421,
            routes: vec!["M15-SBS".into()],
            shapes: vec![vec![[0.0, 0.0], [100.0, 0.0]]],
            trips: vec![BusTrip {
                route_idx: 0,
                shape_idx: 0,
                start_min: 480.0,
                end_min: 510.0,
                keyframes: vec![[480.0, 0.0], [510.0, 100.0]],
            }],
            provenance: prov(),
        };
        let back = BusDayLayer::from_bytes(&bus.to_bytes().unwrap()).unwrap();
        assert_eq!(back.trips.len(), 1);
        assert_eq!(back.trips[0].keyframes.len(), 2);
        assert_eq!(back.service_date, 20260421);

        let taxi = TaxiDayLayer {
            origin: GeoOrigin::MANHATTAN,
            service_date: 20260421,
            routes: vec![VehicleRoute {
                polyline: vec![[0.0, 0.0], [10.0, 0.0]],
                length_m: 10.0,
                weight: 1.0,
            }],
            trips: vec![TaxiTrip {
                pu_min: 540.0,
                route_idx: 0,
                dur_min: 12.0,
            }],
            od_per_minute: vec![TaxiOdMinute {
                pu_min: 540,
                pu_zone: 100,
                do_zone: 200,
                trips: 3,
            }],
            provenance: prov(),
        };
        let back = TaxiDayLayer::from_bytes(&taxi.to_bytes().unwrap()).unwrap();
        assert_eq!(back.trips[0].route_idx, 0);
        assert_eq!(back.od_per_minute[0].trips, 3);
    }
}
