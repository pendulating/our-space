# How taxi trips are generated

> Answers the docs TODO: *"Document thoroughly how taxi trips are currently
> generated. What is the granularity of origin/destination, route heterogeneity
> for the same origin/destination, and similar details."* (`docs/TODO.md:43`)

## TL;DR

The on-screen "taxis" (amber dashcam-vehicle triangles, internally
`AgentClass::Vehicle`) are a **replay of one real day of NYC TLC High-Volume
FHV (Uber/Lyft) trip records** — Tuesday, **April 21, 2026** as the service-date
label, sourced from the `fhvhv_tripdata_2024-06` TLC month, Manhattan↔Manhattan
only.

Three facts a reader usually wants up front:

1. **O/D granularity is the taxi *zone*, not a coordinate.** HVFHV records carry
   only `PULocationID` / `DOLocationID` (≈69 Manhattan TLC zones), no pickup or
   dropoff lat/lon. Each zone is collapsed to **one area-weighted centroid**,
   then snapped to **one nearest street-graph node**. Every trip out of a given
   zone starts at the identical node; every trip into a zone ends at the
   identical node.
2. **Zero within-O/D route heterogeneity.** For a given origin-zone →
   destination-zone pair there is **exactly one** baked polyline (a single
   length-shortest A\* path), shared by every trip on that pair. All variety
   across the day comes from the *set* of distinct O/D pairs (≤5000 routed) plus
   each trip's own pickup-minute and real duration.
3. **Routes are computed on the pedestrian walk graph**, so they ignore one-way
   streets and turn restrictions. These are **decorative agents** — they are not
   part of the citable exposure estimate.

Measured from the baked asset (`taxi_day_20260421.ostaxiday`, 3.73 MB):
**164,184 trips**, peak **3,823 concurrent at 18:20**, mean 1,929 concurrent,
effective per-trip speed median 3.3 m/s.

