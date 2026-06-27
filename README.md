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

## Documentation

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — system architecture: the
  simulation core + exposure model, the data pipeline and baked assets, and the
  app/rendering/web-build layer, with file/line references throughout.
- [`docs/DESIGN.md`](docs/DESIGN.md) — the "Survey of the Watched Commons" visual
  system: the warm-vs-cold palette, typography, confidence-tier coding, map-layer
  treatment, and interaction/motion.
- [`docs/PLAN.md`](docs/PLAN.md) — the original design, data-source decisions, and
  phased roadmap.

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
  from OpenStreetMap via Overpass) and the **real fixed-camera layers** — a unified
  street-CCTV census (**~4,400** cameras: Amnesty *Decode Surveillance NYC* crowdsourced
  counts **+** Dahir et al. ML detections, aggregated & de-duplicated), DeFlock ALPR
  readers, and **NYC DOT traffic cameras** (~370, locations only).
- ✅ End-to-end headless demo produces real exposure numbers for real walks.
- ✅ `app-interactive`: native Bevy map UI — click A/B to route, animated walker,
  camera dots + FOV wedges + ACE corridors over the street network, live
  "devices that saw you" panel. Runtime-verified (Metal/M2).
- ✅ **Phase 2**: time-of-day model + three mobile/ambient classes —
  **ACE bus cameras** (real MTA GTFS corridors, 20 routes), **rideshare cameras**
  (spatial field from real NYC TLC Uber/Lyft trip density per taxi zone × tunable
  fitting rate), and **smart glasses** (Tier-D scenario). Departure-hour scrubber +
  sliders re-evaluate the route live; per-source breakdown tagged by confidence tier.
- ✅ **Modes**: animated walk (cameras pulse + live capture count as the walker
  passes them) and a **10-minute walkshed** (cameras covering everywhere you could
  reach on foot from one point).
- ✅ **Phase 3**: native headless `batch` computes a **citywide exposure heatmap**
  (per-class expected devices/min for all 220k street edges, rstar-accelerated,
  ~0.1s) rendered in-app as a class-selectable heat overlay; plus an **equity
  overlay** — block-group **Shannon diversity** (Census ACS) as a choropleth,
  with detected cameras joined by point-in-polygon and the live diversity↔camera
  correlation, framed by the Dahir et al. finding.
- ✅ **Phase 4**: the app compiles to **WebGPU/WASM** (cross-platform asset
  loading via `AssetServer`; per-target Bevy features). `web/build.sh` produces a
  static `web/dist/` bundle (wasm-bindgen + wasm-opt) with `web/index.html` doing
  WebGPU-support detection, a loading screen, and the "estimate, not a
  surveillance map" + route-stays-client-side framing.
- ✅ **Basemap (web)**: the public build renders the **NYC Human Geography
  basemap** (ArcGIS vector tiles via MapLibre GL) as the ground layer beneath a
  **transparent** Bevy canvas. Bevy still owns all input; MapLibre is driven
  passively from the camera each frame (top-down, synced center + zoom). Native
  dev renders on a dark "caution" theme (noalprs-inspired: near-black zinc + hazard
  yellow), independent of the web shell's basemap.
- ✅ **Running ACE buses**: the ACE routes animate as **moving buses** along their
  real GTFS route shapes, counts scaled by headway/time-of-day (schedule-simulated
  from the GTFS *static* feed — fully offline; structured so a future GTFS-realtime
  source could drive live positions). They feed the narrative live tally.
- ✅ **Phase 5**: **animated ambient agents** — clay rideshare **dashcam vehicles**
  following real TLC trip-O-D routes and slate smart-glasses **pedestrians**
  wandering via graph random walks, on a fixed recycled entity pool (60 fps,
  no runtime routing). Density scales with the hour + sliders. A **dual exposure
  mode** lets you switch between the deterministic *Research estimate* (the
  reproducible Poisson figure) and a *Live walk* where the agents that actually
  pass you tally a stochastic "saw you" count — a Monte-Carlo sample of the
  same model.

## Quick start

Requires the Rust stable toolchain (`rustup`).

