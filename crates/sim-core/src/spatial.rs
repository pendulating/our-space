//! Spatial index over placed sensors for fast range queries.
//!
//! The exposure simulation tests each walker position against every sensor's
//! capture geometry. With ~31k sensors and ~1200 ticks per route, that is 37M
//! wedge tests. A sensor with a 15 m range can only fire within 15 m of the
//! walker, so an R-tree cull reduces the inner loop by 100–500×.

use rstar::{RTree, RTreeObject, AABB};

use crate::assets::DashcamZone;
use crate::math::Vec2;
use crate::simulation::SensorInstance;

/// A uniform grid over ACE corridor segments, indexed by each segment's bbox
/// expanded by `capture_range_m`. A single cell lookup returns all segments
/// that could possibly be within capture range of a query point.
pub struct AceGrid {
    cell_m: f64,
    min_x: f64,
    min_y: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<usize>>,
}

impl AceGrid {
    pub fn new(segments: &[[Vec2; 2]], capture_range_m: f64) -> Self {
        let cell_m = capture_range_m.max(1.0);
        if segments.is_empty() {
            return AceGrid { cell_m, min_x: 0.0, min_y: 0.0, nx: 1, ny: 1, cells: vec![Vec::new()] };
        }
        let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
        let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
        for [a, b] in segments {
            min_x = min_x.min(a.x.min(b.x));
            min_y = min_y.min(a.y.min(b.y));
            max_x = max_x.max(a.x.max(b.x));
            max_y = max_y.max(a.y.max(b.y));
        }
        min_x -= cell_m * 2.0;
        min_y -= cell_m * 2.0;
        max_x += cell_m * 2.0;
        max_y += cell_m * 2.0;
        let nx = (((max_x - min_x) / cell_m).ceil() as usize).max(1);
        let ny = (((max_y - min_y) / cell_m).ceil() as usize).max(1);
        let mut cells = vec![Vec::new(); nx * ny];
        let clamp_i = |x: f64| ((x - min_x) / cell_m).floor().clamp(0.0, (nx - 1) as f64) as usize;
        let clamp_j = |y: f64| ((y - min_y) / cell_m).floor().clamp(0.0, (ny - 1) as f64) as usize;
        for (si, [a, b]) in segments.iter().enumerate() {
            let i0 = clamp_i(a.x.min(b.x) - capture_range_m);
            let j0 = clamp_j(a.y.min(b.y) - capture_range_m);
            let i1 = clamp_i(a.x.max(b.x) + capture_range_m);
            let j1 = clamp_j(a.y.max(b.y) + capture_range_m);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    cells[j * nx + i].push(si);
                }
            }
        }
        AceGrid { cell_m, min_x, min_y, nx, ny, cells }
    }

    /// Indices of segments whose expanded bbox overlaps `p`'s cell.
    pub fn candidates_at(&self, p: Vec2) -> &[usize] {
        let i = ((p.x - self.min_x) / self.cell_m).floor().clamp(0.0, (self.nx - 1) as f64) as usize;
        let j = ((p.y - self.min_y) / self.cell_m).floor().clamp(0.0, (self.ny - 1) as f64) as usize;
        &self.cells[j * self.nx + i]
    }
}

struct SensorPoint {
    pos: [f64; 2],
    idx: usize,
}

impl RTreeObject for SensorPoint {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.pos)
    }
}

/// A spatial index over [`SensorInstance`] apex positions, enabling fast
/// "which sensors are within radius R of this point" queries.
pub struct SensorIndex {
    tree: RTree<SensorPoint>,
    max_range: f64,
}

impl SensorIndex {
    /// Build the index from a sensor slice. `max_range` is the largest
    /// `wedge.range_m` across all sensors (the query radius upper bound).
    pub fn new(sensors: &[SensorInstance]) -> Self {
        let max_range = sensors
            .iter()
            .map(|s| s.wedge.range_m)
            .fold(0.0_f64, f64::max);
        let points: Vec<SensorPoint> = sensors
            .iter()
            .enumerate()
            .map(|(i, s)| SensorPoint {
                pos: [s.wedge.apex.x, s.wedge.apex.y],
                idx: i,
            })
            .collect();
        SensorIndex {
            tree: RTree::bulk_load(points),
            max_range,
        }
    }

    /// The maximum sensor range in the index (the query radius needed to
    /// guarantee no sensor is missed).
    pub fn max_range(&self) -> f64 {
        self.max_range
    }

    /// Indices of sensors whose apex is within `radius` of `point`.
    /// The caller still needs to run the full wedge + occlusion test.
    pub fn candidates_within(&self, point: Vec2, radius: f64) -> Vec<usize> {
        let r2 = radius * radius;
        self.tree
            .locate_in_envelope(&AABB::from_corners(
                [point.x - radius, point.y - radius],
                [point.x + radius, point.y + radius],
            ))
            .filter(|p| {
                let dx = p.pos[0] - point.x;
                let dy = p.pos[1] - point.y;
                dx * dx + dy * dy <= r2
            })
            .map(|p| p.idx)
            .collect()
    }

