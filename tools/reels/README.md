# Reels — scripted vertical video from the live map

Records the **web-deployed our·space build** (WebGPU/WASM Bevy app) playing a scripted
StoryMap tour, straight to a vertical **9:16 MP4** — no manual clicking. This is the
Phase-0/1 implementation of [`docs/REELS_PLAN.md`](../../docs/REELS_PLAN.md).

Like `tools/inspect`, it drives **system Google Chrome** via `playwright-core`
(`channel:'chrome'`) so the Bevy canvas gets the Metal GPU and renders for real. It reuses
`tools/inspect`'s installed `playwright-core` (no separate install), records the canvas
over a CDP screencast, and stitches the frames with **ffmpeg**.

## Prerequisites

- `tools/inspect` set up (`cd tools/inspect && npm install`) — this tool borrows its
  `playwright-core` + system-Chrome setup.
- **ffmpeg** on `PATH` (`brew install ffmpeg`).
- The bundle served on `:8080` (`python3 -m http.server -d web/dist 8080`, or pass
  `--serve`). Rebuild first if you changed the app: `./web/build.sh`.

## Usage

```sh
# from the repo root

# data-driven: author a JSON spec and render it (no recompile) — the main path
node tools/reels/render.mjs --spec tools/reels/specs/watch-astor.json                 # pillar #1 Walkshed
node tools/reels/render.mjs --spec tools/reels/specs/direct-capture-times-square.json # pillar #2 DirectCapture
node tools/reels/render.mjs --spec tools/reels/specs/walk-to-the-train.json           # pillar #3 Route (A→B walk)
node tools/reels/render.mjs --spec tools/reels/specs/neighborhoods-sweep.json         # pillar #4 Neighborhoods
node tools/reels/render.mjs --spec tools/reels/specs/watching-the-park.json           # pillar #5 Institutions (parks)
node tools/reels/render.mjs --spec tools/reels/specs/day-timelapse.json               # pillar #6 clock time-lapse
node tools/reels/render.mjs --spec tools/reels/specs/who-owns-cameras.json            # pillar #8 Operators

# batch: render every spec in specs/ (each drops an mp4 + .stats.json + .png poster)
node tools/reels/render.mjs --all --serve

# built-in tours
node tools/reels/render.mjs --story longitudinal --out decade.mp4 --secs 42
node tools/reels/render.mjs --story tutorial     --out tutorial.mp4 --secs 56

node tools/reels/render.mjs --spec … --serve         # auto-start the server
node tools/reels/render.mjs --spec … --keep-frames   # keep the JPEG frames
```

## Reel specs (`specs/*.json`)

A spec is the authorable unit — a title + ordered steps, no recompile needed. `render.mjs`
base64url-encodes `{title, steps}` into `?reelspec=` and the app plays it (`from_spec_json`
+ `storymap_autostart` in the Rust build). Driver-only fields (`id`, `size`, `fps`, `secs`)
tune the render; capture length auto-sizes to the sum of step dwells when `--secs` is
omitted.

```jsonc
{
  "id": "watch-astor",                 // output filename stem
  "title": "How many cameras watch Astor Place?",
  "size": [1080, 1920], "fps": 30,     // driver-side (optional)
  "steps": [
    { "action": "Overview",  "secs": 2.0, "caption": "New York City" },
    { "action": "FlyTo",     "lat": 40.73, "lon": -73.991, "zoom": 2.6, "secs": 2.5, "caption": "Astor Place" },
    { "action": "Walkshed",  "lat": 40.73, "lon": -73.991, "secs": 5.0, "caption": "Every camera within a 10-minute walk" },
    { "action": "Caption",   "secs": 3.0, "caption": "Hold on the result" }
  ]
}
```