```sh
# Run the fast analytical test suite
cargo test -p sim-core --no-default-features

# Bake assets (fetch raw snapshots into data/snapshots/ first; see below)
# The trailing neighborhoods GeoJSON clips the graph to the Manhattan borough (drops
# the Bronx/Queens/Brooklyn bridge approaches the Overpass bbox pulls in). Omit it for
# the unclipped, region-wide network.
cargo run -p data-pipeline -- bake-graph   --overpass-json data/snapshots/osm/manhattan_walk.json assets/processed/graph_manhattan.postcard data/snapshots/neighborhoods/custom-pedia-cities-nyc-Mar2018.geojson
# Fixed CCTV: Amnesty Decode Surveillance NYC census + Dahir et al., aggregated & de-duplicated
cargo run -p data-pipeline -- bake-cctv    data/snapshots/amnesty/counts_per_intersections.csv data/snapshots/dahir/map_data.csv assets/processed/cameras_fixed.oscctv
# ALPR plate readers (DeFlock via OSM/Overpass: man_made=surveillance, surveillance:type=ALPR)
cargo run -p data-pipeline -- bake-alpr    data/snapshots/deflock/alpr.json assets/processed/alpr.osalpr
# NYC DOT traffic cameras (nyctmc.org feed; locations only — images are never used)
cargo run -p data-pipeline -- bake-dot     data/snapshots/dot/cameras.json  assets/processed/dot_cameras.osdot
# LinkNYC Wi-Fi kiosks (Socrata n6c5-95xh). A fixed point layer — not cameras, but
# each surveils when you connect to its Wi-Fi. Trailing GeoJSON clips to Manhattan.
#   curl -L "https://data.cityofnewyork.us/resource/n6c5-95xh.json?\$where=boro='Manhattan'&\$limit=5000&\$select=latitude,longitude,status,kiosk_type" -o data/snapshots/linknyc/kiosks_manhattan.json
cargo run -p data-pipeline -- bake-linknyc data/snapshots/linknyc/kiosks_manhattan.json assets/processed/linknyc.oslink data/snapshots/neighborhoods/custom-pedia-cities-nyc-Mar2018.geojson
# ACE corridors (teal). Trailing neighborhoods GeoJSON clips shapes to Manhattan
# (the M60-SBS otherwise runs out to LaGuardia); omit it to keep the full route.
cargo run -p data-pipeline -- bake-ace     data/snapshots/gtfs/gtfs_m data/snapshots/gtfs/ace_routes.json assets/processed/ace_corridors.postcard data/snapshots/neighborhoods/custom-pedia-cities-nyc-Mar2018.geojson
cargo run -p data-pipeline -- bake-equity  data/snapshots/census/blockgroups.geojson data/snapshots/census/acs.json data/snapshots/dahir/map_data.csv assets/processed/equity.postcard
# Rideshare-camera density (NYC TLC Uber/Lyft trips per taxi zone). Aggregate the
# remote Parquet with DuckDB, then bake against the taxi-zone polygons:
cargo run -p data-pipeline -- bake-dashcam-field data/snapshots/tlc/taxi_zones.geojson data/snapshots/tlc/zone_trips.csv assets/processed/dashcam_field.osfield
# Animated rideshare-vehicle routes (real TLC zone O-D, drives the moving dashcam
# agents). Routed over a **drive** graph — the OSM network minus pedestrian plazas
# (e.g. Broadway at Union Square) + access-restricted ways — so cars don't drive on
# pedestrianized streets. The drive graph is a build-time artifact (the app loads the
# walk graph for pedestrian routing); bake it first:
cargo run -p data-pipeline -- bake-graph --overpass-drive data/snapshots/osm/manhattan_walk.json data/snapshots/osm/graph_manhattan_drive.osgraph data/snapshots/neighborhoods/custom-pedia-cities-nyc-Mar2018.geojson
cargo run -p data-pipeline -- bake-vehicle-routes data/snapshots/osm/graph_manhattan_drive.osgraph data/snapshots/tlc/taxi_zones.geojson data/snapshots/tlc/zone_od.csv assets/processed/vehicle_routes.osroutes 1000
# Robotability field (speculative sidewalk delivery robots): the IRL-CT NYC
# Robotability Score per-sidewalk (0..1), gridded over the walk-graph extent — it
# weights where robots spawn + their exposure density. Fetch the scored GeoJSON,
# then bake against the graph:
#   curl -L https://raw.githubusercontent.com/IRL-CT/robotability/main/public/data/sidewalks.geojson -o data/snapshots/robotability/sidewalks.geojson
cargo run --release -p data-pipeline -- bake-robotability assets/processed/graph_manhattan.osgraph data/snapshots/robotability/sidewalks.geojson assets/processed/robotability.osrobot
# Tesla-camera field (private Tesla density by ZIP — Teslas run always-on Sentry/
# Autopilot cameras). Fetch NYC ZIP polygons + per-ZIP Tesla counts (NYS DMV
# w4pv-hbkt, make=TESLA, 5 boroughs), then bake:
#   curl -L https://raw.githubusercontent.com/fedhere/PUI2015_EC/master/mam1612_EC/nyc-zip-code-tabulation-areas-polygons.geojson -o data/snapshots/teslas/nyc_zips.geojson
#   curl -L "https://data.ny.gov/resource/w4pv-hbkt.csv?\$select=zip,count(*)+as+n&\$where=make='TESLA'+AND+county+in('NEW YORK','KINGS','QUEENS','BRONX','RICHMOND')&\$group=zip&\$limit=5000" -o data/snapshots/teslas/tesla_by_zip.csv
# (commercial Teslas ≈ 7% of TLC FHVs — identifiable by VIN prefix 5YJ/7SA — are
#  already represented by the rideshare-dashcam layer.)
cargo run -p data-pipeline -- bake-teslas data/snapshots/teslas/nyc_zips.geojson data/snapshots/teslas/tesla_by_zip.csv assets/processed/teslas.osteslas
# Automated photo-enforcement cameras (speed / bus-lane / red-light), located from
# NYC DOT 'PHOTO ENFORCED' street signs (Street Sign Work Orders qt6m-xctn). The
# fetch script queries Socrata, converts the state-plane coords (EPSG:2263) to
# WGS84, and dedups co-located signs → a lon,lat,subtype CSV:
#   python3 tools/fetch_enforcement.py   # -> data/snapshots/enforcement/enforcement_signs.csv
cargo run -p data-pipeline -- bake-enforcement data/snapshots/enforcement/enforcement_signs.csv assets/processed/enforcement.oscam
# Manhattan coastline outline (a visual frame around the street network), from the
# NYC Borough Boundaries (DCP). Keeps the borough's main landmass only:
#   curl -L "https://data.cityofnewyork.us/api/geospatial/gthc-hcne?method=export&format=GeoJSON" -o data/snapshots/boroughs/borough_boundaries.geojson
cargo run -p data-pipeline -- bake-borough data/snapshots/boroughs/borough_boundaries.geojson Manhattan assets/processed/borough_manhattan.osboro
# Building footprints (flat ground fabric). Fetch the ~45k Manhattan footprints
# (BIN 1xxxxxx) from the NYC Building dataset, then bake (clipped to the borough):
#   curl -L "https://data.cityofnewyork.us/resource/5zhs-2jue.geojson?\$select=the_geom,bin,height_roof,ground_elevation,feature_code&\$where=bin%20between%201000000%20and%201999999&\$limit=50000" -o data/snapshots/buildings/manhattan_footprints.geojson
cargo run -p data-pipeline -- bake-footprints data/snapshots/buildings/manhattan_footprints.geojson assets/processed/footprints.osbldg data/snapshots/neighborhoods/custom-pedia-cities-nyc-Mar2018.geojson
# Landmark 2.5D massings (recognizable buildings for orientation), from the NYC 3D
# Building Model (LoD2 CityGML). Download once (916 MB), extract the curated set via
# the Python tool (streams only the covering tiles), then bake the JSON:
#   curl -L https://s-media.nyc.gov/agencies/oti/DA_WISE_GML.zip -o data/snapshots/buildings/DA_WISE_GML.zip
#   python3 tools/extract_landmarks.py   # -> data/snapshots/buildings/landmarks_lod2.json
cargo run -p data-pipeline -- bake-landmarks data/snapshots/buildings/landmarks_lod2.json assets/processed/landmarks.oslmk

# Citywide exposure heatmap (per-edge intensities; arg = reference hour 0..23)
cargo run -p batch --release -- heatmap assets/processed/heatmap.postcard 17

# Headless end-to-end exposure demo (lat lon lat lon; HOUR=0..23 sets departure time)
HOUR=8 cargo run -p sim-core --example route_demo -- 40.7330 -73.9830 40.7160 -73.9810

# Interactive map (native window)
cargo run -p app-interactive

# Web build (WebGPU/WASM) — one-time tooling, then build + serve
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.125   # match the wasm-bindgen crate
brew install binaryen                              # wasm-opt
./web/build.sh
python3 -m http.server -d web/dist 8080           # open http://localhost:8080 (WebGPU browser)
```

