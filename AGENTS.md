# AGENTS.md

## Commands

```sh
# Fast test suite (sim-core without Bevy — always use --no-default-features)
cargo test -p sim-core --no-default-features

# Native dev window
cargo run -p app-interactive

# Headless exposure demo (HOUR=0..23)
HOUR=8 cargo run -p sim-core --example route_demo -- 40.7330 -73.9830 40.7160 -73.9810

# Citywide heatmap (arg = hour 0..23)
cargo run -p batch --release -- heatmap assets/processed/heatmap.postcard 17

# Web build (requires: wasm32 target, wasm-bindgen-cli 0.2.125, binaryen, brotli)
./web/build.sh
python3 -m http.server -d web/dist 8080

# Data pipeline (requires raw snapshots in data/snapshots/ first — see README)
cargo run -p data-pipeline -- bake-graph --overpass-json <input> <output> [clip.geojson]

# Python tools (uv-managed env)
uv run python3 tools/<script>.py

# Marimo notebook
uv run marimo edit notebooks/exposure_explorer.py
```

## Architecture

Four-crate Cargo workspace (Rust 2021, MSRV 1.88, Bevy 0.18):

- **sim-core** — pure Rust: ENU projection, FOV/occlusion, exposure model, A* routing. No Bevy unless `ecs` feature is enabled. All 25 unit tests run without Bevy.
- **data-pipeline** — native CLI that bakes raw NYC open data into postcard binaries in `assets/processed/`.
- **app-interactive** — Bevy app; native dev (Metal) and public WASM/WebGPU build. Loads baked assets via `AssetServer`.
- **batch** — headless citywide heatmap computation.

## Key constraints

- **`web/dist/` is committed.** CI (`.github/workflows/deploy.yml`) only uploads it to GitHub Pages — it never builds WASM or bakes assets. Rebuild locally with `./web/build.sh`, commit `web/dist/`, then push.
- **Baked assets are gitignored** (`crates/app-interactive/assets/processed/`, `data/snapshots/`). They require gigabytes of raw NYC data fetched via documented curl/DuckDB/Overpass steps in the README.
- **wasm-bindgen-cli must be exactly 0.2.125** to match the crate version. Mismatched versions produce linker errors.
- **sim-core tests: always `--no-default-features`** to avoid pulling Bevy into the test build.
- **`wasm-release` profile** (opt-level "z", fat LTO, panic=abort) is used for the WASM build; regular `release` is for native.
- **web/build.sh has a bundle-size budget gate** — it will fail if tracked assets exceed their MB budgets. Bump budgets deliberately in the script if an asset is meant to grow.
- **Brotli compression in web build**: baked layers get an `OSZ1` magic prefix + brotli; the app's loader (`crates/app-interactive/src/loading.rs`) detects and decompresses. Native assets stay raw.

## Conventions

- Workspace deps are centralized in root `Cargo.toml` `[workspace.dependencies]`. The `leafwing-input-manager = "0.20"` pin is load-bearing (0.18 targets Bevy 0.17 and would split the graph).
- Bevy is declared per-crate (not workspace-inherited) so each binary picks its own feature set.
- Design brief lives in `.impeccable.md` — journalistic/editorial tone (NYT broadsheet), warm newsprint chrome + cold surveillance data layers. No emoji, no rounded chips, no dark-mode neon.
- Docs: `docs/ARCHITECTURE.md` (system design), `docs/DESIGN.md` (visual system), `docs/PLAN.md` (roadmap).
