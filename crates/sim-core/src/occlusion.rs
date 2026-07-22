//! Building-footprint line-of-sight occlusion, spatially indexed.
//!
//! [`crate::geometry::sightline_blocked`] is O(n) over the occluder slice, which is fine for a
//! handful of walls and hopeless for a city: NYC has ~1.08 M footprints and, after RDP, on the
//! order of 10⁷ wall segments. This module wraps them in a uniform grid so a sightline query
//! touches only the walls that could plausibly cross it.
//!
//! **Why a grid and not an R-tree** (`rstar` is already a workspace dependency, so this was a real
//! choice): `sim-core` compiles to WASM, and a grid adds no dependency there. And occlusion queries
//! are *short segments* — bounded by the camera range, ≤30 m — so a segment's cell footprint is
//! tiny (≤9 cells at 25 m spacing), whereas an R-tree bbox query on a diagonal 30 m segment returns
//! everything in a 30×30 m box, most of which the segment never comes near.
//!
//! ## The trap this module exists to avoid
//!
//! A camera whose apex sits **inside** a footprint has every outbound ray cross that footprint's
//! boundary, so it is blocked in all directions and silently contributes **zero** exposure. `R_i`
//! drops and it *looks like occlusion working*. The physical truth is that a facade-mounted camera
//! looks **out** from its host building, which must therefore not occlude it — so every query takes
//! an `exclude` polygon, and [`OccluderIndex::containing_polygon`] identifies it.
//!
//! ## Correctness discipline
//!
//! [`OccluderIndex::blocked`] (grid) and [`OccluderIndex::blocked_reference`] (O(n), obviously
//! correct) must agree on every input. That equivalence is a property test, and it is the only
//! reason to trust the fast path.

use std::collections::HashSet;

use crate::assets::BuildingFootprints;
use crate::geometry::{segments_cross, OccluderEdge};
use crate::math::Vec2;

/// Grid spacing. Sightlines are bounded by the max camera range (~30 m), so at 25 m a query's
/// bounding box spans at most 3×3 cells.
pub const DEFAULT_CELL_M: f64 = 25.0;

#[derive(Clone, Copy, Debug)]
struct Wall {
    a: [f32; 2],
    b: [f32; 2],
    /// Index into `rings` — the footprint this wall belongs to.
    poly: u32,
}

#[inline]
fn v(p: [f32; 2]) -> Vec2 {
    Vec2::new(p[0] as f64, p[1] as f64)
}

