---
title: Web Build
created: 2026-08-23
updated: 2026-08-23
type: codebase
tags: [web, codebase]
sources: [README.md, AGENTS.md, docs/ARCHITECTURE.md]
confidence: high
---

# Web Build and Deployment

`./web/build.sh` produces a static `web/dist/`: wasm-bindgen + wasm-opt (binaryen),
brotli-compressed baked layers (`OSZ1` magic prefix; the loader in
`crates/app-interactive/src/loading.rs` detects and decompresses). The page does
WebGPU detection, shows a loading screen, and carries the framing text:
"estimate, not a surveillance map" + route-stays-client-side.

Basemap: NYC Human Geography vector tiles (ArcGIS via MapLibre GL) as ground layer
under a transparent Bevy canvas. Bevy owns input; MapLibre syncs passively from the
camera each frame. Native dev renders an independent dark "caution" theme.

Constraints:
- `web/dist/` is committed; CI only uploads to GitHub Pages
  (https://pendulating.github.io/our-space/).
- wasm-bindgen-cli pinned to 0.2.125 exactly.
- Bundle budget gate in `build.sh`; bump budgets deliberately.
- ~46 MB payload (22 MB WASM + 24 MB layers).

Address lookup is the one network call: query text / single pin coordinate to the
key-free NYC GeoSearch API; computed routes never leave the browser
([[civic-tool-ethics]]).

Related: [[civic-tool]] · [[codebase-layout]] · [[civic-tool-ethics]]
