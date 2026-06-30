# A probabilistic trip model for realistic dashcam coverage

> **Status:** **built (June 2026)** — Stages 1–3 (drive graph + speed-aware
> routing, within-zone dasymetric endpoints, time/distance route inference) are
> implemented and verified, and Stage 4's analytic sensing-power metric is computed,
> validated, and surfaced as the coverage headline. Supersedes the zone-centroid
> routing described in [`TAXI_GENERATION.md`](TAXI_GENERATION.md) §2–3.

## Goal

Make the **spatiotemporal coverage** of the roving dashcam fleet (rideshare +
ACE buses) realistic. Not point accuracy — we will never know a real curb — but
the *aggregate* "which streets are watched, how often, at what hours" should
reflect how cars actually move through Manhattan. The
[roving-coverage overlay](../crates/app-interactive/src/coverage.rs) is the
consumer: it lights up street segments by how often a camera vehicle crosses
them, so it is only as honest as the routes underneath it.

## Why the current model falls short

Today every taxi trip is collapsed to a single deterministic route
([`taxi_day.rs`](../crates/data-pipeline/src/taxi_day.rs)):

1. **Origin/destination** = the trip's TLC taxi *zone* (≈69 in Manhattan; no
   coordinates exist in modern HVFHV data) → the zone's **area-weighted
   centroid** → snapped to the **nearest walk-graph node**. One fixed node per
   zone.
2. **Route** = a single **A\* shortest-*distance* path over the pedestrian walk
   graph** between those two nodes. One route per (PU, DO) pair, shared by every
   trip on that pair.

Three consequences that distort coverage:

- **No within-zone variation.** Every trip leaving a zone starts at the exact
  same node, so coverage funnels onto a handful of centroid-to-centroid spines
  instead of spreading across the zone the way real pickups do.
