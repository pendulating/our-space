---
title: Cross-Source Dedup
created: 2026-08-23
updated: 2026-08-23
type: concept
tags: [method, data-layer, instrument]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §4.3]
confidence: high
---

# Cross-Source Deduplication

Four independent censuses see overlapping physical cameras: Amnesty crowdsource,
Dahir ML detections, DeFlock ALPR, DOT traffic. Union-find proximity clustering at
15 m merges **only across sources**. One physical camera = one count, with a
`confirmed` flag when any surveyed source attests it.

Most camera papers have one source and cannot do this. Dedup is also what makes
[[capture-recapture-undercount]] possible — two censuses of the same population.

In the app, the merged CCTV headline (~4,400 Manhattan fixed cameras) is a direct
census count, not the Dahir recall-corrected estimate. See README "Data sources &
licenses".

Related: [[exposure-instrument]] · [[camera-layers]] · [[data-confidence-tiers]]
