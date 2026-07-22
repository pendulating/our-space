# July 20 — Web Deploy Optimization Plan

Code review of the WASM/WebGPU deployed build (`web/dist/`), focused on load
time, per-frame throughput on single-threaded WASM, bundle size, and memory.

**Current state:** 23 MB WASM · 71 MB total `web/dist/` · ~26 MB over-the-wire
assets (brotli) · 24+ assets loaded eagerly before first interactive frame ·
single-threaded WASM (no `multi_threaded` on wasm32) · up to 9,500 concurrent
agents · Bevy 0.18 + egui + MapLibre JS interop.

---

## P0 — Load time / first paint

### 1. Progressive asset loading (Manhattan-first)

**File:** `crates/app-interactive/src/main.rs:1563` (`build_world`)

`build_world` blocks until **all 24+ asset handles** resolve. The citywide
assets — `taxi_day_nyc` (7.5 MB), `graph_nyc` (4.3 MB), five-borough footprints
(27 MB), `bus_day_nyc` (912 KB), `ace_corridors_nyc` (369 KB) — are only needed
when the user opts into `?city=nyc`. The default Manhattan build downloads and
decompresses them for nothing.

**Fix:** Split `start_loading` into a critical set (Manhattan graph, cameras,
ACE, heatmap, fields, borough, Manhattan footprints) and a deferred set
(citywide graphs, outer-borough footprints, NYC taxi/bus days). `build_world`
fires on the critical set; the deferred set loads in the background and hot-
swaps when the user toggles citywide. Expected: first interactive frame ~3–5 s
sooner on a typical connection (critical path drops from ~26 MB to ~8 MB).

### 2. Brotli decompression off the main thread

**File:** `crates/app-interactive/src/loading.rs:93`

The `PostcardLoader` decompresses brotli synchronously on the single WASM
thread. The 26 MB of compressed assets expand to ~100+ MB; decompression at
~150 MB/s (wasm brotli) costs ~170 ms per large layer, serialized across all
layers. This blocks the Bevy task pool and delays every asset's resolution.