Taxis run in **both** builds: the default Manhattan asset, and (since #41) a
**citywide** `taxi_day_nyc` — all five boroughs, routed on the citywide graph — in
the `?city=nyc` build. See "Citywide" below.

---

## 1. Where taxi trips come from (the data pipeline)

### Two baked products — only one is the live taxi system

There are two related-but-distinct bakers/assets. It is easy to confuse them:

| Asset | Baker | Struct | Role today |
|---|---|---|---|
| `vehicle_routes.osroutes` | `crates/data-pipeline/src/vehicle_routes.rs` | `VehicleRoutesLayer` | **Legacy / superseded.** A weighted *pool* of representative routes (no per-trip timing). |
| `taxi_day_20260421.ostaxiday` | `crates/data-pipeline/src/taxi_day.rs` | `TaxiDayLayer` | **The current taxi system** — real per-trip schedule replay. |

Both are loaded into the `Sim` resource, but the runtime taxi replay
(`replay_agents`) reads only `sim.taxi_day` / `sim.taxi_routes` / `sim.taxi_paces`
(`crates/app-interactive/src/agents.rs:712`, `:755`, `:772`). The
`VehicleRoutesLayer` is still loaded and logged at setup
(`crates/app-interactive/src/main.rs:1713`, `:1850`) but is **not consumed by the
agent animation** — it is effectively vestigial, predating the real-day model.
The rest of this document describes `taxi_day.rs` / `TaxiDayLayer`.

### Source data

NYC TLC **High-Volume FHV** (HVFHV = Uber/Lyft and other app dispatch; **not**
yellow-medallion taxis) trip records, aggregated with DuckDB
(`README.md:238-246`). The published HVFHV Parquet is
`fhvhv_tripdata_2024-06.parquet`. The DuckDB filter keeps only
**Manhattan↔Manhattan** trips (`PULocationID IN (IDS) AND DOLocationID IN (IDS)
AND PULocationID <> DOLocationID`, where `IDS` is the 69 Manhattan `LocationID`s).

> **Provenance / vintage caveat.** The *service-date label* is `20260421`
> (Tuesday, April 21, 2026 — chosen as the overlap of the GTFS service span and
> the latest published TLC month), but the underlying trip *data* is the TLC
> 2024-06 HVFHV month. The label and the data vintage differ by design; see the
> memory note "Real-day trip model."

### Baker inputs (`crates/data-pipeline/src/taxi_day.rs:25-31`)

`bake(graph_path, geojson_path, perminute_csv, trips_csv, date, out_path)`:

- the baked walk graph (`.osgraph`) — for offline routing;
- the taxi-zone GeoJSON (`LocationID` → ENU polygon rings), parsed by
  `parse_zones` (`crates/data-pipeline/src/dashcam.rs`);
- a **per-minute O/D CSV** `pu_min,PULocationID,DOLocationID,trips`
  (the full analytical aggregate);
- a **per-trip CSV** `pu_min,PULocationID,DOLocationID,dur_min`
  (one row per real trip);
- the `YYYYMMDD` service-date label.

CLI wiring: `crates/data-pipeline/src/main.rs:98-108`
(`bake-taxi-day <graph> <taxi_zones.geojson> <perminute_od.csv> <trips_all.csv>
<YYYYMMDD> <out.ostaxiday>`).

> **Documentation gap (honesty).** The `taxi_day.rs` header comment
> (`:5-8`) refers to "see README/fetch-taxi-day" for the DuckDB query that
> produces the per-minute and per-trip CSVs, but **no such section currently
> exists** in `README.md` — only the older zone-aggregate query for
> `vehicle_routes.osroutes` (`README.md:238-246`) is documented. The exact query
> that derives `pu_min` (pickup minute-of-day) and `dur_min` (trip duration) from
> the HVFHV `request_datetime`/`dropoff_datetime` is not checked into the repo.

### What the baker does (`taxi_day.rs:45-127`)

1. **Read the day's trips** (`:45-60`). Each row → `(pu_min, pu, dolc, dur)`;
   `dur` is clamped to a **0.5-minute floor** (`dur.max(0.5)`, `:56`). It also
   counts O/D-pair frequencies into `od_freq`.
2. **Route the top distinct O/D pairs by frequency** (`:62-85`). The pairs are
   sorted by trip count descending and the top `MAX_OD_ROUTES = 5000` (`:23`,
   `:68`) are each routed **once** from pickup-zone centroid to dropoff-zone
   centroid over the walk graph (`graph.route_points`), then decimated
   (Douglas–Peucker, `DECIMATE_TOL_M = 5.0` m, `:21`). Each routed pair gets a
   `route_idx` and a `VehicleRoute { polyline, length_m, weight = freq }`.
3. **Map each trip → `route_idx`** (`:88-95`), **dropping trips whose O/D pair
   was not routed** (outside the top-5000, or no path). The surviving trips are
   sorted ascending by `pu_min`.
4. **Build the full per-minute O/D aggregate** (`:97-110`) into
   `od_per_minute: Vec<TaxiOdMinute>` — this is the *complete* count (all pairs,
   all minutes), used by the analytical headline, **not** subsampled.

Output struct `TaxiDayLayer` (`crates/sim-core/src/assets.rs:744-758`):
`{ origin, service_date, routes, trips, od_per_minute, provenance }`.

---

## 2. Granularity of origin / destination

**The finest available O/D unit is the TLC taxi zone — there are no point
coordinates in the source.** The HVFHV trip records expose only `PULocationID`
and `DOLocationID` (zone IDs); Manhattan has ≈69 such zones.

The pipeline reduces a zone to a single point twice over:

- **Zone → centroid.** `zone_centroid` (`vehicle_routes.rs:30-54`) takes the
  **largest ring** of the zone polygon and computes its **area-weighted shoelace
  centroid** (degenerate rings fall back to the vertex average). One point per
  zone.
- **Centroid → graph node.** Routing snaps each centroid to the **nearest street
  node** via `route_points` → `snap_nearest`
  (`crates/sim-core/src/graph.rs:107-111`, `:71-81`; a linear nearest-neighbor
  scan).

Consequences:

- O/D granularity is effectively **1-of-69 zones**, each pinned to **one fixed
  node**. Real pickup/dropoff scatter *within* a zone is lost.
- **Every** trip leaving zone *A* begins at the same node; every trip entering
  zone *B* ends at the same node. Trips do not start/end at addresses, curbs, or
  exact coordinates.
- Because routing is over the **pedestrian** walk graph, the snapped node is a
  walk-graph node, not a drive-network node.

---

## 3. Route heterogeneity for the same origin → destination

**None.** A given (PU zone, DO zone) pair maps to a single `route_idx`
(`taxi_day.rs:79`), and every trip on that pair references that one index
(`:91-92`). So **all trips between the same two zones render the identical
polyline**, traversed identically. There is no jitter, no alternate-path
sampling, no per-trip lane/curb variation.

How the one route is computed:

- A single **A\* shortest path by edge length** with a Euclidean (straight-line)
  admissible heuristic (`graph.rs:85-103`), between the two snapped nodes
  (`route_points`, `:107-111`).
- The path is densified into a `Route` polyline and **decimated** with
  Douglas–Peucker at a 5 m tolerance for compactness (`taxi_day.rs:75`).

Where day-to-day variety *does* come from:

- **Distinct O/D pairs:** up to `MAX_OD_ROUTES = 5000` different routes exist in
  the pool, weighted by real trip frequency.
- **Per-trip timing:** each trip has its own `pu_min` (when it appears) and
  `dur_min` (how long it takes), so two trips on the same route still differ in
  *when* they run and *how fast* they cover it.

> **Caveat — walk graph, not drive network** (`assets.rs:651-655`,
> `vehicle_routes.rs:9-14`, `taxi_day.rs:123-126`). Routes are computed on the
> OSM pedestrian network (no drive graph in v1), so they **ignore one-way
> streets and turn restrictions**. The provenance note records this explicitly:
> *"Zone-centroid routes over the pedestrian walk graph (decorative; ignores
> one-way/turn restrictions)."* These agents are a visualization, not a routed
> traffic model.

