# our-space — Implementation Plan

> **Status (current):** this is the *original* roadmap, kept for historical and
> design-rationale context. All of Phases 0–4 shipped, plus a Phase 5 (animated
> ambient agents + dual-mode exposure) and several post-plan additions: **NYC DOT
> traffic cameras**, a **merged + de-duplicated fixed-CCTV census** (Amnesty
> *Decode Surveillance NYC* + Dahir et al.), **ALPR** readers (DeFlock), a **40×
> walker playback time-lapse**, and **GitHub Pages continuous deployment**. For
> the as-built system see [`ARCHITECTURE.md`](ARCHITECTURE.md); for the visual
> system see [`DESIGN.md`](DESIGN.md).

A geospatial **simulation + visualization of NYC's growing sensing layer** (fixed CCTV, ACE bus cameras, dashcams, smart glasses), built on the real Manhattan street network. The user enters a walking route A→B; the sim estimates **how many cameras could capture them** and the **frequency of capture**, with **time-of-day** mattering. Built in **Bevy (Rust/ECS)**, shipping to the browser via **WASM + WebGPU**. Interactive single-route mode first; a batch/citywide mode later.

## Context

This is a greenfield repo (initial commit only; `_raw/` holds source docs, `docs/` empty). The project is the public-facing, forward-looking sibling of Dahir et al. (Nature Cities 2025, in `_raw/surveillance-and-diversity.md`), which found surveillance cameras concentrate in racially **diverse / gentrifying** neighborhoods — more strongly than crime predicts. Where that paper *measured* fixed CCTV from street-view imagery, **our-space** lets a person *feel* the cumulative, time-aware exposure of an ordinary walk as **new** sensing modalities (mobile/ambient cameras) enter public space. The pedestrian-modeling approach draws on Ulrich et al. (`_raw/fbuil-10-1447377.md`): a macroscopic Network Model (Space-Syntax angular betweenness) feeding micro movement — used here only in the batch layer.

A deep, adversarially-verified research workflow (23 agents) grounded every load-bearing claim below; key verifications are cited inline. The goal is an **honest estimate tool**, never a surveillance map or an evasion guide.

## Decisions locked (from the user)

| Decision | Choice | Consequence |
|---|---|---|
| Deployment posture | **Non-commercial / research** | NonCommercial-licensed data (e.g. Mapillary Vistas CC BY-NC-SA) is *permitted* but unneeded; simplest license story. |
| Fixed-camera data | **Dahir CC-BY points + derived** | Ship Stanford `map_data.csv` (511 NYC detected-camera points, CC BY 4.0). **Do not** depend on Amnesty Decode (no license). |
| Web reach | **WebGPU-first, single build** | One WASM artifact for v1 (WebGPU Baseline since Jan 2026). No WebGL2 fallback / dual-build complexity now. |
| Headline metric | **"Cameras that saw you" (count)** | Default prominent number = E[distinct devices]; full E[capture-events] and %-watched computed underneath as secondary readouts. |

## Architecture overview

**One render-agnostic simulation core, two hosts.** Movement, capture-detection, and exposure-accumulation are Bevy ECS systems that touch **no** `Mesh`/`Material2d` types, so they run identically in (1) the interactive app (default Bevy plugins; native window for dev, WebGPU WASM for the public build) and (2) a **native headless batch** (`MinimalPlugins` + `ScheduleRunnerPlugin`) for citywide heatmaps. Keeping the core render-free is the load-bearing rule.

**Routing is decoupled from exposure.** A lone walker doesn't change which sidewalks are walkable, so we compute the shortest path **once** (time-weighted A* over a `petgraph`), advance the walker along edge polylines at constant speed (default **1.34 m/s**) on a discrete clock (**dt = 1 s**), then evaluate every sensor's capture geometry against the walker's position **and** the current clock time. Time-of-day enters *exposure* (bus headways, diurnal traffic, day/night camera schedules), not the path search.