Step `action`s (mirror `StepAction`): `Caption` (hold, view unchanged), `Overview`,
`FlyTo {lat,lon,zoom}`, `Route {a:[lat,lon], b:[lat,lon]}`, `Walkshed {lat,lon}`,
`DirectCapture {lat,lon}`, `Neighborhoods {at?:[lat,lon,zoom]}`, `Institutions {parks_only?,
rank?}` (open the Institutions view, filter markers to parks, and select+fly to the
rank-th most-watched — 0 = the most watched), `Operators`, `Future`,
`Heatmap`, `ClockScrub {from?, to}` (sweep the simulated clock over the step — the
time-lapse; `to` may exceed 24 to cross midnight), `Scene {at?:[lat,lon,zoom], linknyc,
future, operators, heatmap}`.

A spec may also set the initial clock: `"clock": { "t": 6.0, "rate": 36, "play": false }`
→ `?t=6&rate=36&play=0`. In reel mode the time of day is shown on the date plate.

Output MP4s land in `tools/reels/out/` (gitignored). Frames are captured to a temp dir and
deleted unless `--keep-frames`.

## Per-platform crops (`crop.mjs`)

Repurpose a finished 9:16 reel to the other feed aspect ratios **without re-capturing the
WebGPU canvas** — ffmpeg-only, so the whole batch runs in a couple of minutes.

```sh
node tools/reels/crop.mjs --in out/watch-astor.mp4   # → .1x1.mp4 + .16x9.mp4 (+ posters)
node tools/reels/crop.mjs --all                      # every out/*.mp4 (skips existing crops)
node tools/reels/crop.mjs --all --aspect 4:5         # a custom target ratio
node tools/reels/crop.mjs --in out/x.mp4 --mode crop # center-crop instead of blurred fit
```

Default **`fit`** mode is the "blurred-bars" look — a zoomed, blurred copy of the frame
fills the empty sides — so the *whole* 9:16 frame is kept and **no caption or data callout
is cropped off** (the date plate lives at the top edge, the caption at the bottom). `--mode
crop` (center-crop, fills edge-to-edge but loses those edges — motion-only reels) and
`--mode letterbox` (solid `0x0a0a0a` bars) are opt-in. Each target writes `<stem>.<WxH>.mp4`
+ `<stem>.<WxH>.png` next to the source. Aspects: `1:1`, `16:9` by default; `--aspect W:H`
(repeatable) for others (ratios map to a canonical height-1080 pixel size).

| flag | default | meaning |
|------|---------|---------|
| `--in <file>` | — | source `.mp4` (relative → `out/`) |
| `--all` | — | process every `out/*.mp4` (skips files already suffixed `.WxH`) |
| `--aspect <W:H>` | `1:1`, `16:9` | target aspect (repeatable) |
| `--mode <m>` | `fit` | `fit` (blurred bars, keeps captions) \| `crop` \| `letterbox` |
| `--blur <n>` | `22` | gblur sigma for the `fit` background |
| `--pad <hex>` | `0x0a0a0a` | `letterbox` pad colour |
| `--no-poster` | — | don't also crop the `<stem>.png` poster |

## Posting calendar (`calendar.mjs`)

Reads every `specs/*.json` and emits a content calendar — it **plans and reports; it never
posts**.

```sh
node tools/reels/calendar.mjs                                  # → out/calendar.md + .csv
node tools/reels/calendar.mjs --start 2026-07-07 --cadence 3   # one reel every 3 days
node tools/reels/calendar.mjs --platforms tiktok,ig-reels,x    # subset of platforms
node tools/reels/calendar.mjs --print                          # also dump Markdown to stdout
```

`out/calendar.md` has a **schedule** table (date · reel · length · platforms · hook), an
**assets** checklist (which 9:16 render + 1:1/16:9 crops exist vs still need
`render.mjs`/`crop.mjs`), and a **caption copy** block per reel — hook + payoff with
`{cameras}`/`{dashcams}`/… filled from the render's `.stats.json`, plus the required sources
+ "simulated day" line. `out/calendar.csv` is the same rows for a spreadsheet.

| flag | default | meaning |
|------|---------|---------|
| `--start <YYYY-MM-DD>` | today | first posting date |
| `--cadence <days>` | `3` | days between posts |
| `--per-day <n>` | `1` | reels scheduled per posting day |
| `--platforms <a,b,…>` | `ig-reels,tiktok,yt-shorts,ig-feed,x` | any of `ig-reels ig-feed tiktok yt-shorts yt x` |
| `--order <file\|title>` | `file` | spec ordering |
| `--print` | — | also print the Markdown to stdout |

