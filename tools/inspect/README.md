# Frontend inspector

Drives the **web-deployed our-space build** (WebGPU/WASM Bevy app) in a real
browser and captures what it actually renders — so design tweaks can be checked
against the composited frame, not guessed at.

The hard part is WebGPU: the bundled Playwright Chromium can't initialize it
reliably headless on macOS. This tool uses `playwright-core` to drive the
**system Google Chrome** (`channel:'chrome'`), which gets the Metal GPU, so the
Bevy canvas renders for real. Installing pulls one package — no browser binary
download.

## Setup (one-time)

```sh
cd tools/inspect
npm install                 # just playwright-core; uses your installed Chrome
```

## Serve the bundle

The app must be served (WebGPU needs a secure context; localhost counts):

```sh
python3 -m http.server -d web/dist 8080      # from repo root
```

`inspect.mjs --serve` will start this for you if `:8080` is down, and stop it
when done.

## Capture

```sh
cd tools/inspect
node inspect.mjs                         # screenshot localhost:8080 -> out/shot-*.png
node inspect.mjs --out hero.png          # named output (relative paths land in out/)
node inspect.mjs --width 1920 --height 1080
node inspect.mjs --story intro           # deep-link ?story=intro (StoryMap tours)
node inspect.mjs --headed --keep         # watch it live; leave Chrome open
node inspect.mjs --console --dom         # also dump console logs + UI panel text
node inspect.mjs --eval "document.querySelector('#search')?.click()"   # interact first
```

Run `node inspect.mjs --help` for the full flag list.

### Known limitation: clicks/drags don't reach the app

`--wheel` works (zoom), but **mouse-button events (`--click`, `--pan`) do not reach the
Bevy/winit app in headless Chrome** — neither CDP mouse events nor synthetic
`PointerEvent`s trigger winit's pointer handlers here. So the inspector verifies
*render* state (colors, sizes, layer visibility, zoom) but **not click/drag
interactions** (the route/walkshed click, the operator-view button, the ALPR/CCTV
modals, the FOV toggle). Those work in a real browser — verify them by hand in the
live app, or screenshot a state that a `?story=` deep-link can reach without a click.

### Notes

- Output PNGs go to `tools/inspect/out/` (gitignored). Default is retina (`@2x`);
  override with `--scale 1`.
- The tool waits for the app's own boot signal — the loading overlay clearing —
  then a `--wait` settle (default 4s) for Bevy to finish loading ~26 MB of
  assets and render. Bump `--wait` for slow first loads.
- Exit code is non-zero if WebGPU never initialized or the app hit its failure
  page, so it's usable as a smoke check.
- It reports failed requests collapsed by path. The `<asset>.meta` 404s are
  **expected** — Bevy's `AssetServer` probes for an optional sidecar per asset
  and falls back to defaults. They are not missing files.
