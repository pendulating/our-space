//! Serde structs for the baked static assets produced by the `data-pipeline`
//! crate and loaded by the app/batch hosts. Kept Bevy-free; (de)serialize with
//! postcard for a compact, WASM-friendly binary.

use crate::exposure::SourceKind;
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

/// Baked ACE bus-camera corridors: the line segments enforced buses traverse,
/// in ENU meters. A walker within the configured curb reach of any segment can
/// be captured by a passing bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AceCorridorLayer {
    pub origin: GeoOrigin,
    /// `[[x0,y0],[x1,y1]]` ENU segments.
    pub segments: Vec<[[f64; 2]; 2]>,
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

#[cfg(test)]
mod tests {
    use super::*;

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
