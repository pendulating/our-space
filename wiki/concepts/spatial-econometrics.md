---
title: Spatial Econometrics
created: 2026-08-23
updated: 2026-09-24
type: concept
tags: [econometrics, method]
sources: [tools/spatial_econometrics.py, docs/RELATED_METHODS.md §4-§7, §10]
confidence: high
---

# Estimator and Spatial Econometrics

Exposure and demographics are strongly autocorrelated: Moran's I on `R_i` residuals = +0.817
(p < .001), so iid OLS standard errors are invalid.

## Headline specification: SLX with Conley HAC SEs (decided 2026-09-24)

`R = Xβ + WXθ + ε`, W = Queen contiguity (islands attached), row-standardised. Total effect of a
covariate = β + θ. All SEs are Conley/Kelejian–Prucha HAC (triangular kernel; 2 km headline, 1 and
5 km reported).

**The rule, stated rather than data-mined.** `R_i` counts cameras in a 10-minute walkshed, and
adjacent walksheds share cameras by construction, so the dependence among neighbouring `R_i` is
*measurement structure*, not a causal ripple. An endogenous lag (SAR/SDM) reads that structure as
spillover and multiplies every effect by 1/(1−ρ) ≈ 11 at the fitted ρ = 0.91. Since the overlap is
something we built, it must not be used as an amplifier. SLX keeps only local spillovers (the
neighbours' covariates), leaves residual dependence to the HAC SEs, and its total effect needs no
multiplier (Halleck Vega & Elhorst 2015).

| `R_i` on … (cameras/SD, N = 5,547) | direct β | neighbours θ (WX) | total β+θ | t at 2 / 5 km |
|---|---|---|---|---|
| %Hispanic | +2.09 (0.71) | +10.14 (2.49) | **+12.22 (2.82)** | 4.3 / 3.8 |
| %Black | +0.97 (0.78) | +8.13 (1.85) | **+9.10 (2.00)** | 4.6 / 3.5 |
| income | +0.05 (0.63) | −1.47 (2.53) | −1.43 (3.00) | −0.5 / −0.4 |

Most of the association loads on the neighbourhood term, as a walkshed that spans into adjacent
block groups should; read the total, not the β/θ split (X and WX are collinear).

## What is reported alongside (robustness, not selection)

- AIC: OLS 55,272 · SLX 55,104 · SAR 47,046 · SEM 46,158 · **SDM 46,092** · SDEM 46,102. The AIC
  minimum is SDM; the table says so and says why it is not the headline.
- Anselin LM diagnostics on OLS residuals: robust LM-error 1,054 ≫ robust LM-lag 104 (error
  dependence, consistent with the measurement reading).
- LeSage–Pace totals: SDEM %Hisp +5.59 / %Black +3.48; SDM +14.30 / +11.71 (the ρ multiplier).
- OLS without WX, Conley HAC: %Hisp +8.98 (2.01); %Black +7.01 (1.68); income −0.46 (1.67).
- MAUP: %Hisp +8.71 (BG) / +9.84 (tract) / +10.51 (1 km grid): not a scale artifact.
- The crime/311 ladder is also run as SLX with HAC SEs (`ladder_hac[*].slx`): %Hispanic total
  +12.22 (2.82) → +6.86 (2.90) with crime → +6.20 (2.80; 3.30 at 5 km) with crime + 311, i.e.
  t = 2.2 / 1.9 at rung 4. Robust as a total; marginal once the (plausibly post-treatment) crime
  and 311 controls enter.

## History

- Until 2026-09-23 the script's "Conley HAC" SEs were classical iid SEs (spreg ignores `gwk`
  without `robust='hac'`); fixed, SEs grew ~3.5×.
- 2026-08-25 re-bake flipped the AIC minimum from SDEM to SDM (ΔAIC 10); the caption still said
  "AIC-selected error specification" until 2026-09-24, when the selection was moved to the stated
  rule above. See `docs/RELATED_METHODS.md` §10.

Related: [[results-placement-bias]] · [[capture-recapture-undercount]] · [[exposure-instrument]]