### Hosting

`web/dist/` is a fully static bundle — deploy it to any static host with HTTPS
(WebGPU requires a secure context): Cloudflare Pages, GitHub Pages, Netlify,
Vercel. No server, API keys, or backend: the route is computed entirely in the
browser and never transmitted. (The one exception is **address lookup** — typing a
place sends *that query text*, and clicking the map to drop a pin sends *that one
coordinate*, to the key-free NYC GeoSearch API (`geosearch.planninglabs.nyc`, DCP's
PAD data) to resolve an address; the computed route itself still never leaves the
browser.) Note the ~46 MB payload (22 MB WASM + 24 MB baked layers); the page shows a
loading screen while it fetches.

**GitHub Pages (continuous deployment).** `.github/workflows/deploy.yml` publishes
the committed `web/dist/` to Pages on every push to `main` that touches it (or via
the Actions "Run workflow" button). Because the WASM build is slow and the baked
assets need gigabytes of raw NYC data, the bundle is **built locally and committed**
rather than reproduced in CI — the workflow only uploads and publishes it:

```sh
./web/build.sh            # rebuild web/dist/ (WASM + assets + index.html)
git add web/dist && git commit -m "Rebuild web bundle"
git push                  # → Actions deploys to https://pendulating.github.io/our-space/
```

