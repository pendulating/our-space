#!/usr/bin/env bash
# Bake the citywide (five-borough) STATIC asset set for our·space and stage it
# into the app's canonical asset dir under `_nyc` names. The app loads these when
# launched with `?city=nyc` (web) or `OURSPACE_CITY=nyc` (native).
#
#   ./tools/bake_citywide.sh
#
# This is the static-first MVP set (per docs/SCALING.md): the citywide fixed-camera
# census + neighborhoods + the five-borough outline. Each is small (the whole set
# is ~2 MB), because the camera census — not download weight — is the cheap part.
#
# Footprints are lazy-loaded per borough in the citywide build (the full fabric is
# ~51 MB, so it never hits first-load). Baking them needs the citywide footprints
# GeoJSON (NYC Building Footprints, 5zhs-2jue, ~856 MB) — set FOOTPRINTS_GEOJSON to
# its path to (re)build the per-borough assets; otherwise that step is skipped:
#   curl -o nyc_footprints_all.geojson \
#     "https://data.cityofnewyork.us/api/geospatial/5zhs-2jue?method=export&format=GeoJSON"
#   FOOTPRINTS_GEOJSON=nyc_footprints_all.geojson ./tools/bake_citywide.sh
#
# Citywide ACE buses need all 5 borough GTFS feeds merged into one dir. Because the
# live MTA feeds only serve the current board, all 5 must be re-fetched together onto
# one board; the baked date must fall in their calendar range (a current-board
# weekday — NOT the Manhattan build's 2026-04-21). Set GTFS_NYC_DIR + ACE_DATE to
# build them; otherwise that step is skipped. To make the merged dir:
#   for b in m b q bx si; do curl -o g_$b.zip \
#     "https://rrgtfsfeeds.s3.amazonaws.com/gtfs_$b.zip"; unzip -oq g_$b.zip -d gtfs_$b; done
#   mkdir gtfs_nyc; for f in trips shapes stop_times calendar calendar_dates; do \
#     head -1 gtfs_m/$f.txt > gtfs_nyc/$f.txt; awk 'FNR>1' gtfs_{m,b,q,bx,si}/$f.txt >> gtfs_nyc/$f.txt; done
#   for f in routes stops; do head -1 gtfs_m/$f.txt > gtfs_nyc/$f.txt; \
#     awk -F, 'FNR>1 && !seen[$1]++' gtfs_{m,b,q,bx,si}/$f.txt >> gtfs_nyc/$f.txt; done
#   cp gtfs_m/agency.txt gtfs_nyc/
#
# NOT baked here (still Manhattan / pending data acquisition; see docs/SCALING.md):
#   - landmarks (3D building massing) → Manhattan-only curated set (bridges, below,
#     ARE baked here — they span boroughs and load in both builds)
#   - ALPR / enforcement / LinkNYC → Manhattan-only raw snapshots (re-fetch for citywide)
#   - the OSM street graph     → only needed for routing/walkshed/robotability/taxi (deferred)
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=target/release/data-pipeline
OUT=crates/app-interactive/assets/processed
SNAP=data/snapshots
FOOTPRINTS_GEOJSON="${FOOTPRINTS_GEOJSON:-}"
GTFS_NYC_DIR="${GTFS_NYC_DIR:-}"
ACE_DATE="${ACE_DATE:-20260707}"  # a current-board weekday the merged feeds cover

echo "==> cargo build -p data-pipeline --release"
cargo build -p data-pipeline --release >/dev/null

echo "==> fixed CCTV census (Amnesty + Dahir), citywide"
"$BIN" bake-cctv "$SNAP/amnesty/counts_per_intersections.csv" \
  "$SNAP/dahir/map_data.csv" "$OUT/cameras_fixed_nyc.oscctv" nyc

echo "==> DOT traffic cameras, citywide"
"$BIN" bake-dot "$SNAP/dot/cameras.json" "$OUT/dot_cameras_nyc.osdot" nyc

echo "==> five-borough outline (one asset, 5 main-landmass rings)"
"$BIN" bake-borough "$SNAP/boroughs/borough_boundaries.geojson" nyc "$OUT/borough_nyc.osboro"

echo "==> neighborhoods (already all five boroughs — shared with the Manhattan build)"
"$BIN" bake-neighborhoods "$SNAP/neighborhoods/custom-pedia-cities-nyc-Mar2018.geojson" \
  "$OUT/neighborhoods.osneigh"