/// Segment crossing test operating on f32 coordinates directly (avoids the
/// per-wall f64 promotion). Cross-product magnitudes at city scale (≤30 m
/// segments) are well within f32 precision.
#[inline]
fn segments_cross_f32(a1: Vec2, a2: Vec2, b1: [f32; 2], b2: [f32; 2]) -> bool {
    let (ax, ay) = (a1.x as f32, a1.y as f32);
    let (bx, by) = (a2.x as f32, a2.y as f32);
    let dx = bx - ax;
    let dy = by - ay;
    let d1 = dx * (b1[1] - ay) - dy * (b1[0] - ax);
    let d2 = dx * (b2[1] - ay) - dy * (b2[0] - ax);
    let ex = b2[0] - b1[0];
    let ey = b2[1] - b1[1];
    let d3 = ex * (ay - b1[1]) - ey * (ax - b1[0]);
    let d4 = ex * (by - b1[1]) - ey * (bx - b1[0]);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// A uniform-grid spatial index over building-footprint wall segments.
///
/// An **empty** index short-circuits every query to `false`, so the no-occlusion path costs
/// nothing and the pre-occlusion behaviour is exactly recoverable (which is how the regression
/// guard works).
pub struct OccluderIndex {
    cell_m: f64,
    min_x: f64,
    min_y: f64,
    nx: usize,
    ny: usize,
    /// cell -> wall indices whose segment bbox touches it
    cells: Vec<Vec<u32>>,
    /// cell -> polygon indices whose *bbox* touches it (for point-in-polygon; a wall grid alone
    /// cannot answer "is p inside this footprint" when p is deep inside a large ring and none of
    /// its walls land in p's cell).
    poly_cells: Vec<Vec<u32>>,
    walls: Vec<Wall>,
    rings: Vec<Vec<[f32; 2]>>,
}

impl Default for OccluderIndex {
    fn default() -> Self {
        Self::empty()
    }
}

impl OccluderIndex {
    /// No occluders: every sightline is clear. Free.
    pub fn empty() -> Self {
        OccluderIndex {
            cell_m: DEFAULT_CELL_M,
            min_x: 0.0,
            min_y: 0.0,
            nx: 0,
            ny: 0,
            cells: Vec::new(),
            poly_cells: Vec::new(),
            walls: Vec::new(),
            rings: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.walls.is_empty()
    }
    pub fn n_walls(&self) -> usize {
        self.walls.len()
    }
    pub fn n_polygons(&self) -> usize {
        self.rings.len()
    }

    /// Build from baked footprint layers (one per borough). Rings are exterior only.
    pub fn from_footprints(layers: &[BuildingFootprints], cell_m: f64) -> Self {
        let rings: Vec<Vec<[f32; 2]>> = layers
            .iter()
            .flat_map(|l| l.polygons.iter().cloned())
            .filter(|r| r.len() >= 3)
            .collect();
        Self::from_rings(rings, cell_m)
    }

    /// Build from a flat edge slice — each edge becomes its own degenerate "polygon". For tests
    /// and for parity with [`crate::geometry::sightline_blocked`].
    pub fn from_edges(edges: &[OccluderEdge], cell_m: f64) -> Self {
        let rings: Vec<Vec<[f32; 2]>> = edges
            .iter()
            .map(|e| {
                vec![
                    [e.a.x as f32, e.a.y as f32],
                    [e.b.x as f32, e.b.y as f32],
                    // A 2-point "ring" would produce a zero-length second wall; close it back on
                    // itself so the wall set is exactly {a->b} plus two degenerates that can never
                    // properly cross anything (segments_cross rejects collinear/touching).
                ]
            })
            .collect();
        Self::from_rings(rings, cell_m)
    }

    pub fn from_rings(rings: Vec<Vec<[f32; 2]>>, cell_m: f64) -> Self {
        if rings.is_empty() {
            return Self::empty();
        }
        let cell_m = cell_m.max(1.0);
        let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
        let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
        for r in &rings {
            for p in r {
                min_x = min_x.min(p[0] as f64);
                min_y = min_y.min(p[1] as f64);
                max_x = max_x.max(p[0] as f64);
                max_y = max_y.max(p[1] as f64);
            }
        }
        // Pad so a query just outside the fabric still lands in a valid cell range.
        min_x -= cell_m;
        min_y -= cell_m;
        max_x += cell_m;
        max_y += cell_m;
        let nx = (((max_x - min_x) / cell_m).ceil() as usize).max(1);
        let ny = (((max_y - min_y) / cell_m).ceil() as usize).max(1);

        let mut idx = OccluderIndex {
            cell_m,
            min_x,
            min_y,
            nx,
            ny,
            cells: vec![Vec::new(); nx * ny],
            poly_cells: vec![Vec::new(); nx * ny],
            walls: Vec::new(),
            rings,
        };

        for (pi, ring) in idx.rings.iter().enumerate() {
            let n = ring.len();
            // Ring bbox -> poly_cells (for point-in-polygon).
            let (mut rx0, mut ry0, mut rx1, mut ry1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for p in ring.iter() {
                rx0 = rx0.min(p[0] as f64);
                ry0 = ry0.min(p[1] as f64);
                rx1 = rx1.max(p[0] as f64);
                ry1 = ry1.max(p[1] as f64);
            }
            let (i0, j0) = cell_ix(rx0, ry0, min_x, min_y, cell_m, nx, ny);
            let (i1, j1) = cell_ix(rx1, ry1, min_x, min_y, cell_m, nx, ny);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    idx.poly_cells[j * nx + i].push(pi as u32);
                }
            }
            // Walls: every ring edge, closing back to the first vertex.
            for k in 0..n {
                let a = ring[k];
                let b = ring[(k + 1) % n];
                if a == b {
                    continue; // degenerate
                }
                let wi = idx.walls.len() as u32;
                idx.walls.push(Wall { a, b, poly: pi as u32 });
                let (i0, j0) = cell_ix(
                    (a[0] as f64).min(b[0] as f64),
                    (a[1] as f64).min(b[1] as f64),
                    min_x, min_y, cell_m, nx, ny,
                );
                let (i1, j1) = cell_ix(
                    (a[0] as f64).max(b[0] as f64),
                    (a[1] as f64).max(b[1] as f64),
                    min_x, min_y, cell_m, nx, ny,
                );
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        idx.cells[j * nx + i].push(wi);
                    }
                }
            }
        }
        idx
    }

    /// Is the sightline `from`→`to` blocked by a building wall?
    ///
    /// `exclude` is the querying camera's **host** footprint (see the module docs): a facade-mounted
    /// camera must not be occluded by the building it is bolted to.
    ///
    /// Uses supercover line traversal (visits only cells the segment crosses, not
    /// the full bbox) and f32 crossing tests (avoids per-wall f64 promotion).
    pub fn blocked(&self, from: Vec2, to: Vec2, exclude: Option<u32>) -> bool {
        if self.walls.is_empty() {
            return false;
        }
        let (mut ci, mut cj) = self.cell_of(from.x, from.y);
        let (ei, ej) = self.cell_of(to.x, to.y);
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let step_i: i64 = if dx > 0.0 { 1 } else if dx < 0.0 { -1 } else { 0 };
        let step_j: i64 = if dy > 0.0 { 1 } else if dy < 0.0 { -1 } else { 0 };
        let cell = self.cell_m;
        let next_x = if step_i != 0 {
            let boundary = self.min_x + ((ci as f64) + if step_i > 0 { 1.0 } else { 0.0 }) * cell;
            (boundary - from.x) / dx
        } else {
            f64::INFINITY
        };
        let next_y = if step_j != 0 {
            let boundary = self.min_y + ((cj as f64) + if step_j > 0 { 1.0 } else { 0.0 }) * cell;
            (boundary - from.y) / dy
        } else {
            f64::INFINITY
        };
        let dt_x = if step_i != 0 { (cell / dx).abs() } else { f64::INFINITY };
        let dt_y = if step_j != 0 { (cell / dy).abs() } else { f64::INFINITY };
        let mut tx = next_x;
        let mut ty = next_y;
        loop {
            for &wi in &self.cells[cj * self.nx + ci] {
                let w = self.walls[wi as usize];
                if Some(w.poly) == exclude {
                    continue;
                }
                if segments_cross_f32(from, to, w.a, w.b) {
                    return true;
                }
            }
            if ci == ei && cj == ej {
                break;
            }
            if tx < ty {
                tx += dt_x;
                let ni = ci as i64 + step_i;
                if ni < 0 || ni >= self.nx as i64 { break; }
                ci = ni as usize;
            } else {
                ty += dt_y;
                let nj = cj as i64 + step_j;
                if nj < 0 || nj >= self.ny as i64 { break; }
                cj = nj as usize;
            }
        }
        false
    }

    /// O(n) reference: tests **every** wall. Slow, and obviously correct.
    ///
    /// Exists solely so [`Self::blocked`] can be property-tested against it. If those two ever
    /// disagree, the grid is lying and every occlusion number downstream is worthless.
    pub fn blocked_reference(&self, from: Vec2, to: Vec2, exclude: Option<u32>) -> bool {
        self.walls.iter().any(|w| {
            Some(w.poly) != exclude && segments_cross(from, to, v(w.a), v(w.b))
        })
    }

    /// The footprint containing `p`, if any — a camera's host building.
    pub fn containing_polygon(&self, p: Vec2) -> Option<u32> {
        if self.rings.is_empty() {
            return None;
        }
        let (i, j) = self.cell_of(p.x, p.y);
        for &pi in &self.poly_cells[j * self.nx + i] {
            if point_in_ring(p, &self.rings[pi as usize]) {
                return Some(pi);
            }
        }
        None
    }

    /// How many distinct walls a query would actually test — for profiling the grid's selectivity.
    pub fn candidates_for(&self, from: Vec2, to: Vec2) -> usize {
        if self.walls.is_empty() {
            return 0;
        }
        let (i0, j0) = self.cell_of(from.x.min(to.x), from.y.min(to.y));
        let (i1, j1) = self.cell_of(from.x.max(to.x), from.y.max(to.y));
        let mut seen: HashSet<u32> = HashSet::new();
        for j in j0..=j1 {
            for i in i0..=i1 {
                seen.extend(self.cells[j * self.nx + i].iter().copied());
            }
        }
        seen.len()
    }

    #[inline]
    fn cell_of(&self, x: f64, y: f64) -> (usize, usize) {
        cell_ix(x, y, self.min_x, self.min_y, self.cell_m, self.nx, self.ny)
    }
}