---

## 4. Timing & replay over the simulated day

### Per-trip schedule (`crates/sim-core/src/assets.rs:725-732`)

```
struct TaxiTrip { pu_min: f32, route_idx: u32, dur_min: f32 }
```

A trip **appears at `pu_min`** (minutes since midnight), drives `route_idx`,
and **vanishes after `dur_min`** minutes.

### The replay loop (`crates/app-interactive/src/agents.rs:691-780`)

`replay_agents` runs every frame (whether the clock is playing *or* paused —
scrub-and-pause shows exactly the trips active at that minute, `:704-705`):

- `now = clock.time_of_day * 60` (minute-of-day, `:709`).
- A trip is **active** when `now ∈ [pu_min, pu_min + dur_min)` (`:714`).
- Slots are filled by a **forward cursor over the start-sorted trip list**
  (`:742-762`): started trips are admitted into free vehicle slots in start
  order; ended trips free their slot (`:731-739`).
- A clock **wrap/scrub** (jump back, or forward >30 min) drops all taxi slots and
  rebuilds from the cursor (`:716-728`).
- Toggle: active only when `params.dashcam_on` (and `show_agents`, not heatmap)
  — `taxi_on` at `:713`.

### Position within a trip — the `PaceProfile` speed model

Each active taxi's position is set from its **elapsed-time fraction**
`frac = (now - pu_min) / dur_min` (`agents.rs:770`), but **not** as a flat
constant-speed glide. It is mapped through a per-route **`PaceProfile`**
(`agents.rs:772-775`):

```
a.progress_m = sim.taxi_paces[route_idx].arc_at(&route, frac)   // turn-aware
             // fallback: frac * route.total_m                  // linear
```

`PaceProfile` (`crates/sim-core/src/graph.rs:342-425`) is **turn-aware and
speed-limit-capped**:

- Cruise speed is capped at `NYC_SPEED_LIMIT_MPS = 11.176` m/s (25 mph)
  (`main.rs:49-51`).
- Sharp turns are taken down to `TURN_SPEED_FRAC = 0.3` of the limit — the
  vehicle **brakes into corners and accelerates on straights** (`graph.rs:373-393`).
- It stores `time_frac[i]` = cumulative fraction of total trip *time* at each
  route vertex; `arc_at` inverts that to an arc length (`graph.rs:409-424`).
- The profile preserves the trip's **real total duration** — it only redistributes
  speed *within* the trip. Built once per pooled route, not per trip
  (`main.rs:1790-1793`).
- Degenerate routes fall back to a constant (linear time↔arc) pace
  (`graph.rs:365-368`).

**Honesty about the limit** (`graph.rs:349-353`): when a trip's average speed is
at/below the limit — nearly all Manhattan trips, which crawl in traffic — every
instantaneous speed stays ≤ the limit. A trip whose data is faster than even a
limit-paced traversal is replayed faithfully (the record itself implies
speeding).

Effective speeds the replay actually produces (from `examples/taxi_peak.rs`,
which derives speed as routed-`length_m` ÷ real `dur_min`):

```
taxi effective speed m/s (per-trip):     p10=1.7  median=3.3  mean=3.6  p90=5.9
taxi on-screen pace m/s (dur-weighted):  p10=1.6  median=3.1  mean=3.4  p90=5.6
```