echo "==> Manhattan footprints (RDP-simplified; the base + Manhattan lazy region)"
"$BIN" bake-footprints "$SNAP/buildings/manhattan_footprints.geojson" "$OUT/footprints.osbldg"

if [ -n "$FOOTPRINTS_GEOJSON" ] && [ -f "$FOOTPRINTS_GEOJSON" ]; then
  echo "==> per-borough footprints (lazy-loaded on zoom-in) from $FOOTPRINTS_GEOJSON"
  for boro in bronx brooklyn queens statenisland; do
    "$BIN" bake-footprints "$FOOTPRINTS_GEOJSON" "$OUT/footprints_${boro}.osbldg" "$boro"
  done
else
  echo "==> per-borough footprints SKIPPED (set FOOTPRINTS_GEOJSON to the citywide GeoJSON)"
fi

echo "==> citywide street network (NYC Street Centerline / CSCL, all five boroughs)"
# The Overpass route can't pull all of NYC in one query (public instances time out),
# so the citywide graph comes from NYC's own LION/CSCL centerline (Socrata inkn-q76z) —
# already split at intersections, no out-of-city spillover. ~30 MB GeoJSON; cached.
CSCL_GEOJSON="${CSCL_GEOJSON:-$SNAP/osm/cscl.geojson}"
if [ ! -f "$CSCL_GEOJSON" ]; then
  echo "    fetching CSCL centerline -> $CSCL_GEOJSON"
  mkdir -p "$(dirname "$CSCL_GEOJSON")"
  # trafdir (NV = non-vehicular) + nonped (D = park drives) drive the drivability
  # classifier; posted_speed feeds the time router. See graph_osm::bake_cscl.
  curl -sS --compressed -o "$CSCL_GEOJSON" \
    "https://data.cityofnewyork.us/resource/inkn-q76z.geojson?\$select=the_geom,rw_type,trafdir,nonped,posted_speed,full_street_name&\$limit=200000"
fi
# Parks GeoJSON (also baked as a layer below) doubles as the mask that drops car-free
# park-interior drives from the citywide graph — fetch it before the graph bake.
PARKS_GEOJSON="${PARKS_GEOJSON:-$SNAP/parks/parks.geojson}"
if [ ! -f "$PARKS_GEOJSON" ]; then
  echo "    fetching Parks Properties -> $PARKS_GEOJSON"
  mkdir -p "$(dirname "$PARKS_GEOJSON")"
  curl -sS --compressed -o "$PARKS_GEOJSON" \
    "https://data.cityofnewyork.us/resource/enfh-gkve.geojson?\$select=multipolygon,signname,typecategory,borough,acres&\$limit=10000"
fi
# NYC DOT Open Streets (car-free streets) — Layer 4 of the drivability mask: CSCL still
# codes pedestrianized blocks (Broadway plazas, West Village Open Streets) as vehicular.
OPENST_GEOJSON="${OPENST_GEOJSON:-$SNAP/open_streets/open_streets.geojson}"
if [ ! -f "$OPENST_GEOJSON" ]; then
  echo "    fetching Open Streets -> $OPENST_GEOJSON"
  mkdir -p "$(dirname "$OPENST_GEOJSON")"
  curl -sS --compressed -o "$OPENST_GEOJSON" \
    "https://data.cityofnewyork.us/resource/uiay-nctu.geojson?\$select=the_geom,appronstre,reviewstat,boroughname,apprdayswe&\$limit=2000"
fi
# `-` = no borough clip (keep all five boroughs); then parks + Open Streets masks.
"$BIN" bake-graph --cscl "$CSCL_GEOJSON" "$OUT/graph_nyc.osgraph" - "$PARKS_GEOJSON" "$OPENST_GEOJSON"

echo "==> iconic 3D bridges (decks + towers + cables) from named CSCL bridge segments"
# A second, name-carrying CSCL pull (the graph query drops stname_label): every
# segment whose label ends in 'BRG'. `generate_bridges.py` keeps the curated iconic
# set and emits landmark-schema JSON, so the existing landmark renderer draws them.
CSCL_BRIDGES="${CSCL_BRIDGES:-$SNAP/osm/cscl_bridges.geojson}"
if [ ! -f "$CSCL_BRIDGES" ]; then
  echo "    fetching named CSCL bridge segments -> $CSCL_BRIDGES"
  mkdir -p "$(dirname "$CSCL_BRIDGES")"
  curl -sS --compressed -o "$CSCL_BRIDGES" \
    "https://data.cityofnewyork.us/resource/inkn-q76z.geojson?\$select=the_geom,stname_label&\$where=stname_label%20like%20'%25BRG%25'&\$limit=50000"
