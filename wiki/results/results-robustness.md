---
title: Results — Robustness
created: 2026-08-23
updated: 2026-08-23
type: result
tags: [result]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §6.4, §9, §12]
confidence: high
---

# Robustness Ledger

- **Undercount** ([[capture-recapture-undercount]]): correction rescales the
  outcome ~1.83×; scale-free correlation barely moves (+0.204 → +0.217).
  Non-differential detection (p=0.88/0.88/0.42). A robustness result, not
  amplification.
- **Sample selection**: ACS suppresses income in ~1,040 BGs. Ladders now take
  complete cases per spec; demographic ladders recover to N=6,393. Headline barely
  moves (+9.57 → +10.15 canonical).
- **Occlusion**: null; see [[occlusion-null]]. Sensitivity over assumed range
  switches on only past ~60 m.
- **MAUP**: BG +8.26 / tract +9.14 / 1 km grid +9.57.
- **Walk graph fix**: citywide CSCL graph was a drive network; dual walk
  classifier built (`nonped=V` drop). Effect: mean `R_i` +0.7%; %Hispanic β
  +9.57 → +10.15. Reported, not quietly adopted.
- **Subway sweep — the honest weak spot.** Demographic correlations flip sign at
  s ≈ 27.6 (%Hispanic), 138 (income), 185 (%White); central model s ≈ 12. The tiny
  activity-space %Hispanic residual (−0.77) sits below its flip but its sign is
  not robustly identified across the plausible subway-camera range. Disclose
  before a reviewer finds it.
- **ALPR-vs-DOT contrast**: reinstatable since the verisimilitude re-bake lifted
  pop-weighted `R_alpr` from 0.06 to ~1.5. Annotated, not redesigned.

Full reviewer-objection table lives in OUTLINE.md §12.

Related: [[results-placement-bias]] · [[results-equalization]] · [[open-work-before-submission]]