(These are also why the synthetic **Tesla** agents use a fixed
`TESLA_SPEED_MPS = 3.5` — to flow with the replayed taxis rather than zip past;
`agents.rs:44-48`.)

### Clock & time-lapse

The master `SimClock.rate` time-lapse (default 36 sim-s per real-s ≈ a 24 h day
every ~40 min; `main.rs:55-62`) advances `now`, so it controls how fast the day
plays — but a taxi's *position* is always derived from the real-schedule
`frac`, so its on-route pacing stays faithful at any time-lapse rate.

### Heading / rendering

`animate_agents` positions vehicles from the `progress_m` set above and orients
the (symmetric) triangle along travel heading (`agents.rs:582-596`). Vehicles use
`AMBER` material, z-order `Z_VEHICLE = 2.6` (`agents.rs:80`, `:223`).

---

## 5. Counts, caps & scope

Measured from `crates/app-interactive/assets/processed/taxi_day_20260421.ostaxiday`
(3.73 MB) via `cargo run -p sim-core --example taxi_peak`:

```
baked trips:            164184
PEAK concurrent:        3823  at 18:20
mean concurrent (24h):  1929
median / p95:           2379 / 3477
hourly peak:  06=1253 07=2636 08=3535 09=3413 ... 17=3743 18=3823 19=3165 ...
```

- **164,184 baked trips** = the real-day HVFHV trips whose O/D pair landed in the
  top-5000 routed pool (rarer pairs are dropped from the *visual* trip list at
  `taxi_day.rs:90-94`, but still counted in `od_per_minute` for the headline).
- **Routes:** capped at `MAX_OD_ROUTES = 5000` distinct O/D pairs
  (`taxi_day.rs:23`); the actual baked count is ≤5000 minus any no-path skips.
- **On-screen pool:** `MAX_VEHICLES = 4000` pre-spawned entities
  (`agents.rs:30`). This **exceeds the day's true peak concurrency (3,823 at
  18:20)**, so the taxi subsample never triggers — *"effectively uncapped for
  this day"* (`agents.rs:28-29`).
- **Over-cap behavior** (would only matter on a busier day): a **deterministic,
  earliest-by-start** subsample — trips are admitted in start order until the
  pool is full (`agents.rs:740-762`, comment `:685-689`).
- **Pool is fixed** (no runtime spawn/despawn): one shared mesh+material, O(log n)
  `position_at` per agent (`agents.rs:7-10`).

### Citywide (`?city=nyc`) — taxis are ON (all five boroughs)

The five-borough build now replays a real day of **all-borough** TLC HVFHV trips
alongside the ACE buses (glasses/robots/Teslas stay off — their fields are still
Manhattan-only):

```rust
// crates/app-interactive/src/main.rs (citywide block)
if citywide {
    params.ace_on = true;       // ACE buses citywide
    params.dashcam_on = true;   // ← rideshare/taxi vehicles citywide (TLC HVFHV)
    params.glasses_on = false;
    params.robots_on = false;
    params.tesla_on = false;
}
let taxi_day_path = if citywide { TAXI_DAY_PATH_NYC } else { TAXI_DAY_PATH };
```

`taxi_day_nyc.ostaxiday` is baked exactly like the Manhattan asset but **without
the Manhattan zone filter** and **routed over the citywide graph**
(`graph_nyc.osgraph`): 547,263 trips for one real Tuesday (2024-06-25), all 258/259
PU/DO zones; the bake keeps the 391,380 trips on the top-5000 O-D routes (0 no-path).
The full reproducible pipeline (the DuckDB extraction that was previously an
undocumented gap, plus the bake) lives in **`tools/bake_citywide.sh`**.

> **Analytic field (already citywide).** The *visible* citywide taxis come from
> `taxi_day_nyc`; the *analytic* dashcam exposure field (`DashcamFieldLayer`, used for
> the walk estimate + the dashcam heatmap) is a **separate** asset (`dashcam_field.osfield`)
> and was already five-borough — it's baked from the all-borough `zone_trips.csv` (PU+DO,
> 262 zones) over all 263 taxi zones with **no** Manhattan filter, so `intensity_at`
> returns real per-borough density everywhere (probed: Manhattan 3.74× / Brooklyn 1.94×
> / Queens 0.51× the median zone). The exposure model samples it citywide; nothing
> further was needed.

---

## 6. Real vs. modeled — what to trust