fi
python3 tools/generate_bridges.py "$CSCL_BRIDGES" "$SNAP/bridges_landmarks.json"
"$BIN" bake-landmarks "$SNAP/bridges_landmarks.json" "$OUT/bridges.oslmk"

echo "==> parks (green context fabric) — Manhattan-clipped + citywide"
# NYC Parks Properties (enfh-gkve); the geometry column is `multipolygon`. Baked
# twice: Manhattan-only for the default build, all five boroughs for ?city=nyc.
PARKS_GEOJSON="${PARKS_GEOJSON:-$SNAP/parks/parks.geojson}"
if [ ! -f "$PARKS_GEOJSON" ]; then
  echo "    fetching Parks Properties -> $PARKS_GEOJSON"
  mkdir -p "$(dirname "$PARKS_GEOJSON")"
  curl -sS --compressed -o "$PARKS_GEOJSON" \
    "https://data.cityofnewyork.us/resource/enfh-gkve.geojson?\$select=multipolygon,signname,typecategory,borough,acres&\$limit=10000"
fi
# Clip parks to the shoreline (the borough-boundary land) so none spills into open
# water (e.g. Randall's/Ward's Island park extents overshoot the bank).
PARKS_BOUNDARY="$SNAP/boroughs/borough_boundaries.geojson"
"$BIN" bake-parks "$PARKS_GEOJSON" "$OUT/parks.ospark" M "$PARKS_BOUNDARY"
"$BIN" bake-parks "$PARKS_GEOJSON" "$OUT/parks_nyc.ospark" all "$PARKS_BOUNDARY"

echo "==> pedestrian plazas (concrete + hatch) — one asset, both builds"
# NYC DOT Pedestrian Plazas (k5k6-6jex); geometry column `the_geom`. Only ~93
# plazas citywide, so a single asset serves both the Manhattan and citywide builds.
PLAZAS_GEOJSON="${PLAZAS_GEOJSON:-$SNAP/plazas/plazas.geojson}"
if [ ! -f "$PLAZAS_GEOJSON" ]; then
  echo "    fetching Pedestrian Plazas -> $PLAZAS_GEOJSON"
  mkdir -p "$(dirname "$PLAZAS_GEOJSON")"
  curl -sS --compressed -o "$PLAZAS_GEOJSON" \
    "https://data.cityofnewyork.us/resource/k5k6-6jex.geojson?\$select=the_geom,plazaname,boroname&\$limit=5000"
fi
"$BIN" bake-plazas "$PLAZAS_GEOJSON" "$OUT/plazas.osplaza"

echo "==> institutions (schools + libraries) — Manhattan-only + citywide"
# NYC Facilities Database (ji82-xba5), pre-filtered to schools (K-12) + libraries.
# Baked twice: Manhattan-only for the default build, all five boroughs for ?city=nyc.
FACILITIES_JSON="${FACILITIES_JSON:-$SNAP/facilities/facilities.json}"
if [ ! -f "$FACILITIES_JSON" ]; then
  echo "    fetching Facilities Database -> $FACILITIES_JSON"
  mkdir -p "$(dirname "$FACILITIES_JSON")"
  curl -sS --compressed -o "$FACILITIES_JSON" \
    "https://data.cityofnewyork.us/resource/ji82-xba5.json?\$select=facname,latitude,longitude,boro,facgroup,facsubgrp,factype,address&\$where=facgroup%20in('SCHOOLS%20(K-12)','LIBRARIES')&\$limit=5000"
fi
"$BIN" bake-facilities "$FACILITIES_JSON" "$OUT/facilities.osfac" MANHATTAN
"$BIN" bake-facilities "$FACILITIES_JSON" "$OUT/facilities_nyc.osfac"

