# our-space — Architecture

**our-space** is an interactive + batch geospatial simulation of the cameras and
sensing devices entering NYC's public space. You enter a walking route (A→B), or
drop a single point, on the real Manhattan street network and it estimates **how
many cameras could capture you** — across fixed CCTV, Flock/ALPR plate readers,
NYC DOT traffic cameras, ACE bus cameras, rideshare-vehicle dashcams, and
(speculatively) smart glasses — with time-of-day mattering. It is built in Rust with the [Bevy](https://bevy.org)
engine and ships to the browser via WASM + WebGPU.

It is an **honest estimate tool**, not a surveillance map: every number is a
model estimate with a stated confidence tier and provenance, and the route is
computed entirely client-side and never transmitted.

See also: [`DESIGN.md`](DESIGN.md) (the visual/UX system), [`PLAN.md`](PLAN.md)
(the original design + roadmap), and the top-level [`README.md`](../README.md).

## Overview

The system has two halves joined by a set of compact **baked assets**:

- An **offline pipeline** ingests NYC open data (OSM, Census, MTA, TLC, DeFlock,
  the NYC DOT traffic-camera feed, the Amnesty *Decode Surveillance NYC* census,
  the Dahir et al. CCTV dataset), normalizes it, projects it to local ENU meters,
  and serializes it to small `postcard` binaries.
- A **render-agnostic simulation core** (`sim-core`) loads those assets and
  computes exposure. It runs in two hosts: the interactive **Bevy app**
  (native dev window; WebGPU/WASM for the public web build) and a native
  **headless batch** that pre-computes the citywide heatmap.

The load-bearing architectural rule: all movement, capture-detection, and
exposure math live in `sim-core` with **no rendering/GPU dependency**, so the
exact same logic runs in the browser and in batch, and the analytical core
compiles and unit-tests in milliseconds (39 tests today).

## Workspace layout

```
crates/
  sim-core/        Render-agnostic core: ENU projection, FOV/occlusion geometry,
                   the exposure model, routable graph + A* + walkshed, the
                   simulation loop, and the baked-asset structs. Pure Rust; an
                   optional `ecs` feature adds a (currently unused) Bevy layer.
  data-pipeline/   Native CLI: bakes raw NYC open data into static assets.
  app-interactive/ Bevy 0.18 app (native window + WebGPU/WASM); the only crate
                   that draws pixels. Loads assets via AssetServer.
  batch/           Native headless CLI: citywide coverage-aggregation heatmap.
crates/app-interactive/assets/processed/   Baked, client-loadable assets.
web/               build.sh (wasm-bindgen + wasm-opt bundler) + index.html chrome.
docs/              This file, DESIGN.md, PLAN.md.
```

The Bevy stack is pinned to `0.18`; `sim-core` is reused unchanged by the app,
batch, and a headless `route_demo` example.

## End-to-end data flow

1. **Fetch** — raw sources are pulled into `data/snapshots/` via `curl`,
   DuckDB (over the remote TLC Parquet), and the Overpass API.
2. **Bake** — `data-pipeline` subcommands parse, filter, project to ENU, and
   write one `.os*` postcard file per layer into the app crate's `assets/processed/`.
3. **Derive** — `batch heatmap` reads the baked graph + sensor layers and writes
   the per-edge `heatmap.osheat`.
4. **Load** — the app/batch deserialize the baked layers (the app via Bevy's
   `AssetServer`, which reads from disk natively and fetches over HTTP on web).
5. **Simulate** — `sim-core` routes A→B (or builds a walkshed) and accumulates
   per-class exposure, which the app renders and the panel summarizes.

---

## Simulation core (`sim-core`) & the exposure model

`sim-core` is the **render-agnostic** heart of our-space: all movement, capture-detection, and exposure math live here with no Bevy/render/GPU dependency, so the identical logic runs in the WASM/WebGPU interactive build and a native headless batch, and the analytical core compiles and unit-tests in milliseconds. A Bevy component/resource layer (`ecs` module) is gated behind the optional `ecs` feature and is the only place Bevy types appear.

### Module map

| Module | File | Responsibility |
| --- | --- | --- |
| `math` | `crates/sim-core/src/math.rs` | 2D `Vec2` (f64) in local ENU meters; `point_segment_distance`, `angle_diff`. |
| `projection` | `crates/sim-core/src/projection.rs` | Closed-form WGS84 ↔ ENU-meters (`EnuProjection`, `GeoOrigin`), no native PROJ. |
| `geometry` | `crates/sim-core/src/geometry.rs` | `FrustumWedge` FOV wedges + 2D line-of-sight occlusion (`captures`, `OccluderEdge`). |
| `exposure` | `crates/sim-core/src/exposure.rs` | The exposure model: `SourceKind`, `ConfidenceTier`, `ExposureTally`, `DAHIR_RECALL`. |
| `mobile` | `crates/sim-core/src/mobile.rs` | Time-of-day functions + mobile/ambient scenario configs. |
| `graph` | `crates/sim-core/src/graph.rs` | `StreetGraph` (petgraph), A* `route`, `Walkshed`/Dijkstra isochrone, position-over-time. |
| `simulation` | `crates/sim-core/src/simulation.rs` | The discrete-clock walk loop (`simulate_full`) and per-minute heatmap rates. |
| `scenario` | `crates/sim-core/src/scenario.rs` | Orchestration: assets → sensors, `run_route`, `summarize`, `walkshed_exposure`. |
| `assets` | `crates/sim-core/src/assets.rs` | Serde/postcard structs for baked layers, incl. `DashcamFieldLayer`. |

### ENU projection

`EnuProjection` (`projection.rs`) uses an **equirectangular tangent-plane** approximation about a `GeoOrigin`, accurate to well under a meter at borough scale, deliberately avoiding the `proj` C crate which does not build for `wasm32-unknown-unknown`. The default origin is `GeoOrigin::MANHATTAN` = lat `40.7831`, lon `-73.9712` (Midtown). With `EARTH_RADIUS_M = 6_371_008.8`:

- `to_enu(lat, lon)` → `Vec2 { x: R·Δlon·cos(lat0), y: R·Δlat }` (east = +x, north = +y).
- `to_wgs84(p)` inverts it. `cos_lat0` is precomputed at construction.

### FOV-wedge geometry & 2D occlusion

A `FrustumWedge` (`geometry.rs`) is `{ apex, heading_rad, half_fov_rad, range_m }` using the compass convention (0 = north, clockwise; `Vec2::bearing_rad` = `x.atan2(y)`). `from_degrees` takes the **full** FOV in degrees and halves it; a `None` heading yields an omnidirectional sensor modeled as a full 360° wedge (`half_fov_rad = π`).

- `covers_unoccluded(p)` — within `range_m` and within the angular wedge (via `angle_diff(bearing, heading) ≤ half_fov_rad`); omnidirectional short-circuits to range-only; a point exactly at the apex counts as covered.
- `segments_cross` — orientation-sign proper-crossing test (collinear/grazing cases return false, treated as clear).
- `sightline_blocked(from, to, occluders)` — true if any `OccluderEdge` (a building-footprint wall segment) crosses the camera→walker sightline. Callers pre-filter occluders to those near the segment (e.g. R-tree). Manhattan sightlines are wall-limited, so without this fixed coverage is grossly over-counted.
- `captures(wedge, target, occluders)` = `covers_unoccluded(target) && !sightline_blocked(apex, target, occluders)`.

### Source classes & confidence tiers

`SourceKind` (`exposure.rs`) enumerates six sensing classes. `SourceKind::ALL` fixes their order, matching the `[SourceTally; 6]` index map in `ExposureTally::idx`.

| `SourceKind` | `label()` | `tier()` | `is_fixed()` | `recall_corrected()` |
| --- | --- | --- | --- | --- |
| `FixedCctv` | "Fixed CCTV" | B | yes | **yes** |
| `DotLiveView` | "DOT live-view" | A | yes | no |
| `AceBus` | "ACE buses" | A | no | no |
| `Dashcam` | "Rideshare cams" | C | no | no |
| `SmartGlasses` | "Smart glasses" | D | no | no |
| `Alpr` | "ALPR readers" | A | yes | no |

`ConfidenceTier` governs UI honesty: **A** mapped/public infrastructure (DOT cams, ACE, ALPR points), **B** estimated from inventories (fixed CCTV), **C** modeled field × assumed penetration (dashcams), **D** speculative/emerging (smart glasses). `is_fixed()` distinguishes point-with-frustum classes (distinct device counting) from mobile/ambient classes (Poisson encounters). The `recall_corrected()` hook applies **only to `FixedCctv`**, and the correction factor comes from the *layer's* `recall` field — so it engages only if the shipped CCTV layer declares one. The current merged layer carries `recall: None` (a direct census; see below), so no inflation is applied; the Dahir-only fallback layer (`bake-cameras`) declares `recall: Some(0.63)` and would re-enable it.

### The exposure model

The headline metric is **"cameras that saw you"** — expected distinct devices whose coverage the route entered, summed across all classes. `DAHIR_RECALL = 0.63` is the recall of the Dahir et al. street-view camera detector; for a Dahir-only CCTV layer, observed density underestimates the truth by this factor, so the count is divided by it (`recall_factor = 1/0.63 ≈ 1.587`) and surfaced as an uncertainty band, never silently applied. The shipped layer instead **merges** the Amnesty crowdsourced census with Dahir (next section), which is a direct count rather than an ML detection, so it ships `recall: None` — the headline is the census count, un-inflated.

`ExposureTally` holds `per_source: [SourceTally; 6]` (each `{ devices, frames }`), a transient `fixed_seen: HashSet<(u8, u64)>` (so a fixed camera seen across many ticks counts once), `route_length_m`, `covered_length_m`, and `recall_factor` (default 1.0). `p_at_least_one(expected) = 1 − e^(−expected)`.

**Fixed cameras** — `record_fixed_capture(kind, id, frame_rate, dt)` accrues `frame_rate × dt` frames every covered tick and increments `devices` by 1 only the first time `(kind_idx, id)` is seen. Distinct devices counted, frames accumulated. Same `id` under a different `kind` counts separately.

**Mobile/ambient** — modeled as rare independent Poisson encounters. `record_mobile(kind, expected_devices, expected_frames)` adds the per-tick intensity increments directly to `devices` (the Poisson mean) and `frames`; `P(≥1) = 1 − e^(−mean)`.

Key tally accessors:
- `adjusted_devices(kind)` — `devices × recall_factor` for recall-corrected classes (FixedCctv only), else raw `devices`.
- `headline_device_count()` — `Σ adjusted_devices` over `ALL`, rounded, clamped ≥ 0, as `u32`.
- `p_capture(kind)` = `p_at_least_one(adjusted_devices(kind))`.
- `total_expected_frames()` — Σ frames across all sources.
- `fraction_surveilled()` = `covered_length_m / route_length_m`, clamped to `[0,1]` (0 if route length ≤ 0).

#### Route-first, expose-second

Routing is decoupled from exposure. The caller computes the `Route`; `simulate_full` (`simulation.rs`) then samples position(t) at constant speed and tests each tick. `simulate_fixed` is a thin wrapper calling `simulate_full` with `MobileScenario::default()`, `departure_hour = 12.0`, no dashcam field.

`simulate_full(route, fixed, occluders, mobile, params, departure_hour, dashcam_field)` walks `route.sample_over_time(speed, dt)`. `SimParams` defaults: `speed_mps = DEFAULT_WALK_SPEED_MPS (1.34)`, `dt = 1.0`, `recall_factor = 1.0`. The final (possibly partial) tick is weighted by its true duration `(t_i − t_{i-1}).min(dt)` so the capture integral isn't over-counted at the route end. The clock advances as `hour = departure_hour + t_elapsed/3600`.

Per tick:
- **Fixed**: for each `SensorInstance`, if `captures(wedge, pos, occluders)` → `record_fixed_capture` and mark `covered_here`.
- **ACE buses**: if the min `point_segment_distance` to any corridor segment ≤ `capture_range_m`, then `headway_s = max(bus_headway_minutes(hour)·60·headway_scale, 1)`, `encounters = (directions / headway_s) · tick_dt`; record as devices and `encounters · frames_per_pass`.
- **Dashcams**: `zone = dashcam_field.intensity_at(pos)` (else 1.0); `veh_per_s = (vehicles_per_min_peak/60) · traffic_multiplier(hour) · zone`; `encounters = veh_per_s · penetration · capture_prob · tick_dt`; frames `· frames_per_pass`.
- **Smart glasses**: `peds_per_s = (peds_per_min_peak/60) · pedestrian_multiplier(hour)`; `encounters = peds_per_s · (per_1000_pedestrians/1000) · p_recording · capture_prob · tick_dt`; frames `· frames_per_pass`.
- `record_progress(prev.distance(pos), covered_here)` accumulates route length and surveilled length.

#### Diurnal functions (`mobile.rs`)

- `bus_headway_minutes(hour)` — step function: **5 min** rush (07–09, 16–19), **10 min** daytime (06–22), **25 min** overnight.
- `traffic_multiplier(hour)` = `diurnal_two_peak(hour, floor=0.12, am=8.5, pm=17.5)`.
- `pedestrian_multiplier(hour)` = `diurnal_two_peak(hour, floor=0.10, am=9.0, pm=18.0)`.
- `diurnal_two_peak` — a smooth two-Gaussian (AM/PM) curve plus a `0.55`-weighted midday bump at h=13, returns `floor + (1−floor)·max(am, pm, midday)`, bounded in roughly `[floor, 1.0]`.

#### Mobile scenario configs (`mobile.rs`)

| Config | Defaults |
| --- | --- |
| `AceConfig::new(segments)` | `capture_range_m = 20.0`, `directions = 2.0`, `frames_per_pass = 30.0`, `headway_scale = 1.0` |
| `DashcamConfig::default()` | `penetration = 0.40`, `vehicles_per_min_peak = 12.0`, `capture_prob = 0.40`, `frames_per_pass = 5.0` (baseline = median-density taxi zone at peak) |
| `GlassesConfig::default()` | `per_1000_pedestrians = 10.0`, `p_recording = 0.05`, `peds_per_min_peak = 60.0`, `capture_prob = 0.4`, `frames_per_pass = 3.0` |

`MobileScenario { ace, dashcam, glasses }` enables classes by `Some(_)`; `MobileScenario::fields_only()` enables dashcam + glasses at defaults with ACE off (ACE needs baked corridors). Dashcam/glasses intensities are explicitly scenario assumptions, not measurements.

#### `DashcamFieldLayer.intensity_at` (`assets.rs`)

The dashcam rate is modulated by real spatial rideshare density from NYC TLC High-Volume FHV trip records, baked per taxi zone. `intensity_at(p)` iterates `zones`; for each `DashcamZone` it first does a cheap bbox prefilter (`p.x/p.y` within `bbox = [min_x, min_y, max_x, max_y]`) then a ray-casting `point_in_ring(p, exterior)`; on a hit it returns that zone's `intensity` (relative to the median Manhattan zone, ≈1.0 typical, up to ~8× in Midtown). Outside all zones it falls back to `1.0` so the dashcam class never silently vanishes.

### The routable graph (`graph.rs`)

`StreetGraph::from_asset(GraphAsset)` builds a petgraph `UnGraph<(), usize>` where node positions come from `asset.nodes` and each edge weight is the index into `asset.edges`. `DEFAULT_WALK_SPEED_MPS = 1.34` (~4.8 km/h).

- `snap_nearest(p)` — linear-scan nearest node (fine for one interactive query at borough scale).
- `route(start, goal)` — petgraph `astar` with edge cost `edges[w].length_m` and an admissible Euclidean straight-line heuristic to `goal_pos`; returns `RouteError::Empty` (empty graph) or `RouteError::NoPath`.
- `route_points(from, to)` — snaps both ENU points then routes (mirrors the OSMnx snap+validate workflow).
- `build_route` stitches the node path into a continuous ENU polyline, reversing each stored edge polyline when traversed against its `from→to` direction and dropping duplicated joint vertices (`push_polyline`).

`Route { points, cumulative_m, total_m }` (`from_points` computes cumulative arc length). `position_at(d)` clamps to `[0, total_m]` and binary-searches the containing segment to lerp. `sample_over_time(speed, dt)` yields `(elapsed_s, pos)` at fixed `dt` over `duration = total_m/speed`, with the final sample landing exactly on the endpoint.

#### Walkshed (Dijkstra isochrone) & walkshed exposure

`walkshed(start, max_seconds, speed_mps)` runs petgraph `dijkstra` with **time-weighted** edges (`length_m / speed_mps`), keeps nodes with cost ≤ `max_seconds`, and includes an edge in the walkshed only when **both** endpoints are reachable in time. Returns `Walkshed { start, max_seconds, node_time: HashMap<u32, f64>, edges: Vec<u32> }`.

`walkshed_exposure(graph, ws, sensors, occluders, recall_factor)` (`scenario.rs`) counts distinct fixed cameras whose FOV covers any reachable street point: for each walkshed edge it samples the first/middle/last polyline vertices, and for each sensor not yet `seen`, if `captures(...)` it records the camera id and apex point. Returns `WalkshedSummary { max_minutes (= max_seconds/60), reachable_edges, cameras_raw, cameras_corrected (= raw · recall_factor), camera_points }`.

### Per-minute exposure rates (heatmap)

`ExposureRates { fixed, ace, dashcam, glasses }` with `total()` is the per-class expected devices/minute-of-presence at a point — kept per class so a uniform field (dashcams) doesn't wash out the spatial signal of fixed cameras / ACE. `exposure_rates_per_minute(point, hour, nearby_fixed, occluders, near_ace, mobile, recall_factor, dashcam_field)`:
- `fixed` = count of `nearby_fixed` whose `captures(...)` is true, × `recall_factor` (caller spatially culls candidates; each covering camera contributes one device).
- `ace` (only if `near_ace`) = `(directions / headway_s) · 60`.
- `dashcam` = `(vehicles_per_min_peak/60) · traffic_multiplier(hour) · intensity_at(point) · penetration · capture_prob · 60`.
- `glasses` = `(peds_per_min_peak/60) · pedestrian_multiplier(hour) · (per_1000_pedestrians/1000) · p_recording · capture_prob · 60`.

These map onto the baked `HeatmapLayer` (`assets.rs`), which stores per-edge `fixed`/`ace`/`dashcam`/`glasses` vectors at a `reference_hour`, in the same order as `GraphAsset.edges`.

### Key public functions

| Function | Module | Role |
| --- | --- | --- |
| `run_route` | `scenario.rs` | Route two ENU points, simulate full exposure, return `(Route, RouteSummary)`. |
| `summarize` | `scenario.rs` | Simulate + summarize an already-routed path (re-eval on slider/hour change without re-routing). Produces `RouteSummary` with headline, frames, fraction surveilled, and per-class `SourceBreakdown` (classes with `adjusted_devices > 1e-6`). |
| `simulate_full` | `simulation.rs` | The per-tick fixed + mobile exposure loop (described above). |
| `simulate_fixed` | `simulation.rs` | Fixed-only convenience wrapper over `simulate_full`. |
| `exposure_rates_per_minute` | `simulation.rs` | Per-class per-minute rates at a point for the heatmap. |
| `walkshed` | `graph.rs` | Dijkstra time-isochrone of reachable streets. |
| `walkshed_exposure` | `scenario.rs` | Distinct cameras touching a walkshed → `WalkshedSummary`. |
| `sensors_from_layer` | `scenario.rs` | Bake `FixedSensorLayer` → `Vec<SensorInstance>`; the vector index becomes the device `id`. `FixedCameraDefaults` = `full_fov_deg 70.0`, `range_m 15.0`, `frame_rate 15.0`. |

---

## Data pipeline, sources & baked assets

### Offline ingest model

`our-space` ships **no live data fetching at runtime**. Raw NYC open datasets are pulled offline (via `curl` / DuckDB / the Overpass API) into snapshot files, then the `data-pipeline` crate *bakes* each one into a compact, self-contained static binary asset. The app (`app-interactive`) and the headless `batch` host load only these baked assets — they never touch the raw sources or the network.

The flow per layer is:

1. **Snapshot** — raw source downloaded to a working `data/snapshots` location (CSV, GeoJSON, Overpass JSON, GTFS dir).
2. **Bake** — a `data-pipeline` subcommand parses, filters, projects to local ENU meters (`EnuProjection::default()`, origin `GeoOrigin::MANHATTAN`), and serializes to postcard.
3. **Load** — the baked file is placed under `crates/app-interactive/assets/processed/` (Bevy's asset root) with a type-specific extension and consumed by a typed `AssetLoader`.

The `batch heatmap` step is a second-stage derived bake: it reads the already-baked graph + sensor layers and produces the per-edge heatmap.

### Bake subcommands (`data-pipeline/src/main.rs`)

Each subcommand writes to the `<out>` path it is given. Parent directories are auto-created via `ensure_parent`.

| Subcommand | Args | Module / entry | Produces |
|---|---|---|---|
| `bake-graph --overpass-json` | `<walk.json> <out>` | `graph_osm::bake` | routable pedestrian `GraphAsset` from OSM walk network |
| `bake-graph --synthetic` | `<rows> <cols> <spacing_m> <out>` | `graph_synth::synthetic_grid` | synthetic grid graph (testing/dev; no provenance source) |
| `bake-cameras` | `<map_data.csv> <out>` | `cameras_dahir::bake` | Dahir-only fixed-CCTV `FixedSensorLayer` (fallback) |
| `bake-cctv` | `<amnesty_counts.csv> <dahir_map_data.csv> <out>` | `amnesty::bake` | **unified** fixed-CCTV `FixedSensorLayer` (Amnesty census + Dahir, de-duplicated) |
| `bake-ace` | `<gtfs_dir> <ace_routes.json> <out>` | `ace::bake` | `AceCorridorLayer` |
| `bake-equity` | `<bg.geojson> <acs.json> <map_data.csv> <out>` | `equity::bake` | `EquityLayer` |
| `bake-dashcam-field` | `<taxi_zones.geojson> <zone_trips.csv> <out>` | `dashcam::bake` | `DashcamFieldLayer` |
| `bake-alpr` | `<alpr_overpass.json> <out>` | `alpr::bake` | ALPR `FixedSensorLayer` |
| `bake-dot` | `<nyctmc_cameras.json> <out>` | `dot::bake` | NYC DOT traffic-cam `FixedSensorLayer` (locations only) |
| `bake-vehicle-routes` | `<graph.osgraph> <taxi_zones.geojson> <zone_od.csv> <out> [max_routes]` | `vehicle_routes::bake` | `VehicleRoutesLayer` (weighted rideshare route pool for the animated agents) |

The heatmap is **not** a `data-pipeline` subcommand — it is produced by the `batch` crate: `batch heatmap <out.postcard> [hour]` (default `hour = 17.0`).

### Data sources & licenses

| Source | What it is | Provider | License (as baked) |
|---|---|---|---|
| OSM walk network | Pedestrian-usable highways, fetched as an Overpass API JSON dump | OpenStreetMap via Overpass API | ODbL 1.0 |
| Amnesty Decode NYC | Crowdsourced camera **counts per intersection** (median over 3 volunteers; `counts_per_intersections.csv`) — the dominant, most complete street-CCTV census | Amnesty International, Decode Surveillance NYC (`github.com/amnesty-crisis-evidence-lab/decode-surveillance-nyc`) | CC BY-NC-ND 4.0 — used non-commercially, attributed (see note below) |
| `map_data.csv` | Google Street View sample-points where a camera was detected (panorama-level, detector recall ~0.63 — **not** surveyed devices); merged with Amnesty to add mid-block detections | Dahir et al. 2025, Stanford Digital Repository (`purl.stanford.edu/jr882ny4955`) | CC BY 4.0 |
| DeFlock ALPRs | Crowdsourced license-plate-reader points synced into OSM as `man_made=surveillance` + `surveillance:type=ALPR`, fetched via Overpass | DeFlock via OpenStreetMap (`deflock.me`) | ODbL 1.0 |
| NYC DOT traffic cams | Public PTZ traffic-monitoring camera **locations** from the TMC feed (`webcams.nyctmc.org/api/cameras/`); the `imageUrl` field is never read or stored | NYC DOT Traffic Management Center (`nyctmc.org`) | No open license — **coordinates only, images not used or redistributed** |
| MTA GTFS static | Bus route geometry (`routes.txt`, `trips.txt`, `shapes.txt`) | MTA | MTA / OPEN-NY Terms of Use |
| ACE route list `ki2b-sg5y` | "MTA Bus Automated Camera Enforced Routes" JSON | data.ny.gov (`data.ny.gov/d/ki2b-sg5y`) | MTA / OPEN-NY Terms of Use |
| HVFHV trip records | High-Volume For-Hire-Vehicle (Uber/Lyft) trips, aggregated per taxi zone (PU+DO) via DuckDB over the remote Parquet | NYC TLC Trip Record Data | NYC OpenData / TLC terms |
| NYC taxi zones | Taxi-zone polygons keyed by `LocationID` (WGS84) | NYC TLC / OpenData | NYC OpenData / TLC terms |
| TIGER block groups | Block-group boundary GeoJSON, NY County `36061`, keyed by `GEOID`/`GEOID20` | Census TIGERweb | Census public domain |
| ACS B03002 | 5-year race/ethnicity counts (white/Black/Asian/Hispanic + total) per GEOID | Census Reporter (keyless ACS) | Census public domain |

### Baked assets

Counts are the real values produced by this build (from each module's summary).

| File (under `assets/processed/`) | Ext | sim-core type | Source | Counts (this build) |
|---|---|---|---|---|
| `graph_manhattan.osgraph` | `.osgraph` | `GraphAsset` | OSM walk via Overpass (largest connected component, ODbL) | 151,339 nodes / 220,609 edges |
| `cameras_fixed.oscam` | `.oscam` | `FixedSensorLayer` (`FixedCctv`, `recall: None`) | Amnesty Decode NYC census + Dahir et al. (de-duplicated) | 4,422 cameras (4,266 Amnesty + 156 Dahir-unique) |
| `alpr.osalpr` | `.osalpr` | `FixedSensorLayer` (`Alpr`, `recall: None`) | DeFlock via OSM (ODbL) | 444 readers |
| `dot_cameras.osdot` | `.osdot` | `FixedSensorLayer` (`DotLiveView`, `recall: None`) | NYC DOT TMC feed (locations only) | 370 Manhattan cams (online) |
| `vehicle_routes.osroutes` | `.osroutes` | `VehicleRoutesLayer` | TLC HVFHV zone O-D routed over the walk graph (decorative) | 1,000 weighted routes (~159 KB) |
| `ace_corridors.osace` | `.osace` | `AceCorridorLayer` | MTA GTFS + data.ny.gov `ki2b-sg5y` | 20 routes / 15,361 segments |
| `dashcam_field.osfield` | `.osfield` | `DashcamFieldLayer` | NYC TLC HVFHV trips per taxi zone (DuckDB over remote Parquet) | 354 zone parts |
| `equity.osequity` | `.osequity` | `EquityLayer` | Census TIGER geometry + Census Reporter ACS B03002 + Dahir detections | 1,299 block groups |
| `heatmap.osheat` | `.osheat` | `HeatmapLayer` | `our-space` batch coverage aggregation (derived) | per-edge, per-class |

### Serialization & loader resolution

All layers (de)serialize with **postcard** (compact, WASM-friendly binary; `to_bytes` → `postcard::to_allocvec`, `from_bytes` → `postcard::from_bytes`), defined on each type in `crates/sim-core/src/assets.rs`.

The byte format is identical across the three fixed-camera layers (CCTV, ALPR, DOT — all `FixedSensorLayer`), so the file *extension* is what disambiguates them. Each asset type gets a **distinct extension** registered to a typed `PostcardLoader<A>` in `crates/app-interactive/src/loading.rs` whose `extensions()` returns that single extension — so each file resolves to exactly one Rust type with no ambiguity.

Every layer ships a `Provenance { source, url, license, as_of, notes }` struct so the UI can render an honest source/date/license badge.

### Batch coverage-aggregation heatmap (`batch/src/main.rs`)

`batch heatmap <out> [hour]` (default reference hour `17.0`) loads the baked graph + sensor layers and computes, for every graph edge, the **expected devices that would capture you per minute of presence**, split by class. Output is a `HeatmapLayer` whose `fixed`/`ace`/`dashcam`/`glasses` vectors are aligned to `GraphAsset.edges` order (classes kept separate so a uniform field doesn't wash out spatial signal).

Mechanics:

- **Fixed cameras (CCTV + ALPR + DOT)** — the CCTV layer plus (if present) ALPR and NYC DOT traffic cams are merged into one fixed-sensor set via `sensors_from_layer`, reindexed with sequential ids. DOT cams use `FixedCameraDefaults::dot_monitoring()` (omnidirectional, 30 m reach, 1 fps); CCTV/ALPR use the default 70°/15 m/15 fps. An `rstar::RTree<GeomWithData<[f64;2], usize>>` over the apex positions is queried per edge within `cam_query_r2 = 60.0² m²` (a generous cull; the true range is enforced by the FOV test). Recall correction is `1.0 / cam_layer.recall.unwrap_or(1.0)`.
- **ACE** — corridor segments are densified to ~10 m points and bulk-loaded into an `RTree<[f64;2]>`; an edge is "near ACE" if any point is within `capture_range_m²`.
- **Dashcam field** — passed through so the dashcam class is **spatial**, not uniform.
- **Per edge** — at the edge polyline midpoint, `exposure_rates_per_minute(...)` returns per-class rates; the `glasses` field comes from `MobileScenario::fields_only()`.

### Rideshare density (`dashcam.rs`)

Models that NYC for-hire vehicles must carry cameras, so exposure follows where Uber/Lyft drive:

1. Per-zone trip counts (`loc,trips`, pre-aggregated PU+DO from HVFHV records via DuckDB) are read from CSV; taxi-zone polygons are parsed and projected to ENU (exterior rings; multipolygon parts summed).
2. Zone **area** via the shoelace formula; **density** = `trips / area`.
3. The **median** density over zones with data (robust to airport outliers).
4. Each zone's **intensity** = `(density / median)`, clamped to `[0.0, 8.0]` (≈1.0 = a typical zone).
5. `DashcamFieldLayer::intensity_at(p)` does bbox prefilter + ray-cast point-in-polygon; outside all zones it falls back to `1.0`.

### Fixed-CCTV merge & de-duplication (`amnesty.rs`)

`bake-cctv` unifies two independent Google-Street-View censuses of the **same physical camera population**, so they are de-duplicated rather than summed:

1. **Amnesty Decode Surveillance NYC** (`counts_per_intersections.csv`) is the base: each Manhattan intersection reporting `n_cameras_median ≥ 1` becomes `n` **omnidirectional** sensors at the panorama point (the aggregate has no per-camera bearing). Rows with empty `Lat`/`Long` (unresolved panoramas) are skipped. ≈4,266 cameras across ≈2,019 intersections.
2. **Dahir et al.** detections are added **only where they don't duplicate** an Amnesty corner: a Dahir camera within `DEDUP_RADIUS_M = 50 m` (ENU) of any Amnesty camera-bearing intersection is dropped; the rest (≈156, mostly mid-block — Amnesty sampled intersections only) are kept with their GSV heading. 

The merged layer is `FixedCctv` with `recall: None` — Amnesty's direct counts dominate, so the Dahir ML-recall correction does not apply to the set. **Licensing:** the Amnesty data is CC BY-NC-ND 4.0; this project's non-commercial/research posture satisfies NC, the source is attributed on every surface, and baking is treated as permissible factual-data use under that posture. The equity overlay (`bake-equity`) deliberately **stays Dahir-only**, since it replicates the specific Dahir et al. diversity↔camera correlation.

### ALPR headings (`alpr.rs`)

Each Overpass element's coordinate comes from `lat`/`lon` (or `center` for ways). The heading is parsed from the OSM **`direction`** tag (the compass bearing the reader faces, matching `FrustumWedge` heading): `tags.get("direction").and_then(|d| d.parse::<f64>().ok())`. Non-numeric values — including multi-direction sites (e.g. `0;328`) — yield `heading_deg = None`, i.e. modeled **omnidirectional** (a reasonable read of a multi-camera pole). ALPRs are mapped device locations, so the layer carries `recall: None`.

---

## App, rendering, interaction & web build

The interactive front end (`crates/app-interactive`) is a [Bevy](https://bevy.org) `0.18` application — the only crate that draws pixels. All simulation logic stays in `sim-core`; the app consumes results and turns them into meshes.

### App structure (one sim-core, two hosts)

`main()` builds a single `App` that runs on **native** (desktop dev window; Metal/Vulkan/DX auto-selected) and **web** (WebGPU + WASM, bound to `#bevy-canvas`). It sets `DefaultPlugins.set(WindowPlugin{ canvas: Some("#bevy-canvas"), fit_canvas_to_parent: true, prevent_default_event_handling: true, .. })` (canvas fields are ignored natively), `EguiPlugin::default()`, the parchment `ClearColor`, and on `wasm32` calls `console_error_panic_hook::set_once()` first.

| File | Role |
|------|------|
| `main.rs` | App wiring, resources/components, all `Update` systems |
| `loading.rs` | Cross-platform postcard asset loading |
| `world.rs` | ENU→world conversion and mesh builders |
| `ui.rs` | egui side panel (`ui_panel`) |

### Cross-platform asset loading (`loading.rs`)

Baked layers load through Bevy's `AssetServer`, so the same code path reads `assets/` natively and fetches over HTTP on web. Each layer is a transparent newtype `Asset` wrapper (via the `postcard_asset!` macro) around a `sim-core` type, with a **distinct extension** (`.osgraph`/`.oscam`/`.osace`/`.osheat`/`.osequity`/`.osfield`/`.osalpr`/`.osdot`). `PostcardLoader<A>` is a generic `AssetLoader` whose `extensions()` returns the single extension it was constructed with — load-bearing, because `CamerasRes`, `AlprRes`, and `DotRes` all share the inner type `FixedSensorLayer` and a shared extension would silently mis-decode.

Flow: `start_loading` (Startup) spawns the camera and inserts `LoadingHandles` (one handle per layer + `built: false`); `build_world` (Update) returns early until **all eight** assets resolve, then builds the `Sim` resource and spawns the static map meshes once (guarded by `built`).

### ECS resources, components, systems

**Resources:** `Sim` (graph, sensors, layer, ace_segments/routes, heatmap, equity, `equity_corr` = Pearson r diversity↔cameras, dashcam_field), `RouteState`, `Params` (toggles, `departure_hour=17.0`, `dashcam_penetration=0.40`, `glasses_per_1000=10.0`, `heatmap_class`, `mode`), `WalkshedState`, `WalkLive` (`seen`, `count`, `last_progress`), `EguiWants` (pointer/keyboard), `DragState` (click-vs-drag), `ResetRequested`, `LoadingHandles`.

**Components:** `BaseMap`, `FovWedge`, `AceVis`, `HeatmapVis`, `EquityVis`, `RouteVis`, `WalkshedVis`, `CameraDot { id, flash }`, `Walker { progress_m }`.

**Systems** (Update): `build_world`, `camera_control`, `handle_click`, `recompute_on_change`, `animate_walker`, `walk_capture_events`, `camera_flash_decay`, `sync_mode`, `sync_visibility`, `rebuild_heatmap`, `rebuild_equity`, `apply_reset`, `smoke_exit` (a headless-CI helper gated on `OURSPACE_SMOKE`). `ui::ui_panel` runs in `EguiPrimaryContextPass`.

### The two modes (`Params.mode`)

**Route mode** (`handle_click`): first click sets A (lichen), second sets B (terracotta) and calls `run_route(...)`, spawning a `LineStrip` route + a `Walker`. `animate_walker` advances the walker at `WALK_SPEED` and loops; `walk_capture_events` tests every `CameraDot` against its sensor wedge with `captures`, increments `WalkLive.count` and sets `flash = 1.0` the first time each camera is seen this pass; `camera_flash_decay` decays the pulse. `recompute_on_change` re-runs `summarize` whenever `Params` changes (sliders/hour re-evaluate without re-routing).

**Walkshed mode**: one click snaps to the nearest node, runs `graph.walkshed(node, WALKSHED_SECONDS = 600.0, WALK_SPEED)` then `walkshed_exposure(...)`. Reachable streets light up warm gold, in-shed cameras get cold emphasis rings, and the click point gets a center marker; the result lands in `WalkshedState`.

### Ambient moving agents & dual-mode exposure (`agents.rs`)

The mobile sensing classes are also rendered as moving agents: clay **dashcam vehicles** (Tier C) replaying baked `VehicleRoute`s, and slate **glasses pedestrians** (Tier D) following on-device graph **random walks** (`StreetGraph::random_walk_route`). A fixed entity **pool** (`AgentPool`, `MAX_VEHICLES = 250` + `MAX_PEDS = 400`) is spawned once with one shared mesh+material per class; `scale_agent_population` activates/deactivates slots (≤32/frame) toward a target = `MAX × {traffic,pedestrian}_multiplier(hour) × (slider / default)`, so on-screen density tracks the hour and the dashcam/glasses sliders. `animate_agents` advances each active agent (`speed × ANIM_SPEEDUP × dt`, the same time-lapse clock as the walker), sets its `Transform` via `Route::position_at` (O(log n)), orients vehicles by `Route::heading_at`, and **recycles in place** on completion (vehicles re-sample the weighted pool; peds random-walk on from the nearest node) — no spawn/despawn churn. There is **no runtime A***: vehicle paths are baked offline, ped paths are O(1) random walks. Agents are hidden in heatmap mode / when `show_agents` is off, and inactive agents early-out of every system.

`Params.exposure_mode` selects what the panel headlines:
- **Analytical** (default, citable): the deterministic `summarize` Poisson estimate — unchanged.
- **Narrative**: `mobile_capture_events` (chained after `walk_capture_events`) tests the walker's animated position against each active agent's capture range; on a debounced fresh entry it rolls a Bernoulli folding the device∧capture probability (`penetration × capture_prob` for dashcams; `(per_1000/1000) × p_recording × capture_prob` for glasses) and increments a per-class live tally with a pulse. Because agent flux already scales with the same multipliers/sliders the analytical rate uses, the tally is a Monte-Carlo sample whose expectation tracks the analytical figure; the UI labels it as such and keeps Analytical as the reproducible number.

### Rendering (`world.rs` + `main.rs`)

Everything is 2D `Mesh2d` + `ColorMaterial` under one `Camera2d`. **1 world unit = 1 ENU meter** (`world::to_world`); z orders layers.

| Element | Topology / mesh | Builder |
|---------|-----------------|---------|
| Streets | `LineList` | `line_list_mesh(street_line_positions(...))` |
| ACE corridors | `LineList` | `line_list_mesh` |
| CCTV markers | `Circle::new(11.0)` | — |
| ALPR markers | `Rectangle::new(17.0, 17.0)` | — (shape+hue redundancy) |
| DOT traffic-cam markers | `RegularPolygon::new(12.0, 3)` | — (cold triangle; shape+hue redundancy) |
| FOV wedges | `TriangleList` fan | `wedge_mesh(heading, half_fov, range, 16)` — **directional sensors only** (omnidirectional cams draw no cone) |
| Route line | `LineStrip` | `line_strip_mesh(&r.points, 2.0)` |
| Walker / markers | `Circle` | — |
| Dashcam-vehicle agents | `RegularPolygon::new(7.0, 3)` clay | — (oriented by `heading_at`; one shared mesh, batched) |
| Glasses-pedestrian agents | `Circle::new(4.0)` slate | — (one shared mesh, batched) |
| Heatmap | `LineList` per bucket | `line_list_mesh` (6 intensity buckets) |
| Equity choropleth | `TriangleList` | `filled_polygon_mesh` (earcut-triangulated) |

`wedge_mesh` builds a triangle fan from an apex over `segments+1` rim points (compass-bearing math). `filled_polygon_mesh` triangulates the exterior ring with `earcutr::earcut` (handles concave block groups). `rebuild_heatmap` floors `norm·6` into 6 buckets and builds one `LineList` mesh per bucket; `rebuild_equity` lerps each block group between washed-clay and lichen by `entropy / MAX_ENTROPY` (`= ln 5 ≈ 1.609`). In heatmap mode, `sync_visibility` hides the base map so the overlay reads cleanly.

### Interaction (`camera_control`)

- **Pan**: left- or right-drag via `CursorMoved` deltas (works on web without pointer-lock). A **6 px** threshold distinguishes click from drag (`DragState.moved_px`), so dragging never drops a point.
- **Zoom**: `MouseScrollUnit::Line` taken as-is; `Pixel` (trackpads) divided by `ZOOM_PIXEL_DIVISOR = 160.0`. Factor = `(-notches · ZOOM_PER_NOTCH).exp()` (`ZOOM_PER_NOTCH = 0.06`), per-frame clamped to `0.86..=1.16`, scale clamped to `ZOOM_MIN = 0.4 .. ZOOM_MAX = 30.0`.
- **Keyboard pan**: WASD / arrows at `700 · scale.x · delta_secs`.
- **egui yield**: map controls early-out when `EguiWants.pointer/.keyboard` are set (updated each frame from `ctx.wants_pointer_input()` / `is_pointer_over_area()` / `wants_keyboard_input()`).

### Per-target Bevy features (`Cargo.toml`)

The base feature set is in `[dependencies.bevy]` (not only target tables) so Bevy's derive macros resolve crate paths; Cargo unions it with per-target rows.

| Scope | Bevy features | Extra crates |
|-------|---------------|--------------|
| Base | `std`, `bevy_winit`, `bevy_window`, `bevy_render`, `bevy_core_pipeline`, `bevy_sprite`, `bevy_color`, `bevy_asset`, `bevy_log`, `async_executor` | `sim-core`, `bevy_egui`, `bevy_prototype_lyon`, `earcutr`, `serde`, `postcard`, `anyhow` |
| `cfg(not(wasm32))` | `x11`, `wayland`, `multi_threaded` | — |
| `cfg(wasm32)` | `webgpu`, `web` | `wasm-bindgen 0.2`, `console_error_panic_hook 0.1` |

### Web build (`web/build.sh` + `web/index.html`)

A static, backend-free pipeline:

1. `cargo build -p app-interactive --profile wasm-release --target wasm32-unknown-unknown`. `wasm-release` inherits `release` + `opt-level = "z"`, `lto = "fat"`, `codegen-units = 1`, `strip = true`, `panic = "abort"`.
2. `wasm-bindgen --target web --no-typescript --out-name app-interactive` into `web/dist`.
3. `wasm-opt -Oz` with the toolchain's required wasm features enabled (`--enable-bulk-memory` etc.), overwriting `app-interactive_bg.wasm`.
4. Copy `web/index.html` and `crates/app-interactive/assets` into `web/dist`.

Prereqs (in-script): `rustup target add wasm32-unknown-unknown`, `cargo install wasm-bindgen-cli --version 0.2.125` (matched to the crate), `brew install binaryen`. The optimized wasm is ~21 MB; total payload ~44 MB with assets.

`web/index.html` is a self-contained "field-journal" page that: hosts the `#bevy-canvas` + a self-inking compass loading overlay; performs **WebGPU detection** (`'gpu' in navigator`) with a graceful fallback message; dynamically imports `./app-interactive.js` and calls its default init; and carries the A–D confidence chips + full provenance line. No backend.

### Privacy

The route is computed entirely client-side. Once the wasm bundle + baked assets are fetched, all routing/exposure runs in the browser via `sim_core` — there is no server round-trip for a walk and nothing about A/B/the route is transmitted. The UI states this explicitly ("Your route stays on this machine.").
