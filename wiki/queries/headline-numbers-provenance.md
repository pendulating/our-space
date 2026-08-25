---
title: Query — Where Do the Headline Numbers Live?
created: 2026-08-23
updated: 2026-08-23
type: query
tags: [result, facct27]
sources: [data/derived/results/, facct27_digital_eyes_on_the_street/OUTLINE.md §9]
---

# Where Do the Headline Numbers Live?

Filed 2026-08-23. Answer to "which file do I regenerate / trust for number X?"

All persisted results live in `data/derived/results/` (JSON + a README mapping each
file to its paper section). Regeneration commands are in that README.

| Number | File / tool |
|---|---|
| Gini collapse, top-decile shares, P90/P10 | `inequality_stats.py` → `inequality_stats.json`, `lorenz_curves.csv` ([[results-equalization]]) |
| Control ladder | `crime_ladder.py` ([[results-laundering-ladder]]) |
| Recall bootstrap decomposition | `sweep_recall.py` ([[capture-recapture-undercount]]) |
| SDEM / AIC ladder / MAUP | `spatial_econometrics.py` (stdout + .txt log; JSON persistence open) |
| Occlusion null + sensitivity | `occlusion_summary.py` → `occlusion.json`; sweep via `OURSPACE_RANGE_SCALE` |
| Walk-graph effect | `network_effect.json` |
| Paper tables T1–T3 | `tools/make_tables.py --check` → `facct27_digital_eyes_on_the_street/tables/` |

Rule when numbers conflict across docs: OUTLINE.md §9 corrections are newest and
win; mark the older doc `contested`. See SCHEMA.md Update Policy.

Related: [[results-equalization]] · [[figures-and-tables]] · [[open-work-before-submission]]
