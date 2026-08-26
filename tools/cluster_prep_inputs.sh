#!/usr/bin/env bash
# ============================================================================
# SLURM submission header — regenerate every gitignored input the full re-bake
# needs, from scratch, on a compute node (never on a login node):
#
#   mkdir -p data/derived/logs && sbatch tools/cluster_prep_inputs.sh
#
# Produces:
#   data/snapshots/**            raw snapshots (fetch_snapshots.py + GTFS merge)
#   crates/app-interactive/assets/processed/*.osgraph/.oscctv/...  baked assets
#
# Follow with:  sbatch --dependency=afterok:<jobid> tools/cluster_rebake.sh
# ----------------------------------------------------------------------------
#SBATCH --job-name=ourspace-prep
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=16
#SBATCH --mem=48G                     # footprints GeoJSON parse is the peak; lisbeth's free pool is ~52G
#SBATCH --time=08:00:00
#SBATCH --output=data/derived/logs/slurm-prep-%j.out
#SBATCH --partition=default_partition
#SBATCH --nodelist=lisbeth
# ============================================================================
set -euo pipefail

# batch/data-pipeline hardcode repo-relative asset paths, so run from the root.
# NOTE: under sbatch the script runs from SLURM's spool dir ($0 is a copy), so
# anchor on the submit directory instead.
cd "${SLURM_SUBMIT_DIR:-$(dirname "$0")/..}"
ROOT="$(pwd)"
[ -f Cargo.toml ] && [ -d crates/batch ] || {
  echo "error: must run from the workspace root; got $ROOT" >&2
  exit 1
}

PROC=crates/app-interactive/assets/processed
SNAP=data/snapshots
GTFS=$SNAP/gtfs
DERIV=data/derived

mkdir -p "$PROC" "$SNAP" "$DERIV/logs" "$DERIV/exposure"

echo "==> workspace: $ROOT  ($(hostname), $(date '+%F %T'))"

# --- prerequisites ----------------------------------------------------------

# Census API key (ACS fetch). fetch_census.py auto-loads ./.env; export it too
# so any subprocess sees it.
if [ -f .env ]; then set -a; . ./.env; set +a; fi
: "${CENSUS_API_KEY:?CENSUS_API_KEY not set — add it to .env}"

# duckdb CLI (LODES aggregation + TLC trip extraction).
if ! command -v duckdb >/dev/null 2>&1; then
  if [ -x "$HOME/.local/bin/duckdb" ]; then
    export PATH="$HOME/.local/bin:$PATH"
  else
    echo "==> installing duckdb CLI -> ~/.local/bin"
    mkdir -p "$HOME/.local/bin"
    curl -sSL -o /tmp/duckdb.zip \
      https://github.com/duckdb/duckdb/releases/latest/download/duckdb_cli-linux-amd64.zip
    unzip -oq /tmp/duckdb.zip -d "$HOME/.local/bin"
    rm -f /tmp/duckdb.zip
    export PATH="$HOME/.local/bin:$PATH"
  fi
fi
echo "==> duckdb $(duckdb --version | cut -d' ' -f1)"

# Toolchain discovery: sbatch may start with a minimal environment, so locate
# uv + cargo explicitly instead of trusting inherited PATH.
for p in "$HOME/.cargo/bin" /share/ju/matt/.cargo/bin; do
  [ -x "$p/cargo" ] && export PATH="$p:$PATH"
done
command -v cargo >/dev/null 2>&1 || { echo "ERROR: cargo not found" >&2; exit 1; }
echo "==> cargo $(cargo --version)"

if ! command -v uv >/dev/null 2>&1; then
  [ -x /share/pierson/matt_ai/bin/uv ] && export PATH="/share/pierson/matt_ai/bin:$PATH"
fi
if ! command -v uv >/dev/null 2>&1; then
  echo "==> installing uv -> ~/.local/bin"
  curl -LsSf https://astral.sh/uv/install.sh | env CARGO_HOME="$HOME/.cargo" sh
  export PATH="$HOME/.local/bin:$PATH"
fi
command -v uv >/dev/null 2>&1 || { echo "ERROR: uv not found" >&2; exit 1; }
echo "==> $(uv --version)"

# Python env (uv-managed; .venv lives in the shared checkout).
uv run python3 -c 'import pandas, pyarrow; print("==> uv env ok")'

