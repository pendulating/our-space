# Scaling to all five boroughs

A plan for taking **our·space** from Manhattan to all of NYC, and the
performance instrumentation to put in place *first* so we scale on measurements,
not vibes.

This is a planning document, not a spec — it states the bottlenecks, a phased
path, and the monitoring to build before we start. Numbers are the current
Manhattan baseline (from the boot log + `web/dist/assets/processed/`).

---

## 1. Where Manhattan sits today (the baseline)

| Axis | Manhattan now | Notes |
|---|---|---|
| Street graph | 6,762 nodes / 9,994 edges (`graph_manhattan.osgraph`, 952 KB) | drives routing + walkshed |
| Fixed sensors | 4,590 camera nodes (4,312 CCTV + 284 ALPR + 352 DOT + 499 enforcement) | merged into one quad mesh per layer |
| ACE corridors | 14,849 segments / 20 routes (558 KB) | |
| Real-day trips | 5,163 bus trips (1.2 MB) + 164,184 taxi trips / 3,858 pooled routes (3.6 MB) | replayed, not simulated |
| Footprints | `footprints.osbldg` 4.8 MB | the single biggest asset |
| Exposure field | ~45 m cells, grid clamped to **280 × 360 ≈ 100 k cells** | `init_heat_build` in `main.rs` |
| Robotability / Tesla fields | 104 × 213 grid; 262 ZIP zones | |
| Agent on-screen caps | 4,000 vehicles · 400 peds · 300 buses · 200 robots · 400 Teslas | `MAX_*` in `agents.rs`; over-cap sets are subsampled |
| Bundle | **38 MB** `web/dist` (23 MB WASM) | first-load download |

Two facts shape everything below:

- **Several costs are already extent-bounded, not count-bounded.** The exposure
  grid clamps its cell *count* (cells just grow to ~107 m at citywide extent),
  and the agent layers are hard-capped and subsampled. These degrade *gracefully*
  with area — they get coarser/sparser, they don't blow up.
- **The data-volume costs are not bounded.** Footprints, the street graph, the
  TLC trip tables, and the camera census all grow with the city. These are the
  real scaling work.

NYC vs. Manhattan, roughly: **~13× land area** (59 → 778 km²), **~5.5× population**
(1.6 → 8.8 M), and street mileage concentrated in the outer boroughs (Queens +
Brooklyn dwarf Manhattan). Expect **~10–15× graph and footprint volume** and
**~5–10× the camera census** (Amnesty *Decode Surveillance NYC* already covers all
five boroughs; we currently clip it to Manhattan).

---

## 2. The bottlenecks, ranked

1. **Download weight (highest risk).** Naively re-baking everything for five
   boroughs pushes the bundle from 38 MB toward **300–500 MB** — footprints alone
   could hit ~50 MB. A half-GB first load is a non-starter on the web. *This is the
   gating constraint.*
2. **Footprints + landmarks** — geometry-heavy, mostly off-screen at any zoom.
3. **Street graph & routing.** Dijkstra walkshed/route over a ~100 k-node graph is
   noticeably slower than over 6.7 k; the walkshed flood especially.
4. **Click hit-testing.** ALPR/CCTV picking is currently a linear scan over the
   directory (`handle_click`). At ~40 k cameras that's a per-click sweep worth
   indexing.
5. **Exposure / choropleth recompute.** Bounded by the grid clamp, but a citywide
   neighborhood set (Manhattan has ~40 nbhds; NYC has ~300) makes the choropleth
   rebuild + per-frame relabel/declutter loops ~7× longer.
6. **Memory / GPU.** More merged-quad vertices, more footprint triangles, larger
   textures held resident.

Agents are deliberately *absent* from this list: the caps mean five boroughs
render the same agent budget, just spread thinner. The work there is making them
*spawn where the camera is looking* (Phase 3), not making them faster.

---

## 3. Build the instrumentation first (Phase 0)

Do not scale anything until we can see frame time, asset weight, and entity counts
on demand. None of this ships to end users — it's gated behind a debug flag.

