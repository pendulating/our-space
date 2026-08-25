---
title: Exposure Instrument
created: 2026-08-23
updated: 2026-08-23
type: concept
tags: [instrument, method]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §4, docs/surveillance-exposure-disparity-plan.md]
confidence: high
---

# The Exposure Instrument

The instrument is C2, the methodological contribution. Unit of analysis: **2020
census block group**, population-weighted centroid. N = 6,587 block groups
(5 boroughs, 8.80M residents). Ecological; no individual claims.

## Three exposure terms
- **`R_i` residential** — 10-minute Dijkstra walkshed (walk speed 1.34 m/s) from
  the centroid. Count distinct camera groups whose frustum covers any sampled
  point on any reachable edge.
- **`A_i` activity space** — for residents of *i* working in *j*: exposure along
  the routed itinerary plus destination walkshed `R_j`, aggregated over *j*
  weighted by [[lodes-commute-data]] job flow and over mode:
  `A_i = Σ_j flow_ij · Σ_m P(m | i, g) · exposure_m(itinerary_ij)`.
- **`E_i` composite** — time-budget prior `(14·R_i + 1·commute + 9·A_dest)/24`.

## Subsystems
- Sensor geometry: [[sensor-model]]
- Sampling and the path argument: [[point-vs-path]]
- Cross-source deduplication: [[cross-source-dedup]]
- Undercount correction: [[capture-recapture-undercount]]
- Mode choice: [[mode-choice-mnl]]
- Estimation: [[spatial-econometrics]]

## Verisimilitude pass
2026-07-14/15, every headline was re-baked after end-to-end corrections (10 m
sampling stride, citywide enforcement refetch, directional ALPR sensors, OD
top_k=900, per-quintile ASC calibration). All headlines survived and strengthened.
See OUTLINE.md §9.

Related: [[contributions-c1-to-c6]] · [[epistemic-tiers]] · [[camera-layers]]
