---
title: Mode Choice MNL
created: 2026-08-23
updated: 2026-08-23
type: concept
tags: [mode-choice, method, instrument]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §4.4]
confidence: high
---

# Mode Choice (Income-Heterogeneous MNL)

Utilities in dollars, walk as reference:
`U_m = −(VOT_g · time_m + cost_m) + ASC_m`, `VOT_g = income_g / (2080·60) · 0.5`
$/min. Costs: drive $0.20/km + $4.00 fixed; transit $2.90; walk $0.

## The load-bearing fix
A single global ASC lets pure VOT invent a monotonic income→drive gradient
(Q1 0.03 → Q5 0.71) that contradicts NYC's real pattern. Calibrating ASCs **per
income quintile** (incremental logit, 40 iterations, against ACS B08301 targets)
reproduces the true shape: drive share rises Q1→Q4 (0.25→0.41) then falls at Q5 to
0.30 as the affluent shift to walking (0.17). VOT then only shapes within-group
distance substitution.

## Transit routing
Transit legs route over a GTFS headway-based subway graph (`sim_core::subway`,
baked by `data-pipeline bake-subway`). Weekday AM peak; wait = headway/2 capped at
15 min; **common-lines pooling** (Chriqui–Robillard, α=0.5) so waits pool over the
combined attractive line set; real Staten Island Ferry from NYC DOT ferry GTFS.
All 245,520 station pairs reachable. Exact boardings set the per-trip camera
complement (n = boardings − 1, capped 3). Same-station trips are infeasible.

Income-suppressed home BGs (973 of 6,520) are not imputed; they use observed ACS
mode shares.

Related: [[results-equalization]] · [[lodes-commute-data]] · [[exposure-instrument]]
