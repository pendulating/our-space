---
title: Results — Placement Bias
created: 2026-08-23
updated: 2026-08-23
type: result
tags: [result, equity]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §6.1, §6.2]
confidence: high
---

# Placement Bias Is Real, Racialized, and Robust

Pop-weighted `R_i` (cameras per 10-min walkshed): Manhattan **101.7** ·
Brooklyn **97.0** · Bronx **83.2** · Queens **63.6** · Staten Island **17.7**.
Citywide **82.0**. The top decile of people holds **22.6%** of all exposure.

## Need-neutral counterfactual (the headline)
Reallocate exposure ∝ need; population-weighted exposure conserved exactly.

| Group | Excess vs pop-neutral | vs ambient-neutral |
|---|---|---|
| Black-plurality BGs | **+11.2%** | **+32.9%** |
| Hispanic-plurality | +4.9% | +23.1% |
| White-plurality | −8.8% | −23.9% |
| Income Q5 | −8.1% | −31.9% |

## SDEM total impacts (`R_i`, cameras/SD)
%Hispanic direct +1.37 / indirect +4.14 / **total +5.51**. %Black total +3.75.
Most of the effect is the spatial spillover, not the direct term — surveillance
disparity is a property of *neighborhoods of neighborhoods*.

## The laundering result (§6.2, the normative core)
Control ladder on `R_i` (pop-weighted WLS, HC1), %Hispanic:
+10.15 (no controls) → +6.62 (+land use) → +4.06 (+crime) → **+2.99**
(+crime+311, ±0.50). But the mediators are themselves racialized
(`crime ~ %Hisp +0.31`). Controlling for crime **launders** the disparity; the
attenuation measures what the disorder narrative can rationalize, not what is
legitimate. Lead with the no-control number. Under Conley HAC, **+3.88 ±0.60**
survives regardless.

## %Black handling
Bivariate ≈ 0 (r = −0.03) — agrees with [[dahir-nature-cities]]. Net of land use
+4.21; joint spec +6.64; SDEM total +3.75. The gap is a suppression effect, not a
contradiction. Show both.

Related: [[results-laundering-ladder]] · [[results-robustness]] · [[capture-recapture-undercount]]
