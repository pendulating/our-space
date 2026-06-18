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
}
