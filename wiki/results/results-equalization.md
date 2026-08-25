---
title: Results — Equalization
created: 2026-08-23
updated: 2026-08-23
type: result
tags: [result, equity]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §6.3]
confidence: high
---

# The Equalization Finding

Computed by `tools/inequality_stats.py` →
`data/derived/results/inequality_stats.json`. All population-weighted — statements
about *people*, not block groups.

| Measure | Gini | Top-10% share | P90/P10 | Mean |
|---|---|---|---|---|
| `R_i` residential | **0.337** | **22.6%** | **6.73×** | 82.0 |
| `A_drive` | 0.048 | 11.4% | 1.25× | 143.9 |
| `A_modal` | 0.040 | 11.3% | 1.20× | 140.0 |
| **`A_mnl`** | **0.047** | **11.5%** | **1.24×** | 136.5 |
| `E_i` composite | 0.197 | 17.2% | 2.60× | 90.2 |

> **Gini 0.337 → 0.047 — an 86% collapse in inequality.** The most-exposed tenth
> lives under 6.73× as many cameras as the least-exposed tenth, but moves through
> only 1.24× as many. And 11.5% is barely above the 10% perfect equality would
> produce.

Income quintiles (pop-weighted means): `R_i` 92.6 / 85.8 / 77.3 / 66.2 / 83.3;
`A_mnl` **133.6** / 132.5 / 133.8 / 136.1 / **143.9**.

Corroboration from the ladder ([[results-placement-bias]]): %Hispanic falls from
+10.15 on `R_i` to −0.77 on `A_mnl`. Mobility raises exposure ~1.7× for everyone.
Staten Island extreme: `R_i` 17.7 → `A_drive` 135 (7.6× amplification). Robust to
routing (r=0.86–0.91 across mode variants).

## Why it happens, and why it is not good news
Every mode is watched: MTA cameras in every station and car (~12/trip
pop-weighted), ALPR and DOT on the roads, ~29k CCTV on sidewalks. There is no
unsurveilled way to cross New York City. Equal exposure is not equity — it is
saturation. The place-based analysis sees a gradient; the trajectory-based analysis
sees a ceiling. The injustice relocates from lived exposure to placement intent
([[results-placement-bias]]).

Anchor in [[kwan-neap]]: this is a predicted NEAP result, not an awkward null.

Related: [[thesis-in-one-sentence]] · [[point-vs-path]] · [[mode-choice-mnl]]
