---
title: Dahir Nature Cities
created: 2026-08-23
updated: 2026-08-23
type: entity
tags: [academic-paper, surveillance]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §3.1]
confidence: high
---

# Dahir et al., Nature Cities 2025 (The Anchor)

**Dahir, Sheng, Yao, Goel & Hwang. "Surveillance camera prevalence and racial
diversity in ten US cities." *Nature Cities* 2(7): 662–670, 2025.**
doi:10.1038/s44284-025-00274-2. Precursor: Sheng, Yao & Goel, "Surveilling
Surveillance," AIES '21, doi:10.1145/3461702.3462525.

## Their finding — get it right
Cameras peak in racially **diverse / gentrifying** neighborhoods (~25% Black,
~50% white). Diversity predicts cameras more strongly than crime does.
Conditional on crime, %non-Hispanic Black is *negatively* associated with cameras.
Longitudinally, an influx of white residents into non-white neighborhoods drives
camera increases. Their reading: cameras as an instrument of social control wielded
by white gentrifiers.

## Our relationship: extension, not refutation
Our raw %Black coefficient is −1.48 (bivariate r ≈ −0.03) — consistent with theirs.
We are a replication-plus-extension on NYC, one of their ten cities. What we add:
(a) a strong %Hispanic gradient they did not foreground, (b) exposure in activity
space ([[results-equalization]]), (c) the mobile fleets they explicitly exclude.

Their stated limitations are our opening: fixed structures only; no doorbell
cameras; "we may underestimate"; no ownership or function data. Method for the
comparison table: DeepLab V3+/EfficientNet-b3 on Street View; 100k road points per
city; every positive human-verified; recall 0.63; zero-inflated Poisson at block
group. No mobility, no OD, no mobile cameras.

Killer figure: reproduce their BG measure on NYC side-by-side with our
activity-space measure. Same geography, same year, divergent answers.

Related: [[related-work-map]] · [[cross-source-dedup]] · [[results-placement-bias]]
