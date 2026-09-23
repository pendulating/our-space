#!/usr/bin/env bash
# ============================================================================
# Re-bake the exposure steps that consume the rideshare dashcam field, after the
# field itself was rebuilt citywide (2026-09-23; it was Manhattan-only before, see
# docs/RELATED_METHODS.md). Submit with `sbatch tools/cluster_dashcam_rebake.sh`.
#
# Steps (all read crates/app-interactive/assets/processed/dashcam_field.osfield via
# MobileLayers::load()): bg-exposure -> od-exposure (drive) -> od-exposure-modal ->
# od-exposure-mnl (+ per-pair emission for M3) -> exposure-table.
# Fixed-camera columns must come out byte-identical to the previous bake; only the
# m_dash_* columns (and m_ace_* only if the ACE input changed) may move.
# Then run tools/refresh_results.sh.
#
# STEPS selects a subset (space-separated, in this order): bg drive modal mnl table.
# Pass it through sbatch's own --export: on this cluster `sbatch` is an ssh shim to the
# login node (~/.local/bin/_slurm_ssh_shim), so a plain `STEPS=... sbatch` env prefix
# never reaches the job.
#   sbatch --export=ALL,STEPS="mnl table" tools/cluster_dashcam_rebake.sh
# ----------------------------------------------------------------------------
#SBATCH --job-name=ourspace-dashcam-rebake
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=16
#SBATCH --mem=32G
#SBATCH --time=04:00:00
#SBATCH --output=data/derived/logs/slurm-dashcam-%j.out
#SBATCH --partition=default_partition
#SBATCH --nodelist=lisbeth
export RAYON_NUM_THREADS="${SLURM_CPUS_PER_TASK:-16}"
# ============================================================================
set -euo pipefail
cd "${SLURM_SUBMIT_DIR:-$(dirname "$0")/..}"
[ -f Cargo.toml ] && [ -d crates/batch ] || { echo "error: run from the workspace root" >&2; exit 1; }

BATCH=target/release/batch
PROC=crates/app-interactive/assets/processed
SNAP=data/snapshots
DERIV=data/derived
EXP=$DERIV/exposure
LOGS=$DERIV/logs
DRIVE=$PROC/graph_nyc.osgraph
WALK=$PROC/graph_nyc_walk.osgraph
CENT=$SNAP/census/bg_centroids_nyc.csv
OD=$SNAP/lodes/bg_od_nyc.csv
ACS=$SNAP/census/acs_nyc.csv
STATIONS=$SNAP/gtfs/subway/stations_subway.csv
SUBWAY=$DERIV/subway_nyc.ossub
WALK_MIN=10
TOP_K=900
export OURSPACE_EMIT_PAIRS="$EXP/od_pairs_mnl_nyc.csv"
STEPS="${STEPS:-bg drive modal mnl table}"
want() { case " $STEPS " in *" $1 "*) return 0;; *) return 1;; esac; }

[ -x "$BATCH" ] || { echo "error: $BATCH missing; build with: cargo build --release -p batch" >&2; exit 1; }
[ -f "$PROC/dashcam_field.osfield" ] || { echo "error: dashcam_field.osfield missing (bake-dashcam-field first)" >&2; exit 1; }
[ -f "$SUBWAY" ] || { echo "error: $SUBWAY missing (bake-subway first)" >&2; exit 1; }
mkdir -p "$LOGS"

# Keep the previous bake for the byte-identity check on the fixed-camera columns.
PREV=$EXP/prev_$(date +%Y%m%d_%H%M)
mkdir -p "$PREV"
for f in R_i_bg_nyc.csv A_i_drive_bg_nyc.csv A_i_modal_bg_nyc.csv A_i_mnl_bg_nyc.csv exposure_table_nyc.csv od_pairs_mnl_nyc.csv; do
  [ -f "$EXP/$f" ] && cp -n "$EXP/$f" "$PREV/$f" || true