#[inline]
fn cell_ix(
    x: f64, y: f64, min_x: f64, min_y: f64, cell_m: f64, nx: usize, ny: usize,
) -> (usize, usize) {
    let i = ((x - min_x) / cell_m).floor();
    let j = ((y - min_y) / cell_m).floor();
    (
        i.clamp(0.0, (nx - 1) as f64) as usize,
        j.clamp(0.0, (ny - 1) as f64) as usize,
    )
}

/// Standard crossing-number point-in-polygon on a closed ring.
fn point_in_ring(p: Vec2, ring: &[[f32; 2]]) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (ring[i][0] as f64, ring[i][1] as f64);
        let (xj, yj) = (ring[j][0] as f64, ring[j][1] as f64);
        if ((yi > p.y) != (yj > p.y)) && (p.x < (xj - xi) * (p.y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square ring, counter-clockwise, side `s` centred on (cx, cy).
    fn square(cx: f64, cy: f64, s: f64) -> Vec<[f32; 2]> {
        let h = (s / 2.0) as f32;
        let (cx, cy) = (cx as f32, cy as f32);
        vec![
            [cx - h, cy - h],
            [cx + h, cy - h],
            [cx + h, cy + h],
            [cx - h, cy + h],
        ]
    }

    #[test]
    fn empty_index_never_blocks() {
        let idx = OccluderIndex::empty();
        assert!(idx.is_empty());
        assert!(!idx.blocked(Vec2::new(-100.0, 0.0), Vec2::new(100.0, 0.0), None));
        assert_eq!(idx.containing_polygon(Vec2::ZERO), None);
    }

    #[test]
    fn a_wall_between_camera_and_target_blocks() {
        let idx = OccluderIndex::from_rings(vec![square(0.0, 0.0, 10.0)], DEFAULT_CELL_M);
        // Straight through the building.
        assert!(idx.blocked(Vec2::new(-20.0, 0.0), Vec2::new(20.0, 0.0), None));
        // Around it.
        assert!(!idx.blocked(Vec2::new(-20.0, 20.0), Vec2::new(20.0, 20.0), None));
    }

    #[test]
    fn host_building_is_excluded_so_a_facade_camera_can_see_out() {
        // THE TRAP: a camera INSIDE a footprint. Without `exclude` it is blind in every direction.
        let idx = OccluderIndex::from_rings(vec![square(0.0, 0.0, 10.0)], DEFAULT_CELL_M);
        let apex = Vec2::ZERO; // dead centre of the building
        let target = Vec2::new(20.0, 0.0); // out in the street

        let host = idx.containing_polygon(apex);
        assert_eq!(host, Some(0), "the apex must be detected as inside footprint 0");

        assert!(
            idx.blocked(apex, target, None),
            "without host exclusion the camera is (wrongly) blind"
        );
        assert!(
            !idx.blocked(apex, target, host),
            "excluding the host building lets a facade camera see the street"
        );
    }

    #[test]
    fn containing_polygon_finds_a_point_deep_inside_a_large_ring() {
        // Regression: a wall-only grid cannot answer this — the point's cell holds no walls of the
        // ring it is inside. This is why `poly_cells` (ring bboxes) exists.
        let big = square(0.0, 0.0, 400.0); // far larger than the 25 m cell
        let idx = OccluderIndex::from_rings(vec![big], DEFAULT_CELL_M);
        assert_eq!(idx.containing_polygon(Vec2::ZERO), Some(0));
        assert_eq!(idx.containing_polygon(Vec2::new(300.0, 300.0)), None);
    }

    #[test]
    fn grazing_a_corner_is_clear() {
        // `segments_cross` treats collinear/touching as NOT crossing, so a camera geocoded exactly
        // on a facade must not self-block.
        let idx = OccluderIndex::from_rings(vec![square(0.0, 0.0, 10.0)], DEFAULT_CELL_M);
        let corner = Vec2::new(5.0, 5.0);
        assert!(!idx.blocked(corner, Vec2::new(20.0, 20.0), None));
    }

    /// ⭐ THE LOAD-BEARING TEST.
    ///
    /// The grid is an optimisation, and an optimisation you cannot verify is a liability. Here the
    /// fast path is checked against the O(n) reference over a dense, randomised city fabric and
    /// thousands of random sightlines. If this fails, every occlusion number is worthless.
    #[test]
    fn grid_agrees_with_the_on_reference_on_every_sightline() {
        // Deterministic LCG — no rand dependency, and a fixed seed keeps failures reproducible.
        let mut s: u64 = 0x5EED_1234_ABCD_0001;
        let mut rnd = move || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) as f64) / ((1u64 << 31) as f64) // [0,1)
        };

        // A block-grid of buildings with jittered sizes — walls at many angles, many per cell,
        // and rings that straddle cell boundaries (the case a naive grid gets wrong).
        let mut rings = Vec::new();
        for gx in 0..14 {
            for gy in 0..14 {
                let cx = gx as f64 * 30.0 + rnd() * 6.0;
                let cy = gy as f64 * 30.0 + rnd() * 6.0;
                let s = 8.0 + rnd() * 16.0; // 8–24 m: some smaller than a cell, some larger
                rings.push(square(cx, cy, s));
            }
        }
        // Plus a few very large rings, to exercise multi-cell walls.
        rings.push(square(200.0, 200.0, 120.0));
        rings.push(square(60.0, 300.0, 90.0));

        let idx = OccluderIndex::from_rings(rings, DEFAULT_CELL_M);
        assert!(idx.n_walls() > 700, "fabric should be dense: {}", idx.n_walls());

        let mut blocked_count = 0usize;
        let n = 20_000;
        for k in 0..n {
            let from = Vec2::new(rnd() * 440.0 - 20.0, rnd() * 440.0 - 20.0);
            // Mix short (realistic camera-range) and long (stress) sightlines.
            let reach = if k % 2 == 0 { 30.0 } else { 200.0 };
            let to = Vec2::new(
                from.x + (rnd() - 0.5) * 2.0 * reach,
                from.y + (rnd() - 0.5) * 2.0 * reach,
            );
            // Exercise the exclude path too.
            let exclude = if k % 3 == 0 { idx.containing_polygon(from) } else { None };

            let fast = idx.blocked(from, to, exclude);
            let slow = idx.blocked_reference(from, to, exclude);
            assert_eq!(
                fast, slow,
                "grid disagreed with reference on sightline #{k}: \
                 ({:.3},{:.3}) -> ({:.3},{:.3}), exclude={exclude:?}",
                from.x, from.y, to.x, to.y
            );
            if fast {
                blocked_count += 1;
            }
        }
        // Guard against a vacuous pass (e.g. everything clear because the fabric or the query
        // range is wrong): the test only means something if it actually exercises both branches.
        assert!(
            blocked_count > n / 10 && blocked_count < n * 9 / 10,
            "expected a healthy mix of blocked/clear, got {blocked_count}/{n} blocked"
        );
    }

    #[test]
    fn matches_the_flat_slice_implementation() {
        // Parity with the pre-existing geometry::sightline_blocked path.
        use crate::geometry::sightline_blocked;
        let edges = vec![
            OccluderEdge { a: Vec2::new(-5.0, 5.0), b: Vec2::new(5.0, 5.0) },
            OccluderEdge { a: Vec2::new(-5.0, -5.0), b: Vec2::new(5.0, -5.0) },
        ];
        let idx = OccluderIndex::from_edges(&edges, DEFAULT_CELL_M);
        for (from, to) in [
            (Vec2::new(0.0, 0.0), Vec2::new(0.0, 20.0)),   // crosses the north wall
            (Vec2::new(0.0, 0.0), Vec2::new(20.0, 0.0)),   // crosses nothing
            (Vec2::new(0.0, -20.0), Vec2::new(0.0, 20.0)), // crosses both
        ] {
            assert_eq!(
                idx.blocked(from, to, None),
                sightline_blocked(from, to, &edges),
                "index disagreed with the flat slice on {from:?} -> {to:?}"
            );
        }
    }
}
