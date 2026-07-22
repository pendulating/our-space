#!/usr/bin/env bash
# ============================================================================
# SLURM header — UNCOMMENT this block if the cluster uses SLURM, then submit
# with `sbatch tools/cluster_rebake.sh`. On a bare-SSH cluster leave it
# commented and just run `./tools/cluster_rebake.sh`. This script never calls
# sbatch itself; SLURM reads the #SBATCH directives at submit time.
# ----------------------------------------------------------------------------
# #SBATCH --job-name=ourspace-rebake
# #SBATCH --nodes=1
# #SBATCH --ntasks=1
# #SBATCH --cpus-per-task=196          # the 196-CPU node; the GPU node buys nothing (CPU-only rayon)
# #SBATCH --mem=64G                    # CPU-bound; real peak is single-digit GB, this is headroom
# #SBATCH --time=02:00:00              # ~15 min at 196 cores; generous ceiling
# #SBATCH --output=data/derived/logs/slurm-%j.out
# #SBATCH --partition=cpu              # set to your CPU partition / --nodelist=<big-cpu-node>
# # Match rayon to the SLURM allocation (otherwise rayon grabs every core on the node):
# export RAYON_NUM_THREADS="${SLURM_CPUS_PER_TASK:-$RAYON_NUM_THREADS}"
# ============================================================================
#
# Re-bake the heavy surveillance-exposure path on a compute cluster:
#   bake-subway  ->  od-exposure-mnl  ->  od-exposure-modal   (sequential)
#
# See docs/CLUSTER.md for the full runbook (what to rsync, env knobs, the
# lighter batch steps, and the Python analysis half that runs on the laptop).
#
#   ./tools/cluster_rebake.sh
#   RAYON_NUM_THREADS=190 ./tools/cluster_rebake.sh   # cap rayon; unset = all cores
set -euo pipefail

# batch hardcodes repo-relative asset paths (const *_PATH in crates/batch/src/main.rs),
# so every command must run from the workspace root.
cd "$(dirname "$0")/.."
ROOT="$(pwd)"
[ -f Cargo.toml ] && [ -d crates/batch ] || {
  echo "error: must run from the workspace root; got $ROOT" >&2
  exit 1
}

BATCH=target/release/batch
DP=target/release/data-pipeline

PROC=crates/app-interactive/assets/processed
SNAP=data/snapshots
DERIV=data/derived
EXP=$DERIV/exposure
LOGS=$DERIV/logs

# Inputs (see docs/CLUSTER.md "Data to copy to the cluster")
DRIVE=$PROC/graph_nyc.osgraph
WALK=$PROC/graph_nyc_walk.osgraph
CENT=$SNAP/census/bg_centroids_nyc.csv
OD=$SNAP/lodes/bg_od_nyc.csv
ACS=$SNAP/census/acs_nyc.csv
STATIONS=$SNAP/gtfs/subway/stations_subway.csv
SUB_GTFS=$SNAP/gtfs/subway            # bake-subway input (GTFS tables)
SUBWAY=$DERIV/subway_nyc.ossub        # bake-subway output; od-exposure input

# Canonical run parameters (verified against the USAGE strings in batch/src/main.rs).
# top_k=900 = full LODES job-flow coverage — NOT the USAGE default of 100.
WALK_MIN=10
TOP_K=900

mkdir -p "$EXP" "$LOGS"

echo "==> workspace: $ROOT"
echo "==> RAYON_NUM_THREADS=${RAYON_NUM_THREADS:-<all cores>} (rayon defaults to every core)"

# Build the two headless crates if the release binaries are missing. Never build the
# whole workspace: app-interactive pulls the full Bevy/GUI stack (see docs/CLUSTER.md).
if [ ! -x "$BATCH" ] || [ ! -x "$DP" ]; then
  echo "==> cargo build --release -p batch -p data-pipeline"
  cargo build --release -p batch -p data-pipeline
fi

# Run one step, streaming stdout+stderr to $LOGS/<name>.log with start/end timestamps.
# On failure: record the exit code in the log, point at it, and abort (set -e friendly).
run_step() {
  local name=$1; shift
  local log="$LOGS/$name.log"
  echo "==> [$(date '+%F %T')] $name  (log: $log)"
  {
    echo "### step: $name"
    echo "### start: $(date '+%F %T')"
    echo "### threads: RAYON_NUM_THREADS=${RAYON_NUM_THREADS:-<all cores>}"
    echo "### cmd: $*"
    echo
  } >"$log"
  local rc=0
  "$@" >>"$log" 2>&1 || rc=$?
  {
    echo
    echo "### end: $(date '+%F %T')  (exit $rc)"
  } >>"$log"
  if [ "$rc" -ne 0 ]; then
    echo "!! $name FAILED (exit $rc) -- see $log" >&2
    exit "$rc"
  fi
}

# 1) Subway all-pairs router matrix (seconds). Rebuilt here so the launcher is
#    self-contained: it needs only the GTFS snapshot, not a pre-rsynced .ossub.
#    bake-subway's optional 6th arg [ferry_gtfs_dir] is omitted, so it defaults to
#    the subway dir's sibling data/snapshots/gtfs/siferry (the real NYC DOT SI Ferry
#    GTFS). That dir must be in the rsync'd snapshots (see docs/CLUSTER.md); if it is
#    missing, bake-subway still succeeds but falls back -- loudly -- to the old
#    parameterized ferry pseudo-route.
run_step bake-subway "$DP" bake-subway "$SUB_GTFS" "$SUBWAY"

# 2) MNL (income-heterogeneous mode choice) activity-space exposure.
run_step od-exposure-mnl "$BATCH" od-exposure-mnl \
  "$DRIVE" "$WALK" "$CENT" "$OD" "$ACS" "$STATIONS" \
  "$EXP/A_i_mnl_bg_nyc.csv" "$WALK_MIN" "$TOP_K" "$SUBWAY"

# 3) Modal (observed-ACS-share) activity-space exposure.
run_step od-exposure-modal "$BATCH" od-exposure-modal \
  "$DRIVE" "$WALK" "$CENT" "$OD" "$ACS" "$STATIONS" \
  "$EXP/A_i_modal_bg_nyc.csv" "$WALK_MIN" "$TOP_K" "$SUBWAY"

echo "==> done. Outputs in $EXP/ ; logs in $LOGS/"
echo "    rsync data/derived/ back to the laptop, then run tools/refresh_results.sh there."