    /// Iterate sensor indices within `max_range` of `point` (the full
    /// cull for the simulation inner loop).
    pub fn candidates(&self, point: Vec2) -> Vec<usize> {
        self.candidates_within(point, self.max_range)
    }
}

/// A coarse uniform grid over polygon zone bounding boxes, making
/// `intensity_at` O(1) worst-case instead of O(zones). Cell size defaults
/// to 200 m (a taxi zone is ~500–1000 m across, so each cell holds 1–4
/// candidate zones).
pub struct ZoneGrid {
    cell_m: f64,
    min_x: f64,
    min_y: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<usize>>,
}

impl ZoneGrid {
    pub fn new(zones: &[DashcamZone], cell_m: f64) -> Self {
        let cell_m = cell_m.max(1.0);
        if zones.is_empty() {
            return ZoneGrid { cell_m, min_x: 0.0, min_y: 0.0, nx: 1, ny: 1, cells: vec![Vec::new()] };
        }
        let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
        let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
        for z in zones {
            min_x = min_x.min(z.bbox[0]);
            min_y = min_y.min(z.bbox[1]);
            max_x = max_x.max(z.bbox[2]);
            max_y = max_y.max(z.bbox[3]);
        }
        min_x -= cell_m;
        min_y -= cell_m;
        max_x += cell_m;
        max_y += cell_m;
        let nx = (((max_x - min_x) / cell_m).ceil() as usize).max(1);
        let ny = (((max_y - min_y) / cell_m).ceil() as usize).max(1);
        let mut cells = vec![Vec::new(); nx * ny];
        for (zi, z) in zones.iter().enumerate() {
            let i0 = ((z.bbox[0] - min_x) / cell_m).floor().clamp(0.0, (nx - 1) as f64) as usize;
            let j0 = ((z.bbox[1] - min_y) / cell_m).floor().clamp(0.0, (ny - 1) as f64) as usize;
            let i1 = ((z.bbox[2] - min_x) / cell_m).floor().clamp(0.0, (nx - 1) as f64) as usize;
            let j1 = ((z.bbox[3] - min_y) / cell_m).floor().clamp(0.0, (ny - 1) as f64) as usize;
            for j in j0..=j1 {
                for i in i0..=i1 {
                    cells[j * nx + i].push(zi);
                }
            }
        }
        ZoneGrid { cell_m, min_x, min_y, nx, ny, cells }
    }

    /// Candidate zone indices whose bbox contains `p`'s cell.
    pub fn candidates_at(&self, p: Vec2) -> &[usize] {
        let i = ((p.x - self.min_x) / self.cell_m).floor().clamp(0.0, (self.nx - 1) as f64) as usize;
        let j = ((p.y - self.min_y) / self.cell_m).floor().clamp(0.0, (self.ny - 1) as f64) as usize;
        &self.cells[j * self.nx + i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exposure::SourceKind;
    use crate::geometry::FrustumWedge;

    fn sensor_at(x: f64, y: f64, range: f64) -> SensorInstance {
        SensorInstance {
            wedge: FrustumWedge::from_degrees(Vec2::new(x, y), None, 360.0, range),
            frame_rate: 1.0,
            id: 0,
            kind: SourceKind::FixedCctv,
            group: 0,
            confirmed: false,
            host_poly: None,
        }
    }

    #[test]
    fn culls_distant_sensors() {
        let sensors = vec![
            sensor_at(0.0, 0.0, 15.0),
            sensor_at(10.0, 0.0, 15.0),
            sensor_at(1000.0, 0.0, 15.0),
        ];
        let idx = SensorIndex::new(&sensors);
        assert_eq!(idx.max_range(), 15.0);
        let cands = idx.candidates(Vec2::new(5.0, 0.0));
        assert!(cands.contains(&0));
        assert!(cands.contains(&1));
        assert!(!cands.contains(&2));
    }

    #[test]
    fn empty_index_returns_nothing() {
        let idx = SensorIndex::new(&[]);
        assert_eq!(idx.max_range(), 0.0);
        assert!(idx.candidates(Vec2::ZERO).is_empty());
    }

    #[test]
    fn radius_query_respects_distance() {
        let sensors = vec![
            sensor_at(0.0, 0.0, 30.0),
            sensor_at(20.0, 0.0, 10.0),
        ];
        let idx = SensorIndex::new(&sensors);
        let cands = idx.candidates_within(Vec2::new(0.0, 0.0), 5.0);
        assert_eq!(cands, vec![0]);
    }
}
