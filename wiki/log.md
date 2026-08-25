# Wiki Log

> Append-only. Format: `## [YYYY-MM-DD] action | subject`

## [2026-08-23] create | M1/M2/M3 implementation session
- M1 shipped (commit bc8371d): ACE + dashcam wired into bg-exposure / od-exposure*;
  `MobileLayers::load()`, `day_weights()`, `route_mobile_exposure()`; new columns
  m_ace_res/m_dash_res/m_ace_act/m_dash_act; zero blast radius verified old-vs-new.
- M2/M3 analysis shipped (commit f671c5e): tools/compounding.py,
  tools/incidence_inversion.py, exposure-table mobile columns,
  OURSPACE_EMIT_PAIRS emission, refresh_results.sh wiring.
- Cluster incident: bake was launched on the unicorn LOGIN node — wrong; node went
  offline mid-run and killed it. Matt corrected: login nodes are submission-only;
  heavy jobs via SLURM on compute nodes or locally. Handoff runbook written to
  docs/REBAKE_HANDOFF.md for the agent that runs the full re-bake.

## [2026-08-23] create | Wiki initialized
- Domain: our-space civic tool + FAccT '27 paper "Digital Eyes on the Street".
- Structure: SCHEMA.md, index.md, log.md; concepts/, entities/, comparisons/,
  queries/, mocs/, results/, deadlines/.
- Seeded from repo docs: OUTLINE.md, surveillance-exposure-disparity-plan.md,
  README.md, AGENTS.md, OCCLUSION_PLAN.md, facct27_cfp.txt, docs/.
- Created 30 pages: 4 MoCs, 16 concepts/results, 9 entities, 2 comparisons,
  1 query, 2 deadline pages. Verifier added at .tooling/verify_wiki.py
  (adapted from skimmers).