**Projection:** hand-rolled closed-form **local ENU-meters** centered on Manhattan (keeps FOV geometry and any future social-force math Euclidean; trivial at this scale). The native `proj` crate does **not** build for `wasm32` (georust/proj #115, open since 2022); `proj4rs` (pure-Rust, WASM-ok) is the fallback only if multi-CRS is ever needed.

**Web delivery is precompute-biased:** Bevy's WASM scheduler is single-threaded (#4078). Heavy work (graph baking, occlusion precompute, batch heatmaps) runs in the native path and ships as **static assets**; the WASM build is a single-threaded **viewer** doing one A* + per-tick exposure.

## Tech stack (versions verified against published manifests)

- **Bevy 0.18.1** (pinned; 0.19 unreleased). Built-in `PanCamera` covers 2D pan/zoom.
- Companion crates (all Bevy-0.18): **`bevy_egui` 0.39.1** (dev tooling only), **`bevy_ecs_tilemap` 0.18.1** (raster basemap), **`bevy_prototype_lyon` 0.16.0** (FOV cones), **`leafwing-input-manager` 0.20.0** — ⚠️ **must be 0.20.0, not 0.18.0** (leafwing 0.18 pins Bevy 0.17 and will hard-fail the build; verified via its Cargo.toml). Gate with `cargo tree -d -i bevy_ecs`.
- **`petgraph`** (interactive A*/Dijkstra) + **`fast_paths` 1.0.0** (contraction hierarchies for batch O-D sampling; MIT; has WASM support via `serialize_32`/`deserialize_32`).
- **`geo` / `geo-types` / `geojson`** (WASM-safe geometry: point-in-polygon, segment-intersection for line-of-sight) + **`rstar`** (R-tree over cameras + building footprints).
- **`gtfs-structures` 0.48** with `default-features=false` (drops `reqwest`; load GTFS-static from in-memory bytes via `from_reader`, not `from_path` — `std::fs` errors on WASM). `gtfs-rt` is an optional later async-fetch enhancement, not a core dep.
- Web pipeline: **`bevy_cli`** (`bevy build web --bundle`) + `wasm-opt -Oz` + `opt-level="z"` + LTO; budget 15–30 MB. **DOM/HTML overlay** for UI over a named Bevy canvas (smaller, accessible, easier to style than egui-in-canvas); route/scenario in via `#[wasm_bindgen]` → Bevy `Resource`/event, results out via getters. **Avoid `mapgpu`** (it's a TS/Rust-WASM npm lib, not a Bevy crate; README/LICENSE disagree on terms).
- Geocoding: **NYC Planning Labs GeoSearch** (Pelias, no API key, `/search` + `/autocomplete`, returns lon/lat GeoJSON) for A→B typeahead. No reverse-geocode / SLA — budget a self-hosted Pelias fallback only if availability matters.
- ~10⁵ camera points render as **ordinary `Sprite`/`Mesh2d` entities** — Bevy's default renderer auto-batches them (the official `many_sprites` example draws 102,400). Custom WGSL instancing is a contingency only if per-instance color breaks batching.

## Repo structure (Cargo workspace)

```
our-space/
  Cargo.toml                 # workspace
  crates/
    sim-core/                # render-agnostic: components, exposure model, systems, projection
    data-pipeline/           # native CLI: ingest NYC datasets -> baked static assets
    app-interactive/         # Bevy app (native dev window + WebGPU wasm target)
    batch/                   # native headless: citywide heatmap + O-D sampling
  web/                       # DOM/HTML overlay, JS<->wasm bridge, deploy (Cloudflare Pages)
  assets/processed/          # baked binary graph, sensor layers, heatmaps (gitignored if large)
  data/snapshots/            # dated raw-data snapshots + provenance manifest (as-of dates)
```

## Data pipeline (offline, native, run once per data-vintage snapshot)

Pin an **"as-of" date** in the UI; ACE routes/fleet and glasses adoption drift monthly. Each layer ships with a **provenance badge** (source, date, license).

1. **Walking graph** — OSMnx 2.1.0 `ox.graph_from_place('Manhattan, New York, USA', network_type='walk')`. Returns (by default) the largest weakly-connected component as a bidirectional, immediately-routable graph. **Must** add nearest-node snapping (`ox.distance.nearest_nodes`) + endpoint-in-component validation, or peripheral A/B nodes silently throw `NetworkXNoPath` / snap wrong and corrupt counts. *Caveat:* it's a street-**centerline** pedestrian graph, not sidewalk-accurate (OSM NYC sidewalk coverage is spotty) → LION (v26a, `TrafDir='P'`) / Sidewalk Centerline (`a9xv-vek9`) conflation is a **v2** upgrade so cameras can face a specific curb side. Export node/edge lists (edge: ENU polyline, `length_m`, `traversal_time`, source segment ID) to a compact binary.
2. **Fixed CCTV** — Stanford `map_data.csv` (`purl.stanford.edu/jr882ny4955`, **CC BY 4.0**). Columns `panoid,heading,lat,lon,city,year,month,camera_count`; **511 NY rows** with `camera_count≥1`. These are **GSV panorama sample-points** (where a camera was visible), *not* device coordinates → dedupe by `panoid`, label as modeled, carry the **~0.63 recall** undercount (divide density by 0.63 for an unbiased estimate; show an uncertainty band). Heading/FOV/range are **not** in the data → user-settable model assumptions.
3. **ACE buses (mobile line-buffer)** — cameras are **bus-mounted** (~80° context cam; verified 5 ways) → capture follows route geometry, not fixed points. Route list from official keyless dataset **`data.ny.gov` ki2b-sg5y** ("MTA Bus Automated Camera Enforced Routes": Route + Program ACE/ABLE + Implementation Date; ~60–63 ACE routes; OPEN-NY ToU). Join on `Route` to **MTA GTFS-static** `shapes.txt` (keyless ZIP, `rrgtfsfeeds.s3.amazonaws.com`) for geometry and `stop_times.txt` for headways (watch SBS `+` suffixes, e.g. `M15+`). Enforcement **hours** are on signage only — flag 24/7 as an assumption. Bundle a **dated snapshot**.
4. **Dashcam field** — spatial-temporal structure is **empirical**: NYC DOT **Automated Traffic Volume Counts** (`7ym2-wayt`: SegmentID, WktGeom, Yr/M/D/HH, 15-min Vol). Field = real volume × **penetration slider** (default **urban ~40%**, range 25–45%; distinguish forward-facing dashcams from passenger-facing TLC IVCS; weight by Manhattan being ~77% car-free). Tunable, low-confidence, labeled estimate.
5. **Smart glasses** — **no** geospatial dataset (only global sales: ~7M Meta units in 2025 — real, but unconnectable to any NYC per-pedestrian rate). Pure scenario: `glasses_per_1000_pedestrians` (default low, slider) × `P(recording)` (unknowable; LED hardware-defeatable) × pedestrian-density field. **Tier-D speculative**, visually distinct, default-low.
6. **(Optional) DOT live-view cams** — `webcams.nyctmc.org/api/cameras/` (public JSON, ~900–1,000, fluctuates). **Legally encumbered** (no open license; DOT cease-and-desist over reuse) → coords scrape-able but **never redistribute the images**; model as a separate low-weight *monitoring* class. Defer; include only if desired.
7. **Equity overlay (Phase 3)** — Census TIGER block groups (FIPS **36061**, 2023) + ACS 5-year API; Shannon entropy `H = -Σ pᵢ ln pᵢ` over white/Black/Asian/Hispanic/other, exactly per Dahir. Block-group aggregation only, **opt-in**, with the paper's framing to prevent causal misreading.

**Processed outputs:** binary street-graph asset, per-class sensor assets (fixed points + precomputed visible polygons, ACE polylines + headway tables, dashcam/ped-density segment fields), baked citywide heatmap, ACS/TIGER overlay — all static, client-loadable.

## Exposure model

**Compute E[C] = E_fixed + E_mobile per tick; display "cameras that saw you" (count) as the headline, with E[capture-events] and %-watched as secondary.**

- **Fixed (point-with-frustum, occlusion-tested):** for camera *c* with position `p_c`, heading `θ_c`, half-FOV `α_c`, range `R_c`, frame_rate `f_c`, schedule `s_c` — while the walker `x(t)` satisfies `|x−p_c| ≤ R_c` AND `angle(x−p_c, θ_c) ≤ α_c` AND **line-of-sight clear** (2D building-footprint segment test via NYC Building Footprints `5zhs-2jue` + `rstar`) AND `s_c` active: `captures += f_c · dt`. Occlusion matters — Manhattan LOS is wall-limited; ignoring it grossly over-counts. Apply the 1/0.63 recall correction + uncertainty band.
- **Mobile (Poisson encounter / space-time intensity):** each class is an intensity field `λ_class(segment, t)`; expected captures `= ∫ λ(x(t),t)·capture_prob·dt` over route-time. Encounters are independent rare events → model the **count as Poisson(E_mobile)**, also report `P(≥1) = 1 − e^(−E_mobile)`. Per class: **ACE** → expected passes in a dwell window `= window / headway(t)` along route geometry, capture when within bus side/forward FOV (~80–100°, ~20 m); **Dashcam** → empirical traffic × penetration × P(in-FOV); **Smart glasses** → ped-density × adoption × P(recording).
- **Time:** single global clock; matters most for buses (headways vary 3–5× peak↔overnight) and dashcams (diurnal), least for fixed (mostly 24/7). Report E[C] as a function of departure hour ("a 2pm walk vs a 2am walk"). Reuse Ulrich's ~0.075 hourly-of-daily factor pattern.
- **Default parameters (all user-settable, since source has only points):** Fixed CCTV {fov 70° (25–110), range 15 m (10–40), 15 fps, 24/7}; ACE {fov ~80–100°, range 20 m, encounter = 1/headway, headway 6–12 min peak / 15–30 off-peak}; Dashcam {fov 140°, range 20 m, penetration 40% (25–45 slider), forward-facing 0.3–0.8}; Smart glasses {fov 90°, range 5 m, 10/1000 ped **placeholder** (0–50 slider), P(recording) ~0.05, tier-D}.

## ECS design (sim-core)

- **Components:** `WorldPos` (ENU m); `Pedestrian` (route refs, speed, progress, `ExposureTally`); `FixedCamera` (heading, half-FOV, range, fps, owner_type, confidence_tier, `Schedule`); `MobileSensor` (class-tagged; Velocity+route for buses, or field-binding for dashcams/glasses); `Velocity` + `RouteFollow`; `FrustumWedge` (shared in-range/in-angle/LOS test).
- **Resources:** `StreetGraph` (petgraph); `SimClock` (dt + t0); `MapProjection` (ENU); `ExposureLog`; `SensorSpatialIndex` (rstar); `IntensityFields` (per-segment time-scaled λ); `ScenarioParams` (all sliders).
- **Systems (render-agnostic):** Routing (snap → validate → A* → emit `position(t)`); Movement; Capture-detection; Exposure-accumulation. Plus native-only Batch driver and a `#[wasm_bindgen]` JS interop bridge.

## Phased roadmap

**Phase 0 — De-risk spikes & licensing.** `cargo build --target wasm32-unknown-unknown` spike combining Bevy 0.18.1 + petgraph + fast_paths 1.0 + gtfs-structures(`default-features=false`) + geo + rstar; `cargo tree -d -i bevy_ecs` confirms single Bevy 0.18 (leafwing **0.20.0**). ~1e5 `Sprite` stress test under WebGPU. Route 5+ representative Manhattan A→B pairs (incl. a bridge + a plaza) on the OSMnx graph with snapping + endpoint validation. Confirm Dahir `map_data.csv` as the seed; record DOT-cam encumbrance.

**Phase 1 — Thin interactive single-route demo.** Develop against a **native Bevy window** for fast iteration (same ECS code targets WebGPU WASM). OSMnx graph baked to binary + ENU; petgraph A* with snapping/validation. Fixed-CCTV layer from Dahir points (recall-corrected, dedup by panoid) with default FOV/range + 2D footprint occlusion. Constant-speed traversal + SimClock; headline **"~N cameras saw you"** + capture-events/%-watched toggles. Real basemap (`bevy_ecs_tilemap`); About/provenance panel with the 0.63 caveat.

**Phase 2 — Mobile sensors + time.** ACE line-buffer along GTFS shapes joined to `ki2b-sg5y`; headway-driven Poisson encounters. Dashcam field from `7ym2-wayt` × penetration slider; smart-glasses scenario field (tier-D, default-low). Departure-hour input wired to diurnal scaling; per-source E[C] breakdown with confidence tiers A–D.

**Phase 3 — Batch heatmap + equity overlay.** Native headless coverage-aggregation heatmap (per-segment expected-captures/min) as primary; `fast_paths` O-D sampling weighted by Ulrich-NM betweenness flow as secondary. Opt-in block-group equity overlay (TIGER 36061 + ACS, Shannon entropy per Dahir) with anti-misreading framing. Bake to static tiles.

**Phase 4 — Web embedding & hardening.** Single WebGPU WASM build via `bevy_cli` + size-opt; DOM overlay + GeoSearch route entry; Cloudflare Pages deploy. Privacy-by-design verified (**route stays client-side, never transmitted/logged**); per-layer provenance/license/date badges; "estimate, not a surveillance map" messaging; accessibility pass. (WebGL2 fallback explicitly deferred per the WebGPU-first decision; re-check Bevy #13168 if revisited.)

## Ethics & dissemination guardrails (apply throughout)

- **No "least-surveilled route" optimizer** — avoids being an evasion guide. Route comparison, if added, is framed as awareness only.
- **Route privacy:** entire simulation runs client-side; route is never sent to a server or logged.
- **Honest provenance:** every number shown as a **range** with a "model estimate, not ground truth" disclaimer; the 0.63 recall caveat surfaced; dashcam/glasses explicitly labeled **scenario, not measurement**; source + date + license badge per layer.
- **No doxxing:** ship only the CC-BY Dahir points (already panorama-level, not device-level) and aggregated/derived exposure; never redistribute DOT live images.

## Key risks & mitigations

- **Data-provenance misframing** — Dahir coords are GSV sample-points w/ 0.63 recall, not verified devices → "modeled exposure" framing + bands + tier badges.
- **Speculative classes over-claim** — glasses/dashcam intensities are assumptions → tunable sliders, default-low, labeled.
- **Dependency trap** — `leafwing-input-manager` must be **0.20.0**; `cargo tree` gate in Phase 0.
- **ACE drift** — route list grows monthly, enforcement hours absent from feeds → dated snapshot + 24/7-assumption note.
- **Sidewalk fidelity gap** — OSM walk graph is centerline, not sidewalk-accurate → which-side-of-street CCTV facing approximate; LION/Sidewalk-Centerline conflation deferred to v2.
- **WASM single-thread + bundle size** (#4078, 15–30 MB) → web-as-viewer, precompute heavy work natively, aggressive size-opt.

## Verification

- **Phase 0:** build/link spike passes; `cargo tree -d -i bevy_ecs` shows exactly one Bevy 0.18; representative A→B routes succeed (incl. bridge/plaza) without `NoPath`; 1e5 sprites hold framerate under WebGPU.
- **Per-phase:** unit tests on the exposure math (known camera + straight route → analytic frame count; Poisson E and P(≥1) sanity); a fixed seed route produces a deterministic, hand-checkable exposure tally. Snapshot tests on the data-pipeline outputs (graph node/edge counts, 511 NY camera points dedup'd by panoid).
- **End-to-end:** run `app-interactive` natively, enter a known Manhattan A→B at a fixed hour, confirm the headline count + breakdown are plausible and that the occlusion test reduces coverage vs. no-occlusion. Then `bevy build web` and confirm the WebGPU bundle loads, geocodes, routes, and reports — with no route data leaving the browser (network tab clean).
