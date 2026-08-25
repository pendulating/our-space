---
title: Camera Layers
created: 2026-08-23
updated: 2026-08-23
type: moc
tags: [moc, data-layer, surveillance]
sources: [README.md, facct27_digital_eyes_on_the_street/OUTLINE.md §5]
---

# Map of Content — Camera and Sensor Layers

Observed layers used by the paper's empirical claims:

- **Fixed CCTV** — Amnesty *Decode Surveillance NYC* + Dahir et al., merged by
  [[cross-source-dedup]]. 14,100 sites → 28,954 cameras citywide; ~4,400 in the
  Manhattan app build. Tier B.
- **ALPR** — DeFlock via OSM. 444 devices → 945 directional sensors (one per
  gantry bearing). Tier A.
- **DOT traffic cameras** — NYC DOT feed, coordinates only (images never used;
  no open license). 959 units. Tier A.
- **Photo enforcement** — DOT street-sign work orders; 4,182 citywide (fixed from
  Manhattan-only 499 on 2026-07-14). Tier A.
- **Subway** — MTA GTFS; 496 stations; every station and car watched. Tier A.

Excluded or speculative layers:
- **LinkNYC kiosks** — excluded by design (a kiosk watches only when you connect).
- **ACE bus cameras** — real GTFS corridors; observed trajectories; not yet wired
  into block-group tables. See [[compounding-test-m1-m3]].
- **Rideshare dashcams** — real TLC trip intensity (547,263 trips); penetration
  and capture probability are parameters. Tier C.
- **Tesla Sentry / delivery robots / smart glasses** — Tier D, scenario only.
  See [[foresight-scenario-layers]].

Confidence tiers explained in [[data-confidence-tiers]]. Licenses in README
"Data sources & licenses".

Related: [[sensor-model]] · [[exposure-instrument]] · [[epistemic-tiers]]