- **In-app perf HUD** (`?debug=perf` query param, read in `main.rs` like the
  existing `?story=` deep-link). Add Bevy's `FrameTimeDiagnosticsPlugin` and draw a
  small egui overlay: FPS, frame-time p50/p95, entity count, visible-mesh count,
  and (web) `performance.memory.usedJSHeapSize`. Log the same line to the console
  each second so the inspector can scrape it.
- **Frame-time capture in the inspector.** `tools/inspect` already drives real
  Chrome and notes that `--wheel` works. Extend `inspect.mjs` with a
  `--profile <seconds>` mode that samples `requestAnimationFrame` deltas and emits
  a JSON report (p50/p95/p99, long-frame count). Run it across a **matrix of
  presets** — `{borough} × {overview, mid, street} × {layer on/off}` — via
  `?story=` deep-links that set camera + layers without a click (clicks don't reach
  winit headless; see the inspector README).
- **Bundle-size budget in `build.sh`.** After the copy step, sum `web/dist` and
  **fail the build if it exceeds a budget** (start at 45 MB; raise deliberately per
  borough). Print the per-asset breakdown so regressions are obvious.
- **Baseline now.** Capture the Manhattan matrix and commit it as
  `docs/perf-baseline.json`. Every later phase is judged against it: **60 fps
  (16.6 ms) p95 at every preset, first-load under a stated budget.**

Targets to hold as we add boroughs: **p95 ≤ 16.6 ms** at overview/mid/street;
**interaction latency** (walkshed/route/pick) **< 150 ms**; **first-load transfer**
under the per-deployment budget.

---

## 4. Phased rollout

### Phase 1 — Partition the data pipeline by borough
The pipeline is already borough-aware: every `bake-*` command takes an optional
Manhattan-clip GeoJSON (`args.get(..)` in `data-pipeline/main.rs`), and
`bake-borough` exists. Generalize the clip from "Manhattan" to **any borough
boundary**, and bake **per-borough asset sets** plus a small **manifest**
(`assets/processed/manifest.json`: borough → file list + byte sizes + bbox). One
clip in, N borough bundles out. No renderer changes yet — prove the data first.

### Phase 2 — Load on demand, not all up front
Today every asset loads at boot. Instead:
- Load the **borough(s) overlapping the viewport** from the manifest; fetch the
  rest lazily as the camera moves (the loading system already despawns/rebuilds, so
  this is an extension of existing lifecycle, not new infrastructure).
- **LOD the heavy geometry.** Footprints + landmarks get a simplified far-zoom
  bake (fewer vertices, or drop below a zoom floor — we already zoom-floor markers
  and ACE ribbons; reuse the pattern). Footprints are the first target.
- Keep the **TLC trip tables server-side or pre-aggregated** — don't ship 5×
  164 k trips to the browser. The real-day replay can run off a per-borough
  sampled subset sized to the agent caps.

### Phase 3 — Spatial indices + viewport culling
- Put fixed sensors and neighborhoods in a **uniform grid / quadtree** so
  `handle_click` hit-tests O(local) instead of O(all), and so rendering can cull
  off-screen merged-quad batches.
- Make agent spawning **viewport-aware**: admit trips whose current position is
  near the camera, so the capped budget is spent on what's visible rather than
  spread across 778 km².

### Phase 4 — Routing at scale
- Bound the **walkshed flood** to a local subgraph (it's a 10-minute radius — clip
  the graph to a bbox around the origin before flooding).
- For long A→B routes across the whole city, evaluate **contraction hierarchies or
  a precomputed overlay** only if Phase-0 numbers show plain Dijkstra missing the
  150 ms interaction budget. Don't pre-optimize routing before the HUD says so.

### Phase 5 — Ship borough-by-borough
Add Brooklyn → Queens → Bronx → Staten Island incrementally, re-running the perf
matrix at each step and holding the budgets. Ship a borough only when it's green.

---

## 5. Risks & decisions to make early

