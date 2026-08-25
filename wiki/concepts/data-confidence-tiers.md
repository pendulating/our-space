---
title: Data Confidence Tiers
created: 2026-08-23
updated: 2026-08-23
type: concept
tags: [data-layer, method, ethics]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §5]
confidence: high
---

# Data Confidence Tiers (A–D)

The tier system already exists in the codebase. The paper uses it — it is the
epistemic hygiene a FAccT reviewer wants, and almost no surveillance paper has it.

| Layer | Source | N | Tier | In instrument? |
|---|---|---|---|---|
| CCTV | Amnesty + Dahir et al. | 14,100 sites → 28,954 cameras | B | Yes |
| ALPR | DeFlock / OSM | 444 devices → 945 sensors | A | Yes (road-facing) |
| DOT traffic | NYC DOT feed | 959 (958 online) | A | Yes (road-facing) |
| Photo enforcement | DOT sign work orders | 4,182 citywide | A | Yes (road-facing) |
| LinkNYC | city feed | — | — | Excluded by design |
| Commute OD | LODES8 `ny_od_main_JT00` | 1,873,261 BG pairs | A | Yes |
| Demographics | ACS 5-yr 2022 | 6,807 BGs | A | Yes |
| Crime / 311 | NYPD YTD; 311 subset | 855k pts | A | Yes (contested control) |
| Subway | MTA GTFS | 496 stations | A | Yes |
| ACE buses | ki2b-sg5y + GTFS shapes | ~60 routes in snapshot | A | Not yet (M1) |
| Rideshare dashcams | TLC HVFHV one day | 547,263 trips | C | Not yet (M1) |
| Tesla/robots/glasses | DMV; Robotability; none | 29,177 Teslas | D | Foresight only |

**Vintage discipline:** LODES8 (2020 blocks) + TIGER 2020 + CenPop 2020 + ACS
2022. Flag as the #1 silent bug avoided.

Known data defect: `poverty_rate` is 100% empty (B17001 not published at BG level).
Drop it or move to tract level. See [[open-work-before-submission]].

Related: [[camera-layers]] · [[epistemic-tiers]] · [[lodes-commute-data]]
