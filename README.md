# our-space

An interactive + batch geospatial simulation of the cameras and sensing devices
entering NYC's public space — fixed CCTV, ACE bus-lane cameras, dashcams, and
smart glasses. You enter a walking route (A→B) on the real Manhattan street
network and it estimates **how many cameras could capture you** and how often,
with time-of-day mattering. Built in Rust with the [Bevy](https://bevy.org)
engine; the public build targets the browser via WASM + WebGPU.

It is an **honest estimate tool**, not a surveillance map or an evasion guide.
Every number is a model estimate with stated provenance and uncertainty. The
route never leaves your browser.

See [`docs/PLAN.md`](docs/PLAN.md) for the full design, data-source decisions,
and roadmap.

## Workspace layout

```
crates/
  sim-core/        Render-agnostic core: ENU projection, FOV/occlusion geometry,
                   the exposure model, routable graph + A*, and the simulation loop.
                   Pure Rust (no Bevy); optional `ecs` feature adds a Bevy layer.
  data-pipeline/   Native CLI that bakes raw NYC open data into compact assets.
  app-interactive/ Bevy app (native dev window now; WebGPU WASM later).  [Phase 1 WIP]
  batch/           Native headless host for citywide heatmaps.           [Phase 3]
assets/processed/  Baked, client-loadable assets (postcard binaries).
data/snapshots/    Dated raw-data snapshots + provenance (payloads gitignored).
```

## Status

- ✅ Verified toolchain + dependency stack (single Bevy 0.18; builds native **and** `wasm32`).
- ✅ `sim-core`: projection, FOV wedge + 2D line-of-sight occlusion, exposure model
  (headline "cameras that saw you" + expected capture-events + % surveilled), A*
  routing with snap/validate, position-over-time. 25 unit tests.
- ✅ `data-pipeline`: bakes the **real Manhattan walk graph** (151k nodes / 221k edges,
  from OpenStreetMap via Overpass) and the **real fixed-CCTV layer** (217 Manhattan
  cameras from Dahir et al., CC BY 4.0, recall-corrected).
- ✅ End-to-end headless demo produces real exposure numbers for real walks.
- ✅ `app-interactive`: native Bevy map UI — click A/B to route, animated walker,
  camera dots + FOV wedges + ACE corridors over the street network, live
  "devices that saw you" panel. Runtime-verified (Metal/M2).
- ✅ **Phase 2**: time-of-day model + three mobile/ambient classes —
  **ACE bus cameras** (real MTA GTFS corridors, 20 routes), **dashcams** (Tier-C
  modeled field), and **smart glasses** (Tier-D scenario). Departure-hour
  scrubber + penetration/adoption sliders re-evaluate the route live; per-source
  breakdown tagged by confidence tier with Poisson P(seen).
- ⏳ Phase 3 (batch heatmap + equity overlay), Phase 4 (WebGPU web embed).

## Quick start

Requires the Rust stable toolchain (`rustup`).

```sh
# Run the fast analytical test suite
cargo test -p sim-core --no-default-features

# Bake assets (fetch raw snapshots into data/snapshots/ first; see below)
cargo run -p data-pipeline -- bake-graph   --overpass-json data/snapshots/osm/manhattan_walk.json assets/processed/graph_manhattan.postcard
cargo run -p data-pipeline -- bake-cameras data/snapshots/dahir/map_data.csv                       assets/processed/cameras_fixed.postcard
cargo run -p data-pipeline -- bake-ace     data/snapshots/gtfs/gtfs_m data/snapshots/gtfs/ace_routes.json assets/processed/ace_corridors.postcard

# Headless end-to-end exposure demo (lat lon lat lon; HOUR=0..23 sets departure time)
HOUR=8 cargo run -p sim-core --example route_demo -- 40.7330 -73.9830 40.7160 -73.9810

# Interactive map (native window)
cargo run -p app-interactive
```

### Interactive controls

- **Left-click** the map to set the start (A), click again for the destination (B).
- **Right-drag** to pan, **scroll** to zoom.
- Side panel shows the headline count, capture-events, % surveilled, and provenance;
  toggle camera fields of view; "Reset route" to start over.

### Fetching raw data

```sh
# Dahir et al. fixed-camera detections (CC BY 4.0)
curl -L --create-dirs -o data/snapshots/dahir/map_data.csv \
  "https://stacks.stanford.edu/file/druid:jr882ny4955/map_data.csv"

# Manhattan pedestrian network from OpenStreetMap via Overpass (ODbL):
#   POST the walk-network query (bbox 40.698,-74.022,40.882,-73.906) to
#   https://overpass-api.de/api/interpreter and save the JSON to
#   data/snapshots/osm/manhattan_walk.json
```

## Data sources & licenses

| Layer | Source | License |
|---|---|---|
| Walk graph | OpenStreetMap via Overpass API | ODbL 1.0 |
| Fixed CCTV | Dahir et al. 2025, Stanford Digital Repository (`map_data.csv`) | CC BY 4.0 |
| ACE corridors | MTA GTFS (route geometry) + data.ny.gov `ki2b-sg5y` (ACE route list) | MTA / OPEN-NY ToU |

Dashcam and smart-glasses layers are **scenario models** (Tier C/D), not datasets —
their intensities are user-tunable assumptions, surfaced as such in the UI.

Camera points are Google Street View **sample-points where a camera was detected**
(detector recall ~0.63), not surveyed device locations — surfaced in-app as a
modeled estimate with an uncertainty band.