| Aspect | Status |
|---|---|
| Trip **count & timing** (which O/D, what minute, what duration) | **Real** — actual HVFHV records (TLC 2024-06, Manhattan↔Manhattan). |
| Per-minute O/D **volume** (the headline temporal factor) | **Real** — full `od_per_minute` aggregate, normalized to the day's peak (`mobile.rs:91-98`). |
| Pickup/dropoff **location** | **Modeled** — zone centroid, not exact coordinates (≈69 zones, one node each). |
| The **route** taken | **Modeled** — one shortest path on the *pedestrian* graph per O/D pair; ignores one-way/turn rules; identical for all trips on that pair. |
| On-route **speed/pace** | **Modeled** — turn-aware, 25 mph-capped `PaceProfile`; only the *total duration* is real, the within-trip pacing is synthetic. |
| Vehicle **identity** | HVFHV (Uber/Lyft etc.), **not** yellow taxis; framed in the UI as in-vehicle dashcams. |

### The visual / analytical split (important)

The taxi data feeds the app in **two independent ways**:

- **Visual replay** (`agents.rs::replay_agents`): the moving amber triangles
  described above. Decorative; not part of any cited number.
- **Cited headline** (`sim_core::mobile::RealDayRates::from_day`,
  `crates/sim-core/src/mobile.rs:66-101`): derives a per-minute
  `taxi_traffic_mult` by summing **pickups per minute** and normalizing to the
  day's peak (`:91-98`). Note this uses **only the temporal volume** — the
  `pu_zone`/`do_zone` fields of `TaxiOdMinute` are present but **not used
  spatially** here. Per the memory note, *"Only the temporal shape is real; the
  spatial model — corridor proximity, zone density, FOV/occlusion, recall — is
  unchanged."* When no real day is loaded (batch/tests) it falls back to the
  synthetic `traffic_multiplier` diurnal curve (`mobile.rs:115-117`, `:26-28`).

### Caveats a new contributor should keep in mind

1. **Zone-level O/D only** — no intra-zone position; no curb/address precision.
2. **One route per O/D pair** — no within-pair route diversity by design.
3. **Pedestrian-graph routing** — one-way streets and turns are not respected;
   these are decorative agents, explicitly excluded from the citable estimate.
4. **Effective speed = synthetic route length ÷ real duration** — so a taxi's
   apparent speed mixes a real number (duration) with a modeled one (centroid-route
   length); it is not GPS-derived (`examples/taxi_peak.rs:43-45`).
5. **`vehicle_routes.osroutes` is legacy** — still loaded but not used by the
   taxi replay; do not confuse it with `taxi_day`.
6. **Service-date label ≠ data vintage** (2026-04-21 label vs. TLC 2024-06 data).
7. **The CSV-generation DuckDB query is not in the repo** — only the older
   zone-aggregate query is documented; reproducing the bake requires deriving
   `pu_min`/`dur_min` from the HVFHV datetimes yourself.

---

## File map

| Concern | File:line |
|---|---|
| Taxi-day baker (current system) | `crates/data-pipeline/src/taxi_day.rs` |
| Zone centroid + DP decimation (shared) | `crates/data-pipeline/src/vehicle_routes.rs:30-100` |
| Legacy route-pool baker | `crates/data-pipeline/src/vehicle_routes.rs` |
| CLI wiring | `crates/data-pipeline/src/main.rs:98-108`, `:196-203` |
| `TaxiTrip` / `TaxiOdMinute` / `TaxiDayLayer` structs | `crates/sim-core/src/assets.rs:725-767` |
| `VehicleRoute` / `VehicleRoutesLayer` structs | `crates/sim-core/src/assets.rs:651-681` |
| Graph routing (A\*) + node snap | `crates/sim-core/src/graph.rs:71-111` |
| `PaceProfile` speed model | `crates/sim-core/src/graph.rs:342-425` |
| Runtime replay + subsample | `crates/app-interactive/src/agents.rs:691-780` |
| Caps (`MAX_VEHICLES`) | `crates/app-interactive/src/agents.rs:30` |
| Pace construction + `Sim` wiring | `crates/app-interactive/src/main.rs:1780-1865` |
| Speed-limit / turn constants | `crates/app-interactive/src/main.rs:49-53` |
| Citywide taxi-off | `crates/app-interactive/src/main.rs:1004-1021` |
| Analytical headline (temporal only) | `crates/sim-core/src/mobile.rs:66-117` |
| Asset path constant | `crates/app-interactive/src/main.rs:128` (`TAXI_DAY_PATH`) |
| Peak-concurrency / speed report tool | `crates/sim-core/examples/taxi_peak.rs` |
| Source & license | `README.md:238-264` |
