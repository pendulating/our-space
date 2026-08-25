---
title: Comparison — Ours vs Dahir
created: 2026-08-23
updated: 2026-08-23
type: comparison
tags: [comparison, academic-paper]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §3.1]
confidence: high
---

# Our Instrument vs Dahir et al.

| Dimension | Dahir et al. 2025 ([[dahir-nature-cities]]) | our-space instrument |
|---|---|---|
| Question | Where are the cameras? | Who is exposed, along their day? |
| Unit | Census block group (residence) | Block group × {walkshed, commute route, destination walkshed} |
| Sensor universe | Fixed building-mounted only | Fixed + ALPR + DOT + enforcement (+ mobile classes staged) |
| Detection | DeepLab V3+ on Street View; recall 0.63 | Merged human census + ML detections, deduped at 15 m ([[cross-source-dedup]]); recall measured by [[capture-recapture-undercount]] |
| Mobility | None | LODES OD + income-heterogeneous mode choice ([[mode-choice-mnl]]) |
| Headline finding | Diversity predicts cameras more than crime | Placement racialized (+10.15); lived exposure equalized (Gini 0.337 → 0.047) |
| Relationship | Anchor — we replicate and extend | Extension in activity space |

**Verdict:** consistent results on the shared question; the divergence appears only
when exposure is measured over trajectories — same geography, same year, divergent
answers ([[results-equalization]], [[point-vs-path]]).

Related: [[dahir-nature-cities]] · [[results-placement-bias]] · [[kwan-neap]]