One-time setup: in the repo's **Settings → Pages**, set **Source = GitHub Actions**.
The relative asset paths and Bevy's page-relative asset fetches work unchanged under
the `/our-space/` project subpath, so no base-path config is needed.

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

# Amnesty International "Decode Surveillance NYC" crowdsourced camera census
# (CC BY-NC-ND 4.0 — non-commercial use, attributed). The aggregated per-
# intersection counts file is the input to bake-cctv:
curl -L --create-dirs -o data/snapshots/amnesty/counts_per_intersections.csv \
  "https://raw.githubusercontent.com/amnesty-crisis-evidence-lab/decode-surveillance-nyc/main/data/counts_per_intersections.csv"

# Manhattan pedestrian network from OpenStreetMap via Overpass (ODbL):
#   POST the walk-network query (bbox 40.698,-74.022,40.882,-73.906) to
#   https://overpass-api.de/api/interpreter and save the JSON to
#   data/snapshots/osm/manhattan_walk.json

# NYC DOT traffic-camera locations (no open license — we keep only coordinates):
curl -L --create-dirs -o data/snapshots/dot/cameras.json \
  "https://webcams.nyctmc.org/api/cameras/"

# Taxi zone O-D matrix for the animated rideshare agents — aggregate a TLC HVFHV
# Parquet month with DuckDB, filtered to Manhattan zones (IDS = the 69 Manhattan
# LocationIDs from taxi_zones.geojson):
#   duckdb -c "INSTALL httpfs; LOAD httpfs;
#     COPY (SELECT PULocationID, DOLocationID, COUNT(*) AS trips
#           FROM read_parquet('https://d37ci6vzurychx.cloudfront.net/trip-data/fhvhv_tripdata_2024-06.parquet')
#           WHERE PULocationID IN (IDS) AND DOLocationID IN (IDS) AND PULocationID <> DOLocationID
#           GROUP BY 1,2 HAVING COUNT(*) >= 200 ORDER BY trips DESC)
#     TO 'data/snapshots/tlc/zone_od.csv' (HEADER);"
```

## Data sources & licenses

| Layer | Source | License |
|---|---|---|
| Walk graph | OpenStreetMap via Overpass API | ODbL 1.0 |
| Fixed CCTV | Amnesty Int'l *Decode Surveillance NYC* (crowdsourced census) **+** Dahir et al. 2025 (`map_data.csv`), aggregated & de-duplicated | CC BY-NC-ND 4.0 (Amnesty) + CC BY 4.0 (Dahir) |
| ALPR readers | DeFlock crowdsourced plate readers via OpenStreetMap (`surveillance:type=ALPR`) | ODbL 1.0 |
| DOT traffic cams | NYC DOT Traffic Management Center feed (`nyctmc.org`) — **locations only** | No open license; coords only, images never used |
| ACE corridors | MTA GTFS (route geometry) + data.ny.gov `ki2b-sg5y` (ACE route list) | MTA / OPEN-NY ToU |
| Block groups | Census TIGERweb (geometry) + Census Reporter API (ACS 5-year B03002, keyless) | Census public domain |
| Rideshare cams | NYC TLC High-Volume FHV trip records (Uber/Lyft), aggregated by taxi zone via DuckDB | NYC OpenData / TLC terms |

**Rideshare cameras** are framed as the in-vehicle cameras NYC requires for-hire
vehicles to carry; their **spatial density is real** (TLC trip distribution per
taxi zone, normalized to the median zone), while the camera-per-vehicle rate is a
tunable assumption. **Smart glasses** remain a fully speculative scenario layer.

**NYC DOT traffic cameras** (the ~370 Manhattan units behind `nyctmc.org`) have
**no open license**, and DOT has objected to reuse of the camera *images*. We
therefore ingest **only the published coordinates** — the `imageUrl` field is
never read, fetched, stored, or redistributed — and model them as a separate
low-frame-rate *monitoring* class (omnidirectional PTZ; locations are mapped, so
Tier A, no recall correction).

**Fixed CCTV** unifies two independent Google Street View censuses of the *same*
physical camera population, so they are de-duplicated rather than summed: Amnesty's
crowdsourced per-intersection counts (median over 3 volunteers, placed
omnidirectional at each intersection) form the base, plus any Dahir et al.
detection more than 50 m from an Amnesty camera-bearing intersection. These are
**sample-point estimates, not surveyed device coordinates**; the merged headline
is a direct census count, not the Dahir recall-corrected estimate.

The Amnesty data is **CC BY-NC-ND 4.0**: used here non-commercially (research),
attributed, and is the one layer whose source carries a NoDerivatives term — see
`docs/ARCHITECTURE.md` for the handling rationale.