# --- 1) raw snapshots -------------------------------------------------------
# Everything the exposure path reads. Fetched per-dataset with retries: the
# upstreams (Socrata, Census API, cloudfront, s-media) drop connections under
# sustained pulls, and a corrupt partial must be cleared or the idempotent
# skip will keep "succeeding" on garbage.
DATASETS="amnesty dahir deflock dot enforcement boroughs neighborhoods osm_cscl parks plazas open_streets census lodes gtfs ace_routes crime tlc buildings"
echo "==> fetching snapshots: $DATASETS"
for ds in $DATASETS; do
  try=0
  until uv run python3 tools/fetch_snapshots.py "$ds"; do
    try=$((try+1))
    if [ "$try" -ge 3 ]; then
      if [ "$ds" = buildings ]; then
        # Soft-fail: only its landmarks extraction is flaky; the re-bake needs
        # just manhattan_footprints.geojson, covered by the footprints.osbldg
        # bake below plus the final verify gate.
        echo "!! buildings landmarks failing after 3 attempts — continuing (not needed by the re-bake)" >&2
        break
      fi
      echo "!! dataset $ds failed after 3 attempts" >&2
      exit 1
    fi
    echo "==> $ds failed (attempt $try/3); clearing data/snapshots/$ds and retrying"
    rm -rf "data/snapshots/$ds"
    sleep 20
  done
done

# --- 2) merged 5-borough GTFS (ACE corridors input) --------------------------
# Live MTA feeds only serve the current board, so all 5 must be fetched together;
# ACE_DATE below must be a weekday inside that board's calendar range.
ACE_DATE="${ACE_DATE:-20260825}"   # Tue of the week the feeds were fetched
if [ ! -f "$GTFS/gtfs_nyc/calendar.txt" ]; then
  echo "==> merging 5-borough GTFS into $GTFS/gtfs_nyc"
  for b in m b q bx si; do
    [ -f "$GTFS/gtfs_$b.zip" ] || curl -sSL -o "$GTFS/gtfs_$b.zip" \
      "https://rrgtfsfeeds.s3.amazonaws.com/gtfs_$b.zip"
    rm -rf "$GTFS/gtfs_$b"
    unzip -oq "$GTFS/gtfs_$b.zip" -d "$GTFS/gtfs_$b"
  done
  mkdir -p "$GTFS/gtfs_nyc"
  for f in trips shapes stop_times calendar calendar_dates; do
    head -1 "$GTFS/gtfs_m/$f.txt" > "$GTFS/gtfs_nyc/$f.txt"
    awk 'FNR>1' "$GTFS"/gtfs_{m,b,q,bx,si}/"$f.txt" >> "$GTFS/gtfs_nyc/$f.txt"
  done
  for f in routes stops; do
    head -1 "$GTFS/gtfs_m/$f.txt" > "$GTFS/gtfs_nyc/$f.txt"
    awk -F, 'FNR>1 && !seen[$1]++' "$GTFS"/gtfs_{m,b,q,bx,si}/"$f.txt" >> "$GTFS/gtfs_nyc/$f.txt"
  done
  cp "$GTFS/gtfs_m/agency.txt" "$GTFS/gtfs_nyc/"
fi

# --- 3) build the two headless crates ----------------------------------------
# Never a bare `cargo build`: app-interactive pulls the whole Bevy/GUI stack.
if [ ! -x target/release/batch ] || [ ! -x target/release/data-pipeline ]; then
  echo "==> cargo build --release -p batch -p data-pipeline"
  # The shared RUSTUP_HOME's pinned-stable sync can wedge on its wasm32
  # component; fall back to the installed 1.95 toolchain (MSRV is 1.88, and
  # wasm32 only matters for web builds, not these headless crates).
  if ! cargo build --release -p batch -p data-pipeline; then
    echo "==> default toolchain failed; retrying with RUSTUP_TOOLCHAIN=1.95"
    RUSTUP_TOOLCHAIN=1.95-x86_64-unknown-linux-gnu \
      cargo build --release -p batch -p data-pipeline
  fi
fi
BIN=target/release/data-pipeline

# --- 4) bake the fixed-sensor + graph assets ---------------------------------
echo "==> bake-cctv (Amnesty + Dahir, citywide)"
"$BIN" bake-cctv "$SNAP/amnesty/counts_per_intersections.csv" \
  "$SNAP/dahir/map_data.csv" "$PROC/cameras_fixed_nyc.oscctv" nyc

echo "==> bake-dot (citywide traffic cameras)"
"$BIN" bake-dot "$SNAP/dot/cameras.json" "$PROC/dot_cameras_nyc.osdot" nyc