done

run_step() {
  local name=$1; shift
  local log="$LOGS/$name.log"
  echo "==> [$(date '+%F %T')] $name  (log: $log)"
  { echo "### step: $name"; echo "### start: $(date '+%F %T')"; echo "### threads: $RAYON_NUM_THREADS"; echo "### cmd: $*"; echo; } >"$log"
  local rc=0
  "$@" >>"$log" 2>&1 || rc=$?
  { echo; echo "### end: $(date '+%F %T')  (exit $rc)"; } >>"$log"
  [ "$rc" -eq 0 ] || { echo "!! $name FAILED (exit $rc) -- see $log" >&2; exit "$rc"; }
}

want bg    && run_step bg-exposure "$BATCH" bg-exposure "$WALK" "$CENT" "$EXP/R_i_bg_nyc.csv" "$WALK_MIN"
want drive && run_step od-exposure-drive "$BATCH" od-exposure "$DRIVE" "$WALK" "$CENT" "$OD" "$EXP/A_i_drive_bg_nyc.csv" "$WALK_MIN" "$TOP_K"
want modal && run_step od-exposure-modal "$BATCH" od-exposure-modal "$DRIVE" "$WALK" "$CENT" "$OD" "$ACS" "$STATIONS" "$EXP/A_i_modal_bg_nyc.csv" "$WALK_MIN" "$TOP_K" "$SUBWAY"
want mnl   && run_step od-exposure-mnl "$BATCH" od-exposure-mnl "$DRIVE" "$WALK" "$CENT" "$OD" "$ACS" "$STATIONS" "$EXP/A_i_mnl_bg_nyc.csv" "$WALK_MIN" "$TOP_K" "$SUBWAY"
want table && run_step exposure-table "$BATCH" exposure-table "$CENT" "$EXP/R_i_bg_nyc.csv" "$EXP/A_i_drive_bg_nyc.csv" "$ACS" "$EXP/exposure_table_nyc.csv" "$EXP/A_i_modal_bg_nyc.csv" "$EXP/A_i_mnl_bg_nyc.csv"

echo "==> byte-identity check on fixed-camera columns vs $PREV"
# sbatch shells do not source the login profile, so `uv` is usually not on PATH; try the
# known install locations, and fall back to the venv's python so the check still runs.
UV=$(command -v uv || ls -d "$HOME/.local/bin/uv" /share/pierson/matt_ai/bin/uv 2>/dev/null | head -1 || true)
if [ -n "$UV" ]; then PY=("$UV" run python3); else PY=(.venv/bin/python3); fi
"${PY[@]}" - "$EXP" "$PREV" <<'EOF'
import sys, pandas as pd
exp, prev = sys.argv[1], sys.argv[2]
for f in ["R_i_bg_nyc.csv", "A_i_drive_bg_nyc.csv", "A_i_modal_bg_nyc.csv", "A_i_mnl_bg_nyc.csv"]:
    a, b = pd.read_csv(f"{exp}/{f}"), pd.read_csv(f"{prev}/{f}")
    fixed = [c for c in a.columns if not c.startswith("m_")]
    same = a[fixed].equals(b[fixed])
    moved = [c for c in a.columns if c.startswith("m_") and not a[c].equals(b[c])]
    print(f"  {f}: fixed columns identical={same}; mobile columns changed={moved}")
import os
if os.path.exists(f"{prev}/od_pairs_mnl_nyc.csv"):
    a, b = pd.read_csv(f"{exp}/od_pairs_mnl_nyc.csv"), pd.read_csv(f"{prev}/od_pairs_mnl_nyc.csv")
    common = [c for c in b.columns if c in a.columns]
    print(f"  od_pairs: rows {len(a)} vs {len(b)}; shared columns identical={a[common].equals(b[common])}; new columns={[c for c in a.columns if c not in b.columns]}")
EOF
echo "==> done. Now: tools/refresh_results.sh"
exit 0