echo "==> citywide rideshare/taxi (all-borough TLC HVFHV, routed on the citywide graph)"
# The Manhattan taxi_day filtered HVFHV to Manhattan↔Manhattan; the citywide build
# wants all five boroughs. DuckDB streams one real day out of the published HVFHV
# Parquet (date-predicate pushdown → only the relevant row groups are fetched), then
# `bake-taxi-day` routes the top O-D pairs over graph_nyc.osgraph. Needs `duckdb`.
TAXI_TRIPS_NYC="${TAXI_TRIPS_NYC:-$SNAP/tlc/taxi_trips_all_nyc.csv}"
TAXI_PERMIN_NYC="${TAXI_PERMIN_NYC:-$SNAP/tlc/taxi_perminute_od_nyc.csv}"
TAXI_PARQUET="${TAXI_PARQUET:-https://d37ci6vzurychx.cloudfront.net/trip-data/fhvhv_tripdata_2024-06.parquet}"
TAXI_DAY_SRC="${TAXI_DAY_SRC:-2024-06-25}"  # a Tuesday (matches the citywide bus board's weekday)
if [ ! -f "$TAXI_TRIPS_NYC" ]; then
  if command -v duckdb >/dev/null 2>&1; then
    echo "    extracting $TAXI_DAY_SRC HVFHV trips (citywide) via DuckDB -> $TAXI_TRIPS_NYC"
    mkdir -p "$(dirname "$TAXI_TRIPS_NYC")"
    duckdb -c "INSTALL httpfs; LOAD httpfs;
      COPY (SELECT CAST(date_part('hour',pickup_datetime)*60+date_part('minute',pickup_datetime) AS INTEGER) AS pu_min,
                   PULocationID, DOLocationID,
                   CAST(GREATEST(1,date_diff('minute',pickup_datetime,dropoff_datetime)) AS INTEGER) AS dur_min,
                   ROUND(trip_miles,3) AS trip_miles,
                   CAST(trip_time AS INTEGER) AS trip_time
            FROM read_parquet('$TAXI_PARQUET')
            WHERE pickup_datetime >= TIMESTAMP '$TAXI_DAY_SRC 00:00:00'
              AND pickup_datetime <  TIMESTAMP '$TAXI_DAY_SRC 00:00:00' + INTERVAL 1 DAY
              AND PULocationID <> DOLocationID AND PULocationID <= 263 AND DOLocationID <= 263
            ORDER BY pu_min) TO '$TAXI_TRIPS_NYC' (HEADER);"
    # per-minute O-D aggregate, derived from the per-trip CSV (no second parquet scan)
    duckdb -c "COPY (SELECT pu_min, PULocationID, DOLocationID, COUNT(*) AS trips
                     FROM read_csv_auto('$TAXI_TRIPS_NYC') GROUP BY 1,2,3) TO '$TAXI_PERMIN_NYC' (HEADER);"
  else
    echo "    SKIPPED citywide taxi (no \`duckdb\` on PATH; install it or set TAXI_TRIPS_NYC)"
  fi
fi
if [ -f "$TAXI_TRIPS_NYC" ] && [ -f "$TAXI_PERMIN_NYC" ]; then
  "$BIN" bake-taxi-day "$OUT/graph_nyc.osgraph" "$SNAP/tlc/taxi_zones.geojson" \
    "$TAXI_PERMIN_NYC" "$TAXI_TRIPS_NYC" "$ACE_DATE" "$OUT/taxi_day_nyc.ostaxiday"
fi

if [ -n "$GTFS_NYC_DIR" ] && [ -d "$GTFS_NYC_DIR" ]; then
  echo "==> citywide ACE corridors + bus-day ($ACE_DATE) from $GTFS_NYC_DIR"
  "$BIN" bake-ace "$GTFS_NYC_DIR" "$SNAP/gtfs/ace_routes.json" "$OUT/ace_corridors_nyc.osace" nyc
  "$BIN" bake-bus-day "$GTFS_NYC_DIR" "$SNAP/gtfs/ace_routes.json" "$ACE_DATE" \
    "$OUT/bus_day_nyc.osbusday" nyc
else
  echo "==> citywide ACE buses SKIPPED (set GTFS_NYC_DIR to the merged 5-borough GTFS dir)"
fi

echo
echo "==> citywide asset sizes (footprints_* are lazy, NOT in first-load):"
du -h "$OUT/cameras_fixed_nyc.oscctv" "$OUT/dot_cameras_nyc.osdot" \
      "$OUT/borough_nyc.osboro" "$OUT/neighborhoods.osneigh" \
      "$OUT"/footprints*.osbldg 2>/dev/null | sort -h
echo
echo "Done. Launch the citywide build with ?city=nyc (web) or OURSPACE_CITY=nyc (native)."
