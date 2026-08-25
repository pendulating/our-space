---
title: Spatial Econometrics
created: 2026-08-23
updated: 2026-08-23
type: concept
tags: [econometrics, method]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §4.6]
confidence: high
---

# Estimator and Spatial Econometrics

Exposure and demographics are strongly autocorrelated: Moran's I on `R_i`
residuals = +0.824 (p=.001), so OLS standard errors are invalid.

Full ML ladder selected by AIC: OLS 55267 > SAR 46915 > SEM 46006 > SDM 45947 >
**SDEM 45945 (selected)** — the Spatial Durbin Error model, confirming the Anselin
LM diagnostic (robust-error ≫ robust-lag).

Reporting rules:
- LeSage–Pace direct / indirect / total impacts ([[results-placement-bias]]).
- Conley spatial-HAC SEs as cross-check.
- MAUP: re-estimate at tract + 1 km grid (BG +8.26 / tract +9.14 / grid +9.57 —
  not a scale artifact).
- Report SDM's inflated +13.24 as a ρ-explosion artifact (ρ=0.912) and say why it
  was rejected.

Known gap: `spatial_econometrics.py` runs at N=5,547 (income-complete); an
income-free variant would be a worthwhile robustness check. See
[[open-work-before-submission]].

Related: [[results-placement-bias]] · [[capture-recapture-undercount]] · [[exposure-instrument]]
