#!/usr/bin/env bash
# ============================================================================
# Random-destination null model: route every null OD draw with the REAL
# `batch od-exposure-mnl` (tools/null_destinations.py explains the null).
#
#   uv run python3 tools/null_destinations.py make --draws 20      # writes the OD draws
#   sbatch tools/cluster_null_od.sh                                # array 0-19, 4 at a time
#   sbatch --export=ALL,NULL_MODE=bgs tools/cluster_null_od.sh     # the uniform-over-BG variant
#   uv run python3 tools/null_destinations.py analyze              # -> results/null_destinations.json
#
# Each task: one draw, ~1 h at 16 threads (about 3M pairs vs 1.9M observed; top_k is
# lifted so no null destination is truncated). Output: data/derived/null/A_i_mnl_null_<mode>_<d>.csv.
# The observed exposure files are never touched. `sbatch` here is an ssh shim to the login
# node, so variables must go through --export (an env prefix never reaches the job).
# ----------------------------------------------------------------------------
#SBATCH --job-name=ourspace-null-od
#SBATCH --array=0-19%4
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=16
#SBATCH --mem=24G
#SBATCH --time=03:00:00
#SBATCH --output=data/derived/logs/slurm-null-%A_%a.out
#SBATCH --partition=default_partition
#SBATCH --nodelist=lisbeth
export RAYON_NUM_THREADS="${SLURM_CPUS_PER_TASK:-16}"
# ============================================================================
set -euo pipefail
cd "${SLURM_SUBMIT_DIR:-$(dirname "$0")/..}"
[ -f Cargo.toml ] && [ -d crates/batch ] || { echo "error: run from the workspace root" >&2; exit 1; }

MODE="${NULL_MODE:-jobs}"
D=$(printf '%03d' "${SLURM_ARRAY_TASK_ID:-0}")
BATCH=target/release/batch
PROC=crates/app-interactive/assets/processed
SNAP=data/snapshots
NULL=data/derived/null
OD="$NULL/od_null_${MODE}_${D}.csv"
OUT="$NULL/A_i_mnl_null_${MODE}_${D}.csv"
LOG="data/derived/logs/null-od-${MODE}-${D}.log"

[ -x "$BATCH" ] || { echo "error: $BATCH missing; cargo build --release -p batch" >&2; exit 1; }
[ -f "$OD" ] || { echo "error: $OD missing; run null_destinations.py make" >&2; exit 1; }
unset OURSPACE_EMIT_PAIRS   # no per-pair file for null draws

echo "==> [$(date '+%F %T')] draw $D ($MODE): $OD -> $OUT  (threads $RAYON_NUM_THREADS)"
{ echo "### start: $(date '+%F %T')"; echo "### od: $OD"; } >"$LOG"
"$BATCH" od-exposure-mnl \
  "$PROC/graph_nyc.osgraph" "$PROC/graph_nyc_walk.osgraph" \
  "$SNAP/census/bg_centroids_nyc.csv" "$OD" "$SNAP/census/acs_nyc.csv" \
  "$SNAP/gtfs/subway/stations_subway.csv" "$OUT" 10 100000 data/derived/subway_nyc.ossub >>"$LOG" 2>&1
echo "### end: $(date '+%F %T')" >>"$LOG"
echo "==> [$(date '+%F %T')] done: $(wc -l <"$OUT") rows"
