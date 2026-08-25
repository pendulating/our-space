---
title: Capture-Recapture Undercount
created: 2026-08-23
updated: 2026-08-23
type: concept
tags: [undercount, method, instrument]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §4.5, §6.1]
confidence: high
---

# Capture–Recapture Undercount

Two independent enumerations of the same city → Chapman estimator:
`N̂ = (n₁+1)(n₂+1)/(m+1) − 1`, match radius 50 m.

## Findings
- Census recall ≈ 50% (`N̂ = 28,643` true sites vs 14,357 observed; 95% boot
  [45.8, 54.4]%). Constant `sim_core::CENSUS_RECALL = 0.501`.
- Detection is uniform across boroughs (40.6–53.0%).
- Detection model `Logit(Amnesty finds Dahir-known camera) ~ demographics +
  density`: no demographic gradient (%Hisp p=0.88, %Black p=0.88, income p=0.42).
  The undercount is **non-differential**.

## Correct claim
Correcting rescales the outcome ~1.83×; a coefficient in cameras/SD grows
mechanically with it. `tools/sweep_recall.py` decomposes this over 2,000 bootstrap
draws: the %Hispanic correlation moves +0.204 → +0.217 (+6.4% genuine), while the
coefficient "doubles" as a units artifact. A non-differential undercount cannot
create or destroy a relative disparity.

> The census misses roughly half of NYC's cameras. Detection shows no demographic
> gradient. Correction rescales exposure and leaves the relative disparity
> essentially unchanged. The disparity is not an artifact of who gets counted.

Lead with the scale-free correlation, not the coefficient. Caveat stated openly:
both censuses derive from Google Street View → positive dependence inflates
overlap *m* → biases `N̂` down. It is a conservative lower bound.

Related: [[results-placement-bias]] · [[results-robustness]] · [[sensor-model]]
