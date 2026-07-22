# July 20 — Computational Optimization Plan

Code review of `sim-core` + `batch` simulation components, focused on
algorithmic complexity and hot-path throughput.

---

## P0 — Algorithmic

### 1. Spatial cull in `simulate_full`

**File:** `crates/sim-core/src/simulation.rs:121`

The inner loop iterates all ~31k fixed sensors every 1-second tick. A 20-min
walk = 1200 ticks × 31k = 37M wedge tests, each potentially triggering an
occlusion grid query. The batch crate already builds an R-tree for
`exposure_rates_per_minute`; `simulate_full` does not.

**Fix:** Accept a pre-built `RTree<GeomWithData<[f64;2], usize>>` (or a uniform
grid keyed on sensor apex) and cull to `range_m` before the wedge test. The
caller already has the tree in every batch path; the interactive app builds one
for the heatmap. Expected 100–500× reduction in the inner loop for typical
routes (a 15m-range camera only matters within 15m of the walker).

### 2. R-tree snap for `StreetGraph::snap_nearest`

**File:** `crates/sim-core/src/graph.rs:124`

Linear scan over 151k nodes. The comment says "fine for one interactive query."
It isn't: `bg_exposure` calls it per block-group (thousands of points), and
`station_candidates` calls it implicitly via `route_timed_pen` after snapping.
Each call is 151k distance computations.

**Fix:** Build an R-tree (or a uniform grid — the pattern already exists in
`occlusion.rs`) over `positions` at `StreetGraph` construction time. `rstar` is
already a workspace dep. Snap becomes O(log n). Expected ~1000× per snap in
batch paths.

### 3. Spatial cull in `walkshed_exposure_with`

**File:** `crates/sim-core/src/scenario.rs:417`

Tests all passed-in sensors per sample point. The batch pre-culls with an R-tree
(good), but the function signature accepts `&[SensorInstance]` and does a flat
loop. The interactive app passes the full sensor set via `run_route` →
`summarize` → `simulate_full`.

**Fix:** Same spatial index as (1). Alternatively, invert the loop: for each
sensor, test only the sample points within its range (a range query on the
sample-point set). The sample points are known up front; building a grid over
them is O(n) and each sensor query is O(1) amortized. Expected 50–200× on the
app path.

---

## P1 — Data structure / cache

### 4. CSR routing via `fast_paths`

**File:** `crates/sim-core/src/graph.rs:83`

petgraph's `UnGraph` adjacency-list stores edges in a `Vec` with per-node linked
lists. On a 151k/221k graph, A* touches thousands of nodes; each neighbor
traversal is a pointer chase. `fast_paths` is in `[workspace.dependencies]`
(pinned at 1.0) but unused for the walk graph — it builds a CSR with a custom
A* that is typically 5–10× faster on road-scale graphs.

**Fix:** Use `fast_paths` for the walk graph's distance routing. Keep petgraph
for `route_timed` / `route_timed_pen` (non-standard edge weights) or port those
to fast_paths with a custom weight function. The drive graph is a stronger
candidate (larger, queried more in batch).

### 5. Monotone position sampling

**File:** `crates/sim-core/src/graph.rs:492–505`

`Route::position_at` binary-searches `cumulative_m` per call. `sample_over_time`
calls it in a monotone loop: 2000 samples × log(2000) ≈ 22k comparisons. A
single linear pass with a running segment index is O(n) total.

**Fix:** Add `fn sample_over_time_fast(&self, speed, dt) -> Vec<(f64, Vec2)>`
that walks `cumulative_m` with a `seg` cursor. Or restructure `position_at` to
accept a `&mut usize` hint. The existing `binary_search_by` with `partial_cmp`
on f64 also silently maps NaN to `Equal` — not a correctness issue here (arc
lengths are finite) but a smell.

### 6. Dense `node_time` for walkshed edge scan

**File:** `crates/sim-core/src/graph.rs:286–298`

`Walkshed::node_time` is a `HashMap<u32, f64>`; the edge scan does 2 hash
lookups per edge. On a 221k-edge graph, that's 442k hash probes. A dense
`Vec<f64>` indexed by node id (151k entries, ~1.2 MB, sentinel =
`f64::INFINITY`) would be a single array index.

**Fix:** After Dijkstra, build `node_time_dense: Vec<f64>`. The edge scan
becomes two array loads. Keep the HashMap for the public API if needed, or
expose the dense vec.

---

## P2 — Allocation / hot-path hygiene

### 7. Reuse scratch buffer in `sample_polyline`

**File:** `crates/sim-core/src/scenario.rs:182`

Allocates a fresh `Vec<Vec2>` per edge. In `walkshed_exposure_with`, called once
per reachable edge (potentially thousands). Each call allocates, fills, and
drops a small Vec.

**Fix:** Accept a `&mut Vec<Vec2>` scratch buffer, clear-and-reuse. Or inline
the sampling into the test loop so no intermediate Vec exists.

### 8. Avoid allocation in `random_walk_step`

