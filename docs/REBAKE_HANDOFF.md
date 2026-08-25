# NOTE TO THE NEXT AGENT — full re-bake handoff

**Context:** the M1 mobile-columns code and M2/M3 analysis scripts are DONE, COMMITTED,
AND PUSHED (commits `bc8371d`, `f671c5e` on `main`). What remains is mechanical: run
the exposure re-bake on real data, rsync results back, run the analysis half.

## Hard rules (Matt's instructions — do not violate)

1. **NEVER run computational workloads on the unicorn login node.**
   Login nodes are for editing and SLURM submission only. The 2026-08-23 attempt that
   launched `rebake.sh` directly on unicorn-login-04 was a mistake; the node also went
   offline mid-run, killing the bake. Heavy jobs go to **compute nodes via SLURM**
   (`sbatch`), or locally on Matt's laptop (~2.5 h).
2. If SLURM is unavailable or you can't verify compute-node allocation, **stop and do
   other tasks instead of improvising cluster access.**

## What to run

The launcher is `tools/cluster_rebake.sh`. It is idempotent — safe to re-run over the
dead partial state from 2026-08-23 (`~/our-space` on unicorn already has the repo at
`f671c5e`-or-later, plus the full rsync'd `data/snapshots/` and
`crates/app-interactive/assets/processed/`; only its stale `rebake.sh` /
`rebake_console.log` / partial `data/derived/exposure/*.csv` need overwriting).

```sh
# On the cluster (SLURM header in the script is ready — uncomment the #SBATCH block):
cd ~/our-space
git pull                      # must be at f671c5e or later
sbatch tools/cluster_rebake.sh
# ~15 min on the 196-core node

# Then back on the laptop:
rsync -avz unicorn:~/our-space/data/derived/ data/derived/
```

## New outputs to expect (vs the last bake)

| File | New columns | Source |
|---|---|---|
| `data/derived/exposure/R_i_bg_nyc.csv` | `m_ace_res`, `m_dash_res` | bg-exposure |
| `A_i_drive_bg_nyc.csv` | `m_ace_act`, `m_dash_act` | od-exposure |
| `A_i_modal_bg_nyc.csv` | `m_ace_act`, `m_dash_act` | od-exposure-modal |
| `A_i_mnl_bg_nyc.csv` | `m_ace_act`, `m_dash_act` | od-exposure-mnl |
| `exposure_table_nyc.csv` | `M_ace_res`, `M_dash_res`, `M_ace_act_mnl`, `M_dash_act_mnl`, `M_ace_act_modal`, `M_dash_act_modal` | exposure-table |
| `od_pairs_mnl_nyc.csv` (NEW file) | per-pair rows for M3 | see below |

For the M3 pair file, add `OURSPACE_EMIT_PAIRS=$EXP/od_pairs_mnl_nyc.csv` to the env
of the `od-exposure-mnl` step (the cluster script does not yet set it — either export
it before sbatch with `--export`, or edit the step). Without it, M3 cannot run.

## After the bake lands locally

```sh
tools/refresh_results.sh    # runs ALL analyses incl. the two new ones:
                            #   compounding.py          -> results/compounding.json        (M2)
                            #   incidence_inversion.py  -> results/incidence_inversion.json (M3)
                            #                             (needs OURSPACE_PAIRS=data/derived/exposure/od_pairs_mnl_nyc.csv)
```

Then read:
- `results/compounding.json` → pop-weighted corr(R_i, M_i^act). >0 compounds, <0 offsets.
  This is OUTLINE §8's "single statistic that settles the thesis."
- `results/incidence_inversion.json` → share of mobile captures generated outside the
  worker's home borough. Expected headline shape: "Manhattan's dashcams generate X% of
  their captures on Bronx/Queens residents."

Sanity gates before trusting numbers: compounding.py refuses pre-M1 tables (blank M
columns); incidence_inversion.py refuses pair files missing columns. Zero blast radius
on legacy columns was verified twice (old-vs-new binary comparisons, 0 diffs).

Sweep knobs if asked: `OURSPACE_DASH_PEN`, `OURSPACE_DASH_CAP` (defaults 0.40/0.40).

Wiki context: `wiki/deadlines/simulation-risks.md` (risk register A1 = this work),
`wiki/concepts/compounding-test-m1-m3.md` (test definitions).
