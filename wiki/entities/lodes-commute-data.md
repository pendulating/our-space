---
title: LODES Commute Data
created: 2026-08-23
updated: 2026-08-23
type: entity
tags: [data-layer, open-data]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §5]
confidence: high
---

# LODES

Census LEHD Origin-Destination Employment Statistics, version 8:
`ny_od_main_JT00` (2022 release, 2020 blocks). 1,873,261 block-group pairs of
home→work job flows. Tier A. This is the observed flow mass behind the
activity-space term `A_i` in [[exposure-instrument]] and the decomposition in M3
([[compounding-test-m1-m3]]).

OD aggregation uses top_k=900 destination BGs per home = 99.8–100% of job flow.
An earlier top-100 cut covered only ~58%, and that coverage correlated +0.50 with
income and −0.33 with %Hispanic — a differential truncation, eliminated in the
2026-07-14 verisimilitude pass ([[exposure-instrument]]).

Vintage discipline: LODES8 on 2020 blocks, matched to TIGER 2020 + CenPop 2020 +
ACS 2022.

Related: [[mode-choice-mnl]] · [[data-confidence-tiers]] · [[results-equalization]]