**File:** `crates/sim-core/src/graph.rs:330`

Called per-step for every ambient agent (60 fps × N agents). Each call allocates
a `Vec<(u32, u32)>` via `neighbors()`.

**Fix:** Add `fn neighbors_into(&self, node: u32, out: &mut Vec<(u32, u32)>)`
that clears and fills a caller-owned buffer. Or return a small-array
(`ArrayVec<_, 8>`) since Manhattan intersection degree is ≤ 6 with overwhelming
probability.

### 9. Pre-size `ExposureTally` hash sets

**File:** `crates/sim-core/src/exposure.rs:175,180`

`fixed_seen` and `fixed_groups` grow without bound during a simulation. For the
batch walkshed (10-min, potentially hundreds of cameras) the HashSet rehashes
multiple times.

**Fix:** `HashSet::with_capacity(estimated)` where estimated = nearby sensor
count from the R-tree cull. Minor, but free.

---

## P3 — Minor / structural

### 10. Spatial index for `DashcamFieldLayer::intensity_at` / `TeslaField::intensity_at`

**File:** `crates/sim-core/src/assets.rs:596,629`

O(zones) point-in-polygon (~260 taxi zones), called per-tick. The bbox prefilter
saves most of the cost, but a uniform grid or R-tree over zone bboxes would make
this O(1) worst-case.

**Fix:** At load time, build a coarse grid (cell = 200m) mapping cell →
candidate zone indices. Lookup is one array index + a short candidate list. Same
pattern as `OccluderIndex`.

### 11. Spatial index for ACE corridor distance

**File:** `crates/sim-core/src/simulation.rs:131`

Linear scan over all ACE segments per tick (20 routes × ~50 segments ≈ 1000
`point_segment_distance` calls).

**Fix:** Pre-build a uniform grid or R-tree over ACE segment midpoints; query
only segments within `capture_range_m` of the walker. Or precompute a boolean
"on-corridor" per route sample and interpolate.

### 12. Supercover line traversal in `OccluderIndex::blocked`

**File:** `crates/sim-core/src/occlusion.rs:224`

Traverses the bbox of the sightline, not the cells it crosses. For a 30m
diagonal at 25m cells, the bbox is 2×2 = 4 cells; the segment crosses 2–3.
~30% wasted wall checks.

**Fix:** Bresenham-style cell traversal (supercover line algorithm). Marginal
gain given the short segments, but correct and not much code.

### 13. f32 segment crossing in occlusion

**File:** `crates/sim-core/src/occlusion.rs:47`

The grid stores walls as `[f32; 2]` (good for cache), but converts to f64 via
`v()` for `segments_cross`. The conversion is per-wall-test.

**Fix:** Write `segments_cross_f32` that operates on `[f32; 2]` directly. The
cross-product magnitudes at city scale (≤ 30m segments) are well within f32
precision.

---

## Non-issues (look wrong, aren't)

- `angle_diff` using `%` instead of `rem_euclid`: the subsequent `if` chain
  handles both signs correctly.
- `EnuProjection` equirectangular approximation: sub-meter error at Manhattan
  scale, documented, fine.
- `WyRand` modulo bias in `below()`: n ≤ 6 (graph degree), bias is ~10⁻¹⁰.
- `group_sensors` union-find with path halving: correct and fast for 31k
  elements.

---

## Expected impact

| # | Fix | Where | Speedup |
|---|-----|-------|---------|
| 1 | Spatial cull in `simulate_full` | P0 | 100–500× inner loop |
| 2 | R-tree snap | P0 | ~1000× per snap (batch) |
| 3 | Spatial cull in walkshed exposure | P0 | 50–200× (app path) |
| 4 | fast_paths CSR routing | P1 | 5–10× A* |
| 5 | Monotone position sampling | P1 | ~10× `sample_over_time` |
| 6 | Dense node_time | P1 | 2–3× walkshed edge scan |
| 7 | Scratch buffer reuse | P2 | fewer allocs, better cache |
| 8 | No-alloc random walk | P2 | fewer allocs at 60 fps |
| 9 | Pre-sized hash sets | P2 | fewer rehashes |
| 10 | Zone grid lookup | P3 | O(1) field queries |
| 11 | ACE segment grid | P3 | fewer dist tests/tick |
| 12 | Supercover traversal | P3 | ~30% fewer wall tests |
| 13 | f32 crossing | P3 | fewer promotions |

The P0 items are the ones that change whether a batch run takes 40 seconds or
40 minutes. Everything else is polish.

---

## Suggested order

1. P0-1 + P0-3 together (one spatial index, threaded through both paths).
2. P0-2 (R-tree snap — small, self-contained, immediate batch win).
3. P1-5 (monotone sampling — trivial change, measurable in the sim loop).
4. P1-6 (dense node_time — small change, helps every walkshed query).
5. P1-4 (fast_paths — larger refactor, biggest single-query win for routing).
6. P2 items as cleanup alongside the above.
7. P3 items opportunistically.
