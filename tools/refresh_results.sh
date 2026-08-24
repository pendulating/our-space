#!/usr/bin/env bash
# Rebuild every number in the paper, from the baked exposure CSVs through to the LaTeX.
#
# The exposure CSVs themselves come from `batch bg-exposure` / `od-exposure*` /
# `counterfactual` / `covariates`, which are slow (tens of minutes) and are NOT re-run here;
# this script is the cheap downstream half. Run it after any re-bake.
#
# Order matters: exposure-table joins the per-measure CSVs, the Python analyses read that
# join, and make_tables.py reads the analyses. Nothing here is idempotent-unsafe, so it is
# always safe to re-run.
#
#   tools/refresh_results.sh
#
# `make_tables.py --check` in CI will then fail if anyone hand-edits a .tex table or forgets
# to re-run this after a bake.
set -euo pipefail
cd "$(dirname "$0")/.."

B=./target/release/batch
E=data/derived/exposure
R=data/derived/results
SNAP=data/snapshots
PROC=crates/app-interactive/assets/processed

DRIVE=$PROC/graph_nyc.osgraph
WALK=$PROC/graph_nyc_walk.osgraph

[ -x "$B" ] || cargo build --release -p batch

echo "==> 1/4  join the per-measure CSVs into the block-group table"
"$B" exposure-table \
  "$SNAP/census/bg_centroids_nyc.csv" \
  "$E/R_i_bg_nyc.csv" \
  "$E/A_i_drive_bg_nyc.csv" \
  "$SNAP/census/acs_nyc.csv" \
  "$E/exposure_table_nyc.csv" \
  "$E/A_i_modal_bg_nyc.csv" \
  "$E/A_i_mnl_bg_nyc.csv"

echo "==> 2/4  network descriptives (drive vs walk)"
"$B" graph-stats "$DRIVE" "$WALK" "$R/graph_stats.json" >/dev/null

# NOTE: `batch covariates` takes the 311 file as the NINTH argument, AFTER walk_min. Omitting
# it is silent: req311_wsh comes out all-zero and the rung-4 ladder design goes singular, which
# surfaces as an opaque LinAlgError three scripts downstream. It is re-run by the bake, not here,
# but the correct invocation is recorded so nobody rediscovers this:
#
#   batch covariates <walk.osgraph> <centroids> <od> <stations> <crime.csv> <out> 10 <311.csv>
#                                                                                 ^^  ^^^^^^^^^
#                                                                            walk_min  311 LAST

echo "==> 3/4  analyses"
uv run python tools/inequality_stats.py       # -> inequality_stats.json, lorenz_curves.csv
uv run python tools/compounding.py            # -> compounding.json (M2: corr(R_i, M_i^act);
                                              #    needs the M1 mobile columns in the table)
uv run python tools/incidence_inversion.py    # -> incidence_inversion.json (M3; needs
                                              #    OURSPACE_PAIRS=... od_pairs CSV emitted via
                                              #    OURSPACE_EMIT_PAIRS=<path> batch od-exposure-mnl)
uv run python tools/crime_ladder.py           # -> crime_ladder.json
uv run python tools/analyze_counterfactual.py # -> counterfactual.json
uv run python tools/capture_recapture.py      # -> capture_recapture.json (+ recall_draws)
uv run python tools/sweep_recall.py           # -> recall_sensitivity.json  (needs the above)
uv run python tools/undercount_spatial.py     # -> detection_model.json (non-differential check)
uv run python tools/sweep_subway_cameras.py   # -> subway_sweep.json (linear in commute_subway)
uv run python tools/population_mixed.py       # -> population_mixed.json (A_lived: commuter/
                                              #    non-commuter mixture; the C4 scope check)
uv run python tools/network_effect.py         # -> network_effect.json (walk-vs-drive; needs
                                              #    R_i_bg_nyc_drive.csv, a bake-level artifact --
                                              #    regeneration command in the script header)
uv run python tools/occlusion_summary.py      # -> occlusion.json (needs the occl_audit_walk_x*.csv
                                              #    + R_i_bg_nyc_walk_free.csv + occl_probe_walk.txt
                                              #    bake artifacts; commands in the script header)
uv run python tools/spatial_econometrics.py > "$R/spatial_econometrics.txt"

echo "==> 4/4  LaTeX tables + macros"
uv run python tools/make_tables.py

echo
echo "Done. Every table and every inline statistic in the paper now reflects"
echo "the current contents of $E/."