- **Web delivery model.** On-demand fetch implies the assets live at stable URLs
  (GitHub Pages is fine for static per-borough files; a tiles/range-request server
  is only needed if Phase-2 LOD isn't enough). Decide before Phase 2.
- **Trip data.** The honest "real-day" framing depends on real TLC trips. Decide
  whether the citywide version ships pre-aggregated replay tracks (smaller, still
  honest) vs. raw trips (heavy). Pre-aggregation is the likely answer.
- **The Manhattan-only narrative.** The headline ("How watched is a place in
  Manhattan?") and several copy/landmark choices are Manhattan-specific; a citywide
  build needs a borough-aware framing pass, not just more data.

**First concrete step:** build the Phase-0 instrumentation (perf HUD + inspector
`--profile` + bundle budget) and commit `docs/perf-baseline.json`. Everything else
is judged against that baseline.

---

## 6. MVP: a static-first five-borough build (in progress)

An alternative sequencing that front-loads the cheapest signal: **stand up all five
boroughs with only the static layers first, measure on-device + download weight, and
add the dynamic layers only once that's proven performant.** ACE bus trips are the
one dynamic layer carried into this MVP (sparse vs. taxis). This inverts Phase 1/2's
"partition + lazy-load" — we ship one citywide bundle and *measure* before investing
in per-borough manifests/LOD, on the bet that the static census is small.

That bet is now measured and **held**: the five-borough static core is ~2 MB.

### Mechanism
- `data-pipeline/src/extent.rs` — an `Extent {Manhattan, Nyc}` threaded through the
  fixed-camera bakes (`amnesty`/`dot`/`cameras_dahir`) and the ACE bakes
  (`ace`/`bus_day`). It carries the lat/lon bbox + borough-name filter (replacing the
  hardcoded Manhattan ones) and `route_base` (case-normalized ACE route matching —
  GTFS spells the Bronx `Bx41`, the ACE list `BX41`). Default stays Manhattan; pass
  `nyc` to opt a bake in.
- `bake-borough nyc` emits all five main-landmass rings in **one** `BoroughOutline`.
  The app already clips cameras to the borough rings at world-build via
  `in_manhattan(p, rings)` = point-in-**any**-ring, so a five-ring outline un-clips
  the citywide census for free.
- The app selects the citywide asset set + framing at runtime via `?city=nyc` (web)
  / `OURSPACE_CITY=nyc` (native), opens on the all-borough choropleth, and drops the
  Manhattan-only dynamic agent layers (`CityScope` resource, `start_loading`).
- `tools/bake_citywide.sh` reproduces the static set into `assets/processed/*_nyc.*`.

### Measured (Manhattan → NYC, baked asset sizes)
| Layer | Manhattan | NYC (5 boroughs) | Size |
|---|---|---|---|
| Fixed CCTV (Amnesty+Dahir) | 4,422 cams | 29,207 | 99 KB → 623 KB |
| DOT traffic cams | 370 | 958 | 7 KB → 18 KB |
| Borough outline | 1 ring | 5 rings | 77 KB → 783 KB |
| Neighborhoods | — | 312 polys (38 Manhattan) | 534 KB (already citywide) |

The citywide camera census + boundary core ≈ **2 MB** — confirming §2's call that
*download weight is gated by footprints, not the census.* Renders cleanly on an M2
Max (29,427 distinct camera nodes after co-location merge); boots in the same
pipeline as Manhattan.

### Data-acquisition gaps surfaced
- ~~**`gtfs_m` is Manhattan-only**~~ **→ DONE (citywide ACE buses below).**
- **Footprints** citywide = the NYC Building Footprints GeoJSON (5zhs-2jue, ~856 MB,
  ~1.08 M buildings) — *not* the CityGML (`DA_WISE_GML.zip` is the 14 GB 3D model, for
  landmarks). Naive citywide bake ≈ 120 MB; RDP-simplified (@ 1 m, ≥ 18 m²) ≈ 51 MB
  — still far past a flat web budget. **Solved by lazy per-borough loading** (below).
- **ALPR / enforcement / LinkNYC** raw snapshots are Manhattan-only (small re-fetch).
- **OSM street graph** is Manhattan-only — needed only for routing/walkshed/
  robotability/taxi, all deferred, so the static+buses MVP needs no citywide OSM.
  Routing modes stay Manhattan-gated in the citywide build for now.

### Lazy per-borough footprints (Phase-2, built)
The ~51 MB of citywide building fabric never enters first-load. `footprints::bake`
gained RDP simplification (~44% fewer vertices, imperceptible at footprint zoom) +
a `[borough]` BIN filter, baking five per-borough assets (Manhattan 2.8 / Bronx 5.3 /
Brooklyn 15 / Queens 21 / Staten Island 6.6 MB). In the app,
`manage_footprint_regions` (`CityScope`-gated) watches the camera viewport + a
zoom-floor (`FOOTPRINT_ZOOM_FLOOR_MPP = 8 m/px`): at the city overview **no region
loads**; zoom into a borough and its footprints stream in + build a mesh; zoom back
out and they're freed. First-load stays ~38 MB; a borough's fabric is fetched once,
on demand. (Simplification also shrank the Manhattan build's eager footprints 5 → 2.8
MB.) Next refinement if per-borough fetches feel heavy: sub-borough tiling so only the
visible tile loads, not the whole 21 MB of Queens.

### Citywide ACE buses (built) — the one dynamic layer in the MVP
`gtfs_m` is Manhattan-only (its trips/shapes are all `M` routes despite a full
295-route catalog), so citywide ACE buses needed all five borough feeds. The live MTA
feeds only serve the current board, so all five were re-fetched together
(`gtfs_{m,b,q,bx,si}.zip`) and merged into one dir (identical CSV schemas → a safe
raw concat; depot-prefixed service_ids don't collide; readers made `flexible(true)`
for MTA's occasional unquoted-comma route names). `extent.route_base` (case-normalized
— GTFS `Bx41` vs ACE-list `BX41`) selects all-borough ACE routes; no boundary clip
citywide. Result: **66 ACE routes / 80,004 corridor segments (3.1 MB)** + **17,780 bus
trips / 304 shapes (5.2 MB)**, vs Manhattan's 20 routes / 5,163 trips. The catch: the
current board doesn't cover the baked real-day date (2026-04-21), so the citywide
build runs a current-board Tuesday (2026-07-07) — fine because taxi is off citywide,
so there's no cross-layer date conflict (the Manhattan build keeps 04-21). In the app
the citywide build sets `ace_on` + swaps to the `*_nyc` ACE/bus assets + sets its own
`SimDate`; up to 300 buses replay live across the city (`MAX_BUSES` cap, subsampled).

### Citywide street network (built) — from NYC CSCL, not Overpass
The Manhattan graph is an Overpass walk dump, but a single Overpass query for all of
NYC overwhelms the public instances (they time out / 406). NYC's own **LION/CSCL
street centerline** (Socrata `inkn-q76z`, 122 k segments) is the authoritative
five-borough network — already split at intersections, with no out-of-city spillover —
so the citywide graph is baked from it (`bake-graph --cscl`). Keep-set `rw_type`
{street, highway, bridge, tunnel, alley}; each segment becomes an edge, shared
intersection endpoints snap to a 1 m grid, and the largest connected component is kept.
**Crucially the drive-ish network (not walk) is used**: bridges and tunnels keep all
five boroughs in one component — including Staten Island via the Verrazzano, which a
pedestrian-only network would strand (no walk crossing). Result: **71,483 nodes /
109,195 edges, 7.8 MB**, spanning lat 40.50–40.92 / lon −74.26–−73.70. The app swaps
`GRAPH_PATH_NYC` in for the citywide build; the exposure grid stays bounded (it clamps
to ≤280×360 cells and just coarsens), and buses/taxi don't index the graph, so the
swap is safe.

### On-the-fly NYCOpenData APIs — spike findings (2026-06-25)

The dynamic layers (esp. taxi) and the footprint fabric are heavy because we ship
pre-baked tables. The alternative for scale-on-GitHub-Pages: **fetch from NYC
OpenData (Socrata) at runtime, per viewport**, instead of baking everything. This
breaks the current "no live data fetching at runtime" invariant (see
ARCHITECTURE.md), so the spike's job was to de-risk it empirically before we commit.

**Feasibility: confirmed.** Tested against dataset `5zhs-2jue` (Building Footprints,
~1.08 M features) with Socrata's SODA API:

| Property | Result |
| --- | --- |
| CORS | `Access-Control-Allow-Origin: *` — works cross-origin from GitHub Pages |
| Real-browser fetch | Chrome `fetch()` of a street-level viewport: **434 buildings, 1.25 s** end-to-end |
| Spatial query | `$where=within_box(the_geom, NW_lat, NW_lon, SE_lat, SE_lon)` — server-side bbox, no full-table scan |
| Geometry-only | `$select=the_geom` drops 15 property fields → roughly halves raw bytes |
| Compression | `Content-Encoding: gzip` (browsers always send `Accept-Encoding: gzip`, so this is automatic) |

**Measured transfer (geometry-only + gzip, what the browser actually receives):**

| Viewport | Buildings | Raw | **Gzipped (on the wire)** | Latency |
| --- | --- | --- | --- | --- |
| ~600 m (street level, where footprints render) | 434 | 171 KB | **38 KB** | ~1.25 s |
| ~1.5 km (dense downtown Brooklyn) | 3,476 | 1.46 MB | **334 KB** | ~2.0 s |

So a footprint viewport costs **tens to a few hundred KB on the wire** — comparable to
a single map image — versus the **~51 MB** baked per-borough fabric. This eliminates
both the first-load cost *and* the tracked-repo-size hit, and the existing lazy
`manage_footprint_regions` system (viewport + zoom-floor) is the natural integration
point: swap the baked per-borough asset source for a per-viewport Socrata query.

**Per-layer offload assessment:**

| Layer | Baked size | Offload? | Notes |
| --- | --- | --- | --- |
| **Footprints** | ~51 MB | **Yes — strong** | Geometry-only spatial query, ~38–334 KB/viewport gzipped; clean fit to the lazy loader; kills the repo-size concern. The prime candidate. |
| Taxi / HVFHV | (deferred) | Harder | TLC records are huge and O–D structured (not a point-in-box query for the routed-replay model); needs aggregation/sampling server-side. |
| ACE buses (static) | 8.3 MB | No | GTFS static isn't a spatial query. *But* MTA **GTFS-realtime** (live positions) is a separate, compelling offload → truly live buses; needs an API key + protobuf parsing + CORS check. Future opportunity. |
| Cameras (CCTV/ALPR/DOT) | ~0.6 MB | No | Tiny already, and most aren't on Socrata (Amnesty/Dahir/DeFlock are GitHub/Stanford/OSM). Keep baked. |
| Neighborhoods / outlines / heatmap | small | No | Static, small, keep baked. |

**Recommended architecture (footprints):** in the citywide build, replace the baked
per-borough `.osbldg` source inside `manage_footprint_regions` with a viewport-bbox
query — `$select=the_geom&$where=within_box(...)&$limit=N` — issued via WASM `fetch`
(`gloo-net`/`web-sys`) on a debounced "viewport settled" event. Parse GeoJSON in-app,
reuse the existing ENU projection + RDP simplification, cache fetched tiles (don't
re-fetch visited areas), and show a brief loading state. Add a Socrata **app token**
(higher rate limits; the unauthenticated tier throttles aggressively) and keep the
baked assets as a `file://`/offline fallback.

**Tradeoffs (deliberate):** breaks the "no live fetch" invariant; ~1–2 s first-paint
latency per new area (mitigated by debounce + cache + loading state); depends on
Socrata uptime/CORS; won't work offline or from `file://`. Net: for footprints the win
(−51 MB baked, −48 MB tracked repo) clearly outweighs these.

**Verdict: build the footprints offload.** It's the single biggest data cost, the
feasibility is proven end-to-end in a real browser, and it drops cleanly into the lazy
loader we already have. The WASM async-fetch path (fetch → channel → parse → mesh) is
the one real piece of new engineering. GTFS-realtime buses are a strong follow-on for
genuinely live motion. Taxi/HVFHV remains the hardest (volume + O–D shape) and stays
deferred.
