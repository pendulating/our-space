---
title: Codebase Layout
created: 2026-08-23
updated: 2026-08-23
type: codebase
tags: [codebase]
sources: [README.md, docs/ARCHITECTURE.md, AGENTS.md]
confidence: high
---

# Codebase Layout

Four-crate Cargo workspace (Rust 2021, MSRV 1.88, Bevy 0.18):

- **`crates/sim-core`** — pure Rust core: ENU projection, FOV/occlusion geometry,
  exposure model, routable graph + A*, simulation loop. No Bevy unless the `ecs`
  feature is on. All unit tests run with `cargo test -p sim-core
  --no-default-features`. The paper's instrument lives here ([[sim-core]]).
- **`crates/data-pipeline`** — native CLI that bakes raw NYC open data into
  postcard binaries in `assets/processed/`.
- **`crates/app-interactive`** — Bevy app; native dev window and public
  WASM/WebGPU build. See [[civic-tool]] and [[web-build]].
- **`crates/batch`** — headless host for citywide heatmaps and the OD/block-group
  exposure bakes.

## Load-bearing facts (from AGENTS.md)
- `web/dist/` is committed; CI only uploads it.
- wasm-bindgen-cli must be exactly 0.2.125.
- sim-core tests always use `--no-default-features`.
- `web/build.sh` enforces bundle-size budgets.
- Baked layers get an `OSZ1` prefix + brotli on web; native stays raw.

## Analysis tools (`tools/`, Python via uv)
`inequality_stats.py` ([[results-equalization]]), `crime_ladder.py`
([[results-laundering-ladder]]), `sweep_recall.py`
([[capture-recapture-undercount]]), `spatial_econometrics.py`
([[spatial-econometrics]]), `occlusion_summary.py` ([[occlusion-null]]),
`make_tables.py`, `network_effect.py`, `undercount_spatial.py`,
`sweep_subway_cameras.py`.

Related: [[exposure-instrument]] · [[civic-tool]] · [[paper-manuscript]]