- **No highways.** Routing is on the *walk* graph, which by construction omits
  `motorway` (you can't walk the FDR). `is_kept_highway`
  ([`graph_osm.rs`](../crates/data-pipeline/src/graph_osm.rs)) keeps up to
  `trunk` but not `motorway`/`motorway_link`, so the **FDR Drive, Henry Hudson
  Parkway, and upper West Side Highway are not in the Manhattan graph at all.**
  A trip that really took the FDR gets drawn snaking through surface streets.
- **Speed limits shape *pace*, not *path*.** We **do** already apply NYC's
  25 mph default limit — but as a single uniform constant
  (`NYC_SPEED_LIMIT_MPS`, [`main.rs:53`](../crates/app-interactive/src/main.rs))
  that [`PaceProfile`](../crates/sim-core/src/graph.rs) uses to cap cruise speed
  and brake into corners *while replaying an already-chosen route*
  ([`main.rs:2061`](../crates/app-interactive/src/main.rs),
  [`agents.rs:772`](../crates/app-interactive/src/agents.rs)). It is **not**
  per-edge and it **never enters routing**: the path is still A\*
  shortest-*distance* on the walk graph (cost = `length_m`). So the limit cannot
  reveal highway use — there are no faster highway edges for fast trips to prefer,
  and a real FDR trip is routed over surface streets, then (because its data is
  faster than 25 mph) replayed *faithfully* along that wrong path.

The net effect: the coverage overlay over-concentrates on avenues and never
shows the riverside highways, which in reality carry a large share of long
trips. **The fix is to make speed *per-edge* and to put it in routing** — exactly
Layer 2 below.

## The data we actually have (and aren't using)

The HVFHV records expose **`trip_miles` and `trip_time` per trip** — the routed
distance and the door-to-door duration. **As of Stage 3 the aggregation keeps
both** (`pu_min, PULocationID, DOLocationID, dur_min, trip_miles, trip_time`);
the trip CSV was re-extracted from `fhvhv_tripdata_2024-06` for Tuesday
2024-06-25, Manhattan↔Manhattan. Those two numbers are the key to route
inference: together with the per-edge speed priors they tell us *which* path a
trip plausibly took (see Layer 2 and Stage 3).

## Proposed model — three layers

### Layer 1 — within-zone endpoints (dasymetric, hierarchical)

Disaggregate each zone into a realistic distribution of likely endpoints, and
distinguish **concentrated** zones (a hub: an airport terminal, Penn Station, a
single hospital) from **dispersed** zones (residential, endpoints spread across
the blocks).

Per zone, build two weight surfaces, rasterized to ~30 m cells and clipped to
the zone polygon:

- **Pickup surface** ∝ jobs (LEHD LODES WAC) + POIs (OSM: transit entrances,
  hotels, nightlife, hospitals, venues) + building mass.
- **Dropoff surface** ∝ residential population (census blocks) + building mass.

(Pickups skew to commercial/activity; dropoffs skew residential — a documented
asymmetry.) This is **dasymetric mapping**: spread a coarse count onto sub-units
using ancillary "where can it actually be" layers. Reference implementation:
[`pysal/tobler`](https://github.com/pysal/tobler) (BSD-3) — simple enough to
mirror in Rust.

Tag each zone **concentrated vs dispersed** with a normalized-entropy (or
Gini/HHI) score over its surface. Sampling per trip:

- **dispersed** → draw a cell via `rand::WeightedIndex`, jitter uniformly inside
  it, reject if outside the polygon (`georust/geo` point-in-polygon).
- **concentrated** → pick a top-weight node (hub), jitter with a tight Gaussian.

> **Decision (June 2026):** start **dasymetric-only** (buildings + POIs + jobs +
> population). It needs no heavy download, covers every zone, and is appropriate
> for FHV. A later refinement can blend in an **empirical prior** learned from
> the pre-2016 TLC data that *did* carry lat/lon — strong for concentrated-vs-
> dispersed *shape*, but it's yellow-cab, not FHV, so it should blend toward the
> dasymetric surface rather than be trusted absolutely.

### Layer 2 — probabilistic route (speed-limit priors)

Build a **drive graph** with speed limits, then infer the most probable route
per trip from its observed time and distance.

**Drive graph.** Add `motorway`/`motorway_link` to the drive keep-set and bake
from a drive-network OSM dump (the walk dump omits motorways), honoring `oneway`
and turn restrictions. Per edge, a **free-flow speed** from OSM `maxspeed`,
falling back to a highway-class default (NYC default **25 mph** surface;
`trunk`/`motorway` get arterial/highway speeds — FDR ≈ 40, West Side Hwy ≈ 30,
Henry Hudson higher). Patch OSM gaps with
[NYC VZV Speed Limits](https://data.cityofnewyork.us/Transportation/VZV-Speed-Limits/7n5j-865y).
Edge cost = `length_m / speed` = **free-flow seconds**. *(The citywide CSCL
graph already includes highways via `rw_type` 2; only the Manhattan Overpass
build needs the motorway fix.)*

**Route inference.** Per trip with origin `o`, destination `d`, observed time
`T`, distance `D`: generate `K` candidate routes via **k-shortest-paths** on the
free-flow-time graph, and score each route `r` (length `L_r`, free-flow time
`t_r`):

```
P(r | o,d,T,D)  ∝  prior(r) · Normal(L_r ; D, σ_D²) · g(T / t_r)
```

- **prior(r)** — path-size logit favoring short/fast routes, with an overlap
  correction so near-duplicate candidates don't dominate.
- **Normal(L_r; D)** — distance likelihood; prunes candidates whose length is
  inconsistent with the trip's `trip_miles`.
- **g(T / t_r)** — the load-bearing term. The slowdown `s = T / t_r` is **≥ 1**
  (you cannot beat the posted limit; traffic only slows you). `g` puts mass at
  `s ≳ 1` (typical congestion) and **penalizes `s < 1` sharply.**

The MAP route is `argmax_r`; the normalized scores give a route *distribution*
(soft highway attribution). **The highway-reveal falls straight out:** a fast
trip's observed `T` is physically impossible on 25 mph surface streets
(`s < 1`), so the only candidates that survive are those using the FDR / West
Side Hwy, whose speed-limited free-flow time can actually reproduce `T`. No
special-casing.

Calibrate the congestion mean/spread of `g` (optionally per hour-of-day) once by
MLE over the trip set, so the time term is data-grounded rather than guessed.

### Layer 3 — coverage (the consumer)

The [roving-coverage overlay](../crates/app-interactive/src/coverage.rs)
accumulates over these varied, highway-aware routes → faithful spatiotemporal
coverage with no app change. Optionally add the **analytic sensing-power metric**
(O'Keeffe; below) as a citable headline and a validation cross-check.

## What to borrow

| Need | Use | Why |
|---|---|---|
| Coverage metric + validation | **O'Keeffe et al., PNAS 2019**, "sensing power of vehicle fleets" ([arxiv 1811.10744](https://arxiv.org/abs/1811.10744)); [lexparsimon tutorial](https://lexparsimon.github.io/sensingpower/) | Ball-in-bin "segment popularity" → closed-form coverage(N). Built on Manhattan. **Anchor: ~10 random taxis ≈ ⅓ of segments/day; ~30 → >50%.** |
| OD+time → route recipe | [Lab-Work/taxisim](https://github.com/Lab-Work/taxisim) | Recovers per-link times *and* most-likely paths from OD + total trip time — our exact inputs. |
| Within-zone disaggregation | [pysal/tobler](https://github.com/pysal/tobler) (BSD-3) | Reference dasymetric/areal-interpolation algorithms. |
| Rust routing/geo | [routx](https://github.com/mkuranowski/routx) (MIT), `georust/geo` + `geo-rasterize`, `rand::WeightedIndex` | OSM car routing + polygon clip/rasterize + weighted sampling, all Rust. |
| NYC ridesourcing coverage metrics | [Guo & Qian 2022](https://arxiv.org/abs/2207.11285) | Coverage **and reliability/refresh-time** definitions for a 20k-vehicle NYC fleet. |
| Hourly OD demand (optional) | [xinychen HVFHV tensor](https://spatiotemporal-data.github.io/NYC-mobility/rideshare/) | Ready 265×265×hour FHVHV counts. |

## Data to acquire

| Layer | Data | Source |
|---|---|---|
| trips | `trip_miles` + `trip_time` per trip | re-aggregate the HVFHV DuckDB fetch (one column) |
| drive graph | OSM drive extract incl. `motorway` | Overpass drive query (sibling to the walk dump) |
| speeds | OSM `maxspeed`; NYC VZV speed limits | OSM tags + [VZV open data](https://data.cityofnewyork.us/Transportation/VZV-Speed-Limits/7n5j-865y) |
| within-zone | building footprints, MapPLUTO land use, OSM POIs, LEHD LODES jobs, census population | NYC Open Data / Census / Overpass |

## Staging

1. **Drive graph + speed-aware fastest path.** ✅ **Done (June 2026).** Bake a
   Manhattan CSCL drive graph (704 highway edges incl. the FDR + ramps), carry
   `rw_type` in `segment_id`, derive per-class free-flow speed, and route taxis by
   free-flow time. The coverage overlay snaps to this drive graph. Verified: all
   3,858 O-D pairs route (0 no-path); 472/704 highway edges get covered over a day;
   the FDR / West Side Hwy / Henry Hudson read as continuous shoreline ribbons.
2. **Within-zone sampling.** ✅ **Done (June 2026).**
   - **2a — spread.** Each trip samples its O/D among **K=6 drive-graph nodes per
     zone** (point-in-polygon assignment), with a centroid route + global cap
     (12 000 routes) as fallback. Endpoints fan out to **427 distinct points** (was
     ~69 centroids); coverage grew **6,790 → 8,665 segments**, **472 → 633 highway**.
   - **2b — dasymetric weighting.** Endpoint *selection* (Efraimidis–Spirakis
     weighted reservoir, key `u^(1/w)`) and per-trip *sampling* are weighted by
     **building mass** (`sqrt(mass)+1`, from 45k footprints): concentrated zones
     pick clustered high-mass nodes, dispersed zones spread.
   - **Honest read of the Manhattan effect:** coverage extent barely moves
     (8,665 → 8,707) and route-level trip concentration is flat (Gini 0.438 →
     0.439) — that metric is dominated by O-D popularity, and Manhattan zones are
     uniformly dense, so the 6 per-zone endpoints carry similar mass. The clean
     signal is **highway-edge coverage falling 633 → 574**: mass pulls O/D toward
     dense inland blocks and away from low-mass shoreline/highway-adjacent nodes —
     pickups cluster at buildings, not on the FDR (and not in parks/water). Here the
     weighting is mostly a *realism* property; its concentrated-vs-dispersed
     contrast bites harder in mixed-density zones (a hub beside low-rise) than in
     dense Manhattan.
3. **Route inference from time + distance.** ✅ **Done (June 2026).** Re-extracted
   the Manhattan day (Tuesday **2024-06-25**, the same real day as the citywide
   build) with **`trip_miles` + `trip_time`** per trip. Per endpoint combo, a small
   **candidate set** — the fastest path and a **surface alternative** (highway/ramp
   edges penalized ×8) — is generated lazily and globally capped (14 000 routes).
   Each trip picks the **MAP** candidate under
   `Normal(L; D, (0.35·D)²) · lognormal(T/t_r; s0, τ)`, where `s0` (the typical
   slowdown, **2.4×** on this day) and `τ` are **calibrated from the long-trip
   data**, not guessed. *(A pragmatic 2-candidate set rather than full KSP — for the
   highway-vs-surface decision that dominates coverage, the fastest and the
   surface-only paths are the two that matter; KSP can be added later if finer route
   variety is wanted.)* **Verified:** among trips with both candidates, the inference
   sends the **fast** ones to the highway (avg **17.9 mph**, 4 182 trips) and keeps
   the **slow/congested** ones on the surface (avg **9.4 mph**, 3 860 trips) — an
   8.5 mph split, purely from observed time + distance. The FDR / West Side Hwy stay
   continuous ribbons but are no longer over-saturated: highway-edge coverage settles
   at **535/704** (vs Stage 2's pure-fastest 574–633, which forced *every* long trip
   onto the highway). The highway reveal is now **earned by the data**, not by
   shortest-time geometry alone.
4. **Analytic sensing power (O'Keeffe).** ✅ **Computed + validated (June 2026).**
   From the per-trip route→segment incidence, `q_i = (trips covering segment i)/M`,
   the bake reports the closed-form curve `C(N) = (1/S)·Σ_i[1−(1−q_i)^N]` over all
   `S = 11,820` drive segments. Heavy-tail saturation: N=10 trips → 3 %, 100 → 18 %,
   1000 → 41 %. **⅓ coverage at ~447 trips ≈ 17 FHV-vehicle-days** (O'Keeffe's
   anchor: ~10 random taxis ≈ ⅓) — order-of-magnitude consistent; ours runs higher
   because origins are zone-sampled (not GPS) and `S` counts highways/edges the
   fleet never touches. The full fleet's asymptotic **ceiling is 68 %**, which also
   explains why ½-coverage (~2,656 trips) is slower than the paper's ~30 taxis.
   **4b — UI headline ✅.** A compact `SensingPower` summary is baked into
   `TaxiDayLayer.sensing` (both Manhattan + citywide re-baked so both stay loadable)
   and shown in the roving-coverage panel: *"~17 camera-vehicles would watch a third
   of these streets in a day (the full fleet reaches 68%)."*

## Integration map (where it lands)

Stage 1 deliberately avoided an `EdgeData` format change (which would force
re-baking every graph): the CSCL **road class** (`rw_type`) is carried in the
existing `segment_id` field, and per-class free-flow speed is derived at route
time. The posted speed limit was later packed alongside it
(`segment_id = rw_type * 100 + posted_mph`, decoded by
`sim_core::graph::unpack_class`) — same no-format-change trick. As implemented:

- `crates/data-pipeline/src/graph_osm.rs` — `bake_cscl` builds the drivable
  network with a **3-layer blacklist** (each catches what the others can't):
  1. **`rw_type` keep-set** {1,2,3,4,9,10} — physical roadway type. Keeps ramps
     (9, so highways connect to surface streets); drops paths/steps/boardwalks/
     driveways/ferries. *(CSCL already carries highways incl. the FDR — no Overpass
     `motorway` fetch needed.)*
  2. **`trafdir == "NV"`** (Non-Vehicular) — CSCL's own "not for cars" flag, the
     authoritative drivability signal. Catches pedestrian malls / promenades / bike
     paths **mis-typed as `rw_type` 1 "Street"** (~1.8 k citywide, incl. ~490 the
     old keep-set wrongly routed through). Highways/ramps (2/9) are exempt — an NV
     flag there is a coding error, not a closure.
  3. **`nonped == "D"` inside a park polygon** (`boundary::ParkMask`) — the residual.
     Central Park's loop drives have been **closed to cars since 2018** but CSCL
     still codes most of them *vehicular* (`trafdir` TW/FT/TF), so only the `nonped`
     school-walk-route flag (`D`) distinguishes them — and only safely *inside* a
     park (it marks real drivable streets elsewhere). The four crosstown
     **transverses** (65/79/86/97 St) share that same `nonped=D` coding (sunken cuts
     the park bridges over), so they're whitelisted back by name — `full_street_name`
     containing "TRANSVERSE" (exactly 42 segments citywide). This keeps the open
     crosstown car routes while dropping the recreational `EAST/WEST/CENTER/TERRACE
     DR` loop — the old blanket "drop everything inside a park" mask lost both.
     Highways/ramps exempt so riverside parkways live.
  4. **NYC DOT Open Streets** (`boundary::OpenStreetMask`, Socrata `uiay-nctu`) —
     car-free streets CSCL's LION vintage predates (Broadway near Union Square, the
     West Village/LES permanent Open Streets). Kept if `reviewstat` starts with "Full
     Closure" (full + school closures; "Limited Local Access" still admits cars) **and**
     active on the simulated weekday (`apprdayswe` contains `Tue`) — so weekend-only
     closures (5 Ave Sundays, Columbus Ave) stay drivable. Matched by **colinear
     proximity** (midpoint within 10 m of a closure run, parallel within ~30°, so a
     cross street meeting a closed street at a corner isn't dropped). ~190 segments
     citywide / ~98 in Manhattan.
- `crates/sim-core/src/graph.rs` — `class_speed_mps(segment_id)` prefers the
  segment's **posted limit**, else a per-class default (Highway 40 / Ramp·Bridge 30 /
  Tunnel 35 / Street 25 mph), capped at `MAX_DRIVE_SPEED_MPS` (60 mph; keeps the
  distance÷max-speed A\* heuristic admissible). `route_timed` / `route_points_timed`
  route on cost = `length / speed`.
- `crates/data-pipeline/src/taxi_day.rs` — routes the O-D pool with
  `route_points_timed` (free-flow time).
- `crates/app-interactive/{main,loading}.rs` — loads a Manhattan **drive graph**
  (`graph_manhattan_drive.osgraph`) into `Sim.drive_graph` (citywide reuses
  `graph_nyc`).
- `crates/app-interactive/src/coverage.rs` — the overlay snaps + accumulates on
  `Sim.drive_graph`, so highway use lights up.

Later stages: `EdgeData`/asset may still gain real per-edge speed once we want
calibrated (non-class-default) limits; new data-pipeline modules for the zone
weight surfaces (Layer 1) and the route-likelihood reranker (Layer 2).

## Runtime scaling: viewport culling + route compression (citywide all-taxis)

The citywide build routes **every** O-D pair (`MAX_OD_PAIRS=40000`, `MAX_ROUTES`
uncapped) → ~195k routes / ~545k real trips, vs the Manhattan build's smaller pool.
Two mechanisms keep that performant:

- **Viewport cull** (`agents.rs::replay_agents`). `Sim.taxi_route_bboxes` holds one ENU
  bbox per route; the forward-cursor admit loop only spends a pooled slot on a trip
  whose route bbox overlaps the visible rect (camera `translation ± viewport·scale`).
  So the fixed `MAX_VEHICLES` (6000) pool bounds the *on-screen* taxi count, not the
  day's global peak (~13.3k @ 17:33): zoom into a neighborhood and every in-view taxi
  shows; zoom out to the whole city and 6000 subsample into an already-dense blob. A
  settled pan/zoom triggers a rebuild that re-seeks the active window by binary search
  (`partition_point` on `pu_min`), so it stays O(active). Measured: ~120 fps
  (vsync-capped) in WASM at peak; the whole-city frame cost is basemap-bound, not taxis.
- **Delta-quantized routes** (`sim-core/assets.rs`). `TaxiDayLayer.routes` is
  `Vec<TaxiRoute>` whose polyline is a `QuantPolyline` — exact `f32` origin + per-point
  grid-index deltas on a `QUANT_M` (1 m) grid, postcard-varint-encoded to ~1 byte/coord.
  Keeps all routes + shape (error ≤ 0.5 m, invisible for a moving dot); shrinks the
  citywide asset ~3× (110 MB raw `[f32;2]` → ~38 MB). `Deref`s to `Vec<[f32;2]>`, so
  load (`Route::from_points`) is unchanged. The legacy `VehicleRoute` /
  `vehicle_routes.osroutes` pool is left untouched (vestigial — loaded but unrendered).

## Caveats (keep honest)

- Still **inferred, not GPS.** We reconstruct a *plausible* path from OD + time +
  speed priors; it is not the real trajectory. These agents stay non-citable;
  the citable exposure headline remains the analytical field, unchanged.
- **Dasymetric ≠ truth.** Weight surfaces encode where trips *could* plausibly
  start/end, not where they did.
- **Pre-2016 empirical prior is yellow-cab,** not FHV — a later, blended
  refinement only.
- **Drive graph still simplifies** turn-penalty timing, signals, and real
  congestion; `g`'s congestion factor is a coarse correction, not a traffic
  model.
