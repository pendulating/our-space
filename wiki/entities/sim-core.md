---
title: sim-core
created: 2026-08-23
updated: 2026-08-23
type: codebase
tags: [codebase, sim-core]
sources: [README.md, docs/ARCHITECTURE.md]
confidence: high
---

# sim-core

The render-agnostic core crate and the paper's measurement engine. Pure Rust; the
optional `ecs` feature adds a Bevy layer.

## Contents
- ENU projection; routable walk/drive graphs (`CsclNetwork::{Drive, Walk}` duals)
  + A* with snap/validate.
- FOV wedge geometry + 2D line-of-sight occlusion against 5.46M building walls
  ([[occlusion-null]]). One capture predicate for app and paper.
- Exposure model: headline "cameras that saw you", expected capture events, %
  surveilled; `EXPOSURE_SAMPLE_STRIDE_M` 10 m arc-length sampling
  ([[exposure-instrument]]).
- `sim_core::mobile` — Poisson mobile-intensity fields (`MobileScenario`,
  `AceConfig`, `DashcamFieldLayer`, `exposure_rates_per_minute`); used by
  `batch heatmap`, not yet by block-group tables ([[compounding-test-m1-m3]]).
- `sim_core::subway` — GTFS headway-based subway graph with common-lines pooling;
  all 245,520 station pairs reachable ([[mode-choice-mnl]]).

## Constants that matter to the paper
- `CENSUS_RECALL = 0.501` — capture–recapture recall of the merged census.
  `DAHIR_RECALL = 0.63` is documented as the wrong number for this layer
  ([[capture-recapture-undercount]]).
- `OURSPACE_RANGE_SCALE`, `OURSPACE_SUBWAY_CIRCUITY` fallbacks.

Tests: `cargo test -p sim-core --no-default-features`. 151+ workspace tests green
after the subway-graph re-bake.

Related: [[codebase-layout]] · [[sensor-model]] · [[point-vs-path]]