echo "==> bake-alpr (DeFlock)"
"$BIN" bake-alpr "$SNAP/deflock/alpr.json" "$PROC/alpr.osalpr"

echo "==> bake-enforcement (DOT bus-lane/camera signs)"
"$BIN" bake-enforcement "$SNAP/enforcement/enforcement_signs.csv" "$PROC/enforcement.oscam"

echo "==> bake-graph --cscl (drive network; parks + Open Streets masks)"
"$BIN" bake-graph --cscl "$SNAP/osm/cscl.geojson" "$PROC/graph_nyc.osgraph" \
  - "$SNAP/parks/parks.geojson" "$SNAP/open_streets/open_streets.geojson"

echo "==> bake-graph --cscl-walk (pedestrian network)"
"$BIN" bake-graph --cscl-walk "$SNAP/osm/cscl.geojson" "$PROC/graph_nyc_walk.osgraph"

# --- 5) occlusion fabric (footprints) ----------------------------------------
echo "==> bake-footprints (Manhattan base layer)"
"$BIN" bake-footprints "$SNAP/buildings/manhattan_footprints.geojson" "$PROC/footprints.osbldg"

CITYWIDE_FOOTPRINTS="${CITYWIDE_FOOTPRINTS:-$SNAP/buildings/nyc_footprints_all.geojson}"
if [ ! -f "$CITYWIDE_FOOTPRINTS" ]; then
  echo "==> downloading citywide footprints (~856 MB) -> $CITYWIDE_FOOTPRINTS"
  curl -sSL --compressed -o "$CITYWIDE_FOOTPRINTS" \
    "https://data.cityofnewyork.us/api/geospatial/5zhs-2jue?method=export&format=GeoJSON"
fi
for boro in bronx brooklyn queens statenisland; do
  echo "==> bake-footprints ($boro lazy region)"
  "$BIN" bake-footprints "$CITYWIDE_FOOTPRINTS" "$PROC/footprints_${boro}.osbldg" "$boro"
done

# --- 6) mobile layers (M1 observed classes) ----------------------------------
# NOTE: batch reads the UNSUFFIXED paths (const ACE_PATH / DASHCAM_PATH in
# crates/batch/src/main.rs), so the citywide layers must land exactly there.
echo "==> bake-ace (citywide corridors from merged GTFS; board date $ACE_DATE)"
"$BIN" bake-ace "$GTFS/gtfs_nyc" "$SNAP/gtfs/ace_routes.json" \
  "$PROC/ace_corridors.osace" nyc
cp "$PROC/ace_corridors.osace" "$PROC/ace_corridors_nyc.osace"

echo "==> bake-dashcam-field (rideshare dashcam field from TLC zone trips)"
"$BIN" bake-dashcam-field "$SNAP/tlc/taxi_zones.geojson" \
  "$SNAP/tlc/zone_trips.csv" "$PROC/dashcam_field.osfield"

# --- 7) verify ---------------------------------------------------------------
echo "==> verifying baked inputs:"
rc=0
for f in \
  "$PROC/graph_nyc.osgraph" \
  "$PROC/graph_nyc_walk.osgraph" \
  "$PROC/cameras_fixed_nyc.oscctv" \
  "$PROC/dot_cameras_nyc.osdot" \
  "$PROC/alpr.osalpr" \
  "$PROC/enforcement.oscam" \
  "$PROC/footprints.osbldg" \
  "$PROC/footprints_bronx.osbldg" \
  "$PROC/footprints_brooklyn.osbldg" \
  "$PROC/footprints_queens.osbldg" \
  "$PROC/footprints_statenisland.osbldg" \
  "$PROC/ace_corridors.osace" \
  "$PROC/dashcam_field.osfield" \
  "$SNAP/census/bg_centroids_nyc.csv" \
  "$SNAP/census/acs_nyc.csv" \
  "$SNAP/lodes/bg_od_nyc.csv" \
  "$SNAP/gtfs/subway/stations_subway.csv" \
  "$SNAP/gtfs/siferry/routes.txt" \
  "$SNAP/crime/nypd_points.csv" \
  "$SNAP/crime/nyc311_disorder_points.csv"; do
  if [ -s "$f" ]; then du -h "$f"; else echo "MISSING/EMPTY: $f" >&2; rc=1; fi
done
[ "$rc" -eq 0 ] || { echo "!! input verification FAILED" >&2; exit 1; }

echo
echo "==> prep done ($(date '+%F %T')). Submit the re-bake:"
echo "    sbatch --dependency=afterok:\$SLURM_JOB_ID tools/cluster_rebake.sh"
