---
title: Civic Tool
created: 2026-08-23
updated: 2026-08-23
type: concept
tags: [surveillance, privacy]
sources: [README.md, docs/DESIGN.md, docs/PLAN.md]
confidence: high
---

# The Civic Tool

`our-space` is an interactive + batch geospatial simulation of cameras entering NYC
public space. Enter a walking route A→B on the real Manhattan street network; the
tool estimates how many cameras could capture you and how often, with time of day.
It is an **honest estimate tool**, not a surveillance map or an evasion guide.

## What it shows
- Fixed CCTV, ALPR, DOT, and photo-enforcement layers over the 151k-node /
  221k-edge Manhattan walk graph ([[camera-layers]]).
- Time-of-day model with three mobile classes: ACE buses on real GTFS corridors,
  rideshare dashcams from real TLC trip density, smart glasses as a Tier-D
  scenario. Departure-hour scrubber + sliders re-evaluate live.
- Two modes: animated walk (cameras pulse; live capture count) and a 10-minute
  walkshed. Plus a citywide heatmap (`batch`) and an equity overlay — block-group
  Shannon diversity joined to detected cameras, framed by [[dahir-nature-cities]].
- Dual exposure mode: deterministic *Research estimate* (the reproducible Poisson
  figure) vs *Live walk*, a stochastic Monte-Carlo sample of the same model.

## Relationship to the paper
Same engine, same capture predicate — app and paper cannot diverge
([[sim-core]]). The tool makes the instrument's claims inspectable by the public;
the paper makes them rigorous for reviewers. Design brief in `.impeccable.md`;
visual system in `docs/DESIGN.md`.

Related: [[civic-tool-ethics]] · [[web-build]] · [[exposure-instrument]]