### Flags

| flag | default | meaning |
|------|---------|---------|
| `--spec <file>` | — | JSON reel spec to play (data-driven; the main path) |
| `--all [dir]` | `specs/` | render every `*.json` spec in the dir (one child process each) |
| `--poster <s>` | `0.65×len` | seconds into the reel to grab the `<stem>.png` poster still |
| `--story <id>` | — | StoryMap tour to play: `longitudinal` or `tutorial` |
| `--url <url>` | `http://localhost:8080/` | base URL |
| `--out <file>` | `<story>.mp4` | output MP4 (relative → `out/`) |
| `--secs <n>` | `42` | seconds to record (longitudinal ≈ 39 s; tutorial ≈ 54 s) |
| `--fps <n>` | `30` | output frame rate (constant) |
| `--width`/`--height` | `1080`/`1920` | viewport (9:16) |
| `--settle <ms>` | `3500` | wait after boot before recording |
| `--every-nth <n>` | `2` | keep 1 of every N compositor frames (~30 fps in) |
| `--no-hide-chrome` | — | keep the HTML masthead/footer visible |
| `--no-reel-mode` | — | keep the app's right control panel + transport chrome |
| `--serve` | — | auto-start `python3 -m http.server` on `:8080` if down |
| `--headed` | — | show the Chrome window |
| `--keep-frames` | — | don't delete the captured frames |

## How it stays correct

- **Real timing:** CDP screencast frames arrive at a variable rate; each frame's timestamp
  is written into an ffmpeg `ffconcat` list as its duration, then ffmpeg's `fps` filter
  resamples to a constant `--fps`. So playback speed is right no matter how fast/slow the
  compositor emitted frames.
- **Clean framing (`?reel=1`):** the app hides its right control panel and the StoryMap
  transport buttons when `?reel=1` is set (`ReelMode` in `main.rs`), so the recorded frame
  is just the map + the date plate + the story caption. `render.mjs` sets it by default;
  `--no-reel-mode` opts out.
- **Full-bleed:** injected CSS hides the HTML `<header>`/`<footer>` so the fit-to-parent
  canvas fills the 9:16 viewport.
- **Determinism:** the app bakes one real day (Apr 21 2026) and seeds its agents; the
  StoryMap engine uses fixed per-step dwell + 0.7 s smoothstep flies. Reduced-motion is
  forced off (`emulateMedia`) so the clock doesn't pause.

## Status / roadmap

Implemented (Phase 0 + all Phase-1 gaps + autonomous Phase-2 polish): capture→encode at
9:16; `?reel=1` clean-capture mode; **data-driven JSON specs** (`--spec` → `?reelspec=`)
synced to the story timeline via `REEL_STORY_START`; `Caption`/`ClockScrub` hold the scene;
overlays survive story-driven mode switches; **scriptable clock** (`?t/rate/play` +
`ClockScrub` time-lapse, time on the date plate); **live-stat captions** —
`{cameras}`/`{dashcams}`/`{minutes}`/`{cameras_raw}` tokens filled in-app, plus a
`<stem>.stats.json` sidecar from the `REEL_STAT` console line; **`--all` batch** (each spec
in its own child process) with a **`<stem>.png` poster still** per render; legible
DirectCapture cones under `?reel=1`; **per-platform crops** (`crop.mjs` → 1:1 / 16:9,
blurred-bars fit so no caption is lost); and a **posting-calendar generator** (`calendar.mjs`
→ schedule + caption copy + asset checklist). Still to do (see `docs/REELS_PLAN.md §6`,
Phase 2, and both need your input): audio/voiceover (a track + script) and a branded
intro/outro (brand assets).

Caption tokens: `{cameras}` (headline count — walkshed / direct-capture / route exposure /
selected institution's nearby cameras), `{cameras_raw}`, `{dashcams}` (DirectCapture),
`{minutes}` (walkshed), `{place}` (selected institution name). Unresolved tokens render as "…".