**Fix:** Use the browser's native `DecompressionStream('br')` via a JS shim
(web-sys / wasm-bindgen) instead of the Rust `brotli` crate. The browser's
streaming decoder runs off-thread (Chromium's brotli is in a utility process)
and is 3–5× faster than the wasm port. Alternatively, decode in a Web Worker
and transfer the `ArrayBuffer`. Expected: ~500 ms saved on the critical path,
and the Rust `brotli` dep (180 KB of WASM) can be dropped.

### 3. Service Worker asset caching

**File:** (new) `web/sw.js`

Every page load re-downloads and re-decompresses all assets. GitHub Pages sets
no `Cache-Control` on `application/octet-stream`, so the browser may revalidate
or re-fetch the 26 MB payload on each visit. A Service Worker with a
cache-first strategy for `assets/processed/*` (content-hashed filenames or an
ETag check) makes repeat visits instant.

**Fix:** Register a minimal Service Worker that caches `assets/processed/*` and
the WASM bundle on first load. Subsequent loads serve from the Cache API (disk,
no network). Expected: repeat-visit load drops from ~8 s to <1 s (WASM compile
is the only remaining cost; that too is cached by the browser's code cache).

### 4. Pre-tessellated footprint meshes

**File:** `crates/app-interactive/src/main.rs:1852` (parks), `world.rs` (buildings)

`build_world` runs `earcutr` triangulation on ~1.08 M footprint polygons
(Manhattan: ~1.6 MB of rings; citywide: 27 MB). Earcut on 100k+ polygons is
O(n log n) and costs 200–800 ms on single-threaded WASM. This is pure geometry
that never changes.

**Fix:** Pre-tessellate in the `data-pipeline` bake (native, multi-threaded,
fast) and ship the index buffer as a `.osmesh` asset. `build_world` uploads the
pre-built mesh directly — zero triangulation at load. Expected: 200–800 ms off
the build_world critical path; the earcutr dep can be dropped from the WASM
bundle.

---

## P1 — Per-frame throughput (single-threaded WASM)

### 5. Cached segment cursor per agent

**File:** `crates/app-interactive/src/agents.rs:619,631`

`animate_agents` calls `position_at` (binary search, O(log n)) and `heading_at`
(two more binary searches) for every active agent, every frame. At peak:
6000 taxis + 2500 buses + 400 peds + 200 robots + 400 teslas = 9,500 agents ×
3 binary searches × ~11 segments = ~310k comparisons/frame.

**Fix:** Store a `seg_hint: usize` on `MobileAgent`. Since agents advance
monotonically along their route, the segment index only moves forward. Replace
`position_at(progress_m)` with a `position_at_hint(progress_m, &mut seg_hint)`
that walks forward from the cached index (amortized O(1) per frame). Compute
heading from the same segment (one `atan2`, no second `position_at`). Expected:
~10× fewer comparisons in the agent loop; frees ~0.3 ms/frame at peak.

### 6. heading_at without double position_at

**File:** `crates/sim-core/src/graph.rs:480`

`heading_at(d)` calls `position_at` twice (d−ε, d+ε) for a finite-difference
heading. Each is a binary search. Combined with the position query, that's 3
searches per agent per frame.

**Fix:** Return `(position, heading)` from a single segment lookup: the heading
is the segment's direction vector (already known once the segment is found).
Add `fn position_and_heading_at(&self, d: f64, hint: &mut usize) -> (Vec2, Vec2)`.
Expected: eliminates 2/3 of the per-agent search cost.

### 7. egui dirty-flag gating

**File:** `crates/app-interactive/src/main.rs:1173`

Seven egui systems run in `EguiPrimaryContextPass` every frame regardless of
whether any UI input or state changed. egui's immediate-mode redraw rebuilds
the full widget tree each pass (~0.2–0.5 ms on WASM for this panel density).

**Fix:** Gate the egui pass on a dirty flag: set it on pointer/keyboard input,
on `Params` change, on clock hour-step change, or on a 2 Hz heartbeat (for
animated elements like the time display). Skip the pass entirely on quiet
frames. Expected: saves 0.2–0.5 ms/frame when the user isn't interacting with
the panel (the majority of frames during a time-lapse watch).

### 8. Basemap sync throttle during gestures

**File:** `crates/app-interactive/src/basemap.rs:71`

`sync_basemap` calls into JS (`map.jumpTo`) every frame the camera moves. During
a pan/zoom gesture that's 60 JS interop calls/second, each triggering a full
MapLibre re-render (tile fetch + WebGL draw). The existing gate (skip when
static) is correct but doesn't help mid-gesture.

**Fix:** Throttle the JS call to ≤30 Hz (every other frame) during continuous
motion, or batch the final position and push once per rAF. MapLibre's own
render loop is 60 Hz; feeding it at 30 Hz is visually indistinguishable for a
top-down ortho sync. Expected: halves the JS interop + MapLibre render cost
during gestures.

---

## P2 — Bundle size / memory

### 9. WASM bundle trimming

**File:** `crates/app-interactive/Cargo.toml`, `web/build.sh`

The 23 MB WASM bundle includes: Bevy (render, sprite, text, asset, window),
bevy_egui, bevy_prototype_lyon, earcutr, brotli decoder, rstar, fast_paths,
petgraph, serde, postcard, ehttp, base64. Several are candidates for removal or
lazy loading:

- `brotli` (180 KB): replaced by browser DecompressionStream (P0-2).
- `earcutr` (60 KB): replaced by pre-tessellated meshes (P0-4).
- `fast_paths` + `petgraph` (400 KB): only used for route computation (a few
  times per session). Could be wasm-split into a lazy chunk loaded on first
  route request.
- Bevy `default_font` (300 KB): the app uses Parabolica via egui's font atlas;
  the default font is unused.

**Fix:** Drop brotli + earcutr + default_font (−540 KB). Evaluate wasm-split
for the routing libs. Target: <20 MB WASM.

### 10. Lazy citywide assets (memory)

**File:** `crates/app-interactive/src/main.rs:1657`

`build_world` deserializes and holds **all** assets simultaneously:
- `taxi_day_nyc`: 200k routes × ~20 points × 8 bytes = ~32 MB of polylines
- `graph_nyc`: 151k nodes + 221k edges = ~15 MB
- Five-borough footprints: 27 MB of rings → triangulated meshes = ~40 MB GPU

Total WASM linear memory at steady state: ~250–350 MB. Mobile Safari's WASM
limit is ~1 GB, but the GC pressure and initial allocation spike cause jank.

**Fix:** Hold citywide assets behind an `Option` and drop the Manhattan-only
equivalents when citywide loads (they share the same schema). The taxi route
pool is the biggest win: 200k `QuantPolyline`s could be memory-mapped (keep the
postcard bytes, decode on access) instead of fully deserialized. Expected:
~80 MB off the steady-state footprint in the default Manhattan build.

### 11. Taxi route pool: decode-on-access

**File:** `crates/sim-core/src/assets.rs:870` (`TaxiRoute`, `QuantPolyline`)

The citywide taxi pool (200k routes) is fully deserialized at load: every
`QuantPolyline`'s delta-encoded points are expanded into `Vec<[f32; 2]>`. Most
routes are never displayed (viewport culling admits ~3–6k at a time). The
deserialization costs both time (postcard decode of 7.5 MB) and memory (the
expanded Vecs).

**Fix:** Keep the raw postcard bytes for the route pool and decode a route's
polyline on first access (LRU cache of ~10k decoded routes). The `TaxiTrip`
index (pu_min, route_idx, dur_min) is tiny and stays deserialized for the
schedule scan. Expected: 7.5 MB decode deferred; ~25 MB memory saved; the
per-access decode is ~1 µs (a 20-point polyline).

---

## P3 — Structural / polish

### 12. Pre-baked arc-length tables for agent routes

**File:** `crates/app-interactive/src/agents.rs:875`

Each taxi slot clones a `Route` (with its `cumulative_m` Vec) from the shared
pool on admission. At 6000 slots × ~20 f64s = ~1 MB of duplicated arc-length
data. The routes are immutable; the clone is unnecessary.

**Fix:** Store an `Arc<Route>` (or an index into the shared pool) on
`MobileAgent` instead of an owned `Route`. `position_at` takes `&self`, so a
shared reference works. Expected: ~1 MB memory saved; eliminates the per-slot
Vec clone on admission.

### 13. Instanced rendering for agents

**File:** `crates/app-interactive/src/agents.rs:276`

Each agent is a separate Bevy entity with its own `Mesh2d` + `MeshMaterial2d` +
`Transform`. At 9,500 entities that's 9,500 draw calls (Bevy batches by
material, but each entity still costs a uniform buffer update). The icons are
all the same quad + texture per class.

**Fix:** Use Bevy's `Mesh2d` instancing (or a custom `ExtractInstancesPlugin`)
to render all agents of a class in one draw call with per-instance transforms.
Expected: reduces draw calls from ~9,500 to ~5 (one per class); the GPU upload
is a single buffer write instead of 9,500 individual uniform updates.

### 14. Font subsetting / self-hosting

**File:** `web/index.html:14`

The Parabolica Adobe Fonts kit (`use.typekit.net/vfq0lcs.css`) loads the full
character set (Latin + Latin Extended, two optical cuts, multiple weights).
The page uses ~40 distinct glyphs. Adobe's CDN adds a render-blocking CSS fetch
+ 2–4 font file requests (50–150 KB total).

**Fix:** Subset the two cuts (parabolica + parabolica-text) to the glyphs
actually used (nameplate, headline, body copy, UI labels) and self-host as
WOFF2 in `web/dist/fonts/`. Eliminates the third-party render-blocking request
and cuts font payload ~60%. Expected: 100–300 ms off first contentful paint;
removes the typekit.net dependency (offline-capable).

---

## Non-issues (look wrong, aren't)

- **Basemap JS interop when static:** already gated (`last_view` check); a
  viewer watching the time-lapse generates zero JS calls.
- **`recompute_on_change` per-frame cost:** already signature-gated; `summarize`
  runs at most once per 30 sim-minutes, not per frame.
- **Viewport culling of taxis:** already implemented (route-bbox test); the
  pool bounds on-screen count, not the day's global peak.
- **Bus timetable scan:** already windowed via `bus_admit_indices`
  (partition_point, not a full scan).
- **`QuantPolyline` delta encoding:** already 3–4× smaller than raw f32; the
  decode is O(points) and fast.

---

## Expected impact

| # | Fix | Where | Impact |
|---|-----|-------|--------|
| 1 | Progressive loading | P0 | first paint −3–5 s |
| 2 | Browser brotli | P0 | decode −500 ms, −180 KB WASM |
| 3 | Service Worker cache | P0 | repeat visit <1 s |
| 4 | Pre-tessellated meshes | P0 | build_world −200–800 ms |
| 5 | Agent seg cursor | P1 | −0.3 ms/frame at peak |
| 6 | Combined pos+heading | P1 | −2/3 agent search cost |
| 7 | egui dirty gate | P1 | −0.2–0.5 ms/frame idle |
| 8 | Basemap throttle | P1 | −50% JS interop in gestures |
| 9 | Bundle trim | P2 | −540 KB WASM (−2.3%) |
| 10 | Lazy citywide | P2 | −80 MB steady-state RAM |
| 11 | Decode-on-access taxis | P2 | −25 MB RAM, deferred decode |
| 12 | Arc<Route> sharing | P3 | −1 MB, no per-slot clone |
| 13 | Instanced agents | P3 | 9500 → 5 draw calls |
| 14 | Font subsetting | P3 | −100–300 ms FCP, offline |

The P0 items change whether a first-time visitor waits 8 seconds or 3. The P1
items change whether a time-lapse holds 60 fps or drops to 40 on a mid-range
phone. Everything else is polish.

---

## Suggested order

1. P0-3 (Service Worker) — zero code change in Rust, immediate repeat-visit win.
2. P0-1 (progressive loading) — biggest first-visit win, moderate refactor.
3. P1-5 + P1-6 together (agent cursor + combined heading) — one pass over the
   agent struct, measurable frame-time win.
4. P0-2 (browser brotli) — small JS shim, drops a dep.
5. P0-4 (pre-tessellated meshes) — pipeline change, drops earcutr.
6. P2-9 (bundle trim) — free wins from the above removals.
7. P1-7 (egui gate) — small, measurable on idle frames.
8. P2-10 + P2-11 (lazy citywide + decode-on-access) — larger refactor, memory.
9. P3 items opportunistically.
