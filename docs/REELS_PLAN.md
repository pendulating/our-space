# Automated social-media reels — brainstorming & implementation plan

**Status:** _Phase 0 + all Phase-1 gaps (G2–G4 + clean mode) + the autonomous Phase-2 polish
shipped — the pipeline is end-to-end scriptable and batchable._ `tools/reels/render.mjs
--spec foo.json` renders an authored JSON tour to a vertical 9:16 MP4 with no manual
clicking: `?reelspec=` plays it (capture synced to the story timeline), `?reel=1` gives a
clean frame, `?t/rate/play` + `ClockScrub` drive the clock, and captions quote the live
camera count (`{cameras}`) with a `.stats.json` sidecar. `--all` batch-renders every spec
and each render drops a poster thumbnail; DirectCapture cones are legible in reel mode.
`crop.mjs` repurposes any finished reel to 1:1 / 16:9 (blurred-bars fit, so no caption is
lost) and `calendar.mjs` emits a per-platform posting schedule + caption copy from the
specs. The only remaining Phase-2 items need the user's own assets: audio/voiceover (track
+ script) and branded intro/outro cards (brand assets) — see §6. This document is the design
brief for turning the interactive map's modes + explore utilities into short, vertical,
scriptable video reels for Instagram / TikTok / YouTube Shorts / X.

The goal is a **script-driven pipeline**: a reel is a data file (a "spec"); a command
renders it to an `.mp4` with no manual clicking, so a batch of reels can be produced,
regenerated when data changes, and scheduled.

---

## 1. Goals & format

- **Platforms / format:** vertical **1080×1920 (9:16)**, 15–45 s, **silent-first**
  (readable with sound off — burned-in captions + on-frame data callouts), optional
  voiceover/music track.
- **Reproducible:** same spec + same baked assets ⇒ same frames. The app already bakes a
  single **real day** (Tuesday, April 21 2026) for buses/taxis, so the simulated day is
  deterministic; agent spawning is seeded. A reel must not depend on wall-clock timing
  luck.
- **Scriptable end-to-end:** `render.mjs --spec X.json --out X.mp4`, plus a batch mode.
  No human in the loop for a render.
- **Truthful:** every stat on screen is labelled *measured* vs *modeled*, and no reel
  targets a real private individual's home (see §8, Ethics).

---

## 2. Content pillars (mapped to real modes / utilities)

Each pillar is a reusable template; a spec fills in the specifics (address, neighborhood,
time window). Modes referenced are the `Mode` enum
(`crates/app-interactive/src/main.rs:306`): `Walkshed` (My area), `Route` (A→B walk),
`DirectCapture`, `Neighborhoods`, plus the takeover views (Operators, Coverage) and the
day clock.

1. **"How many cameras watch your block?"** — `Walkshed`. Fly to a landmark corner, drop
   the 10-minute walkshed, hold on the "~N cameras watch your area" headline. Punchy,
   universal, remixable per neighborhood. *(Series: one per iconic corner.)*
2. **"The cameras pointed at one address"** — `DirectCapture` (the new mode). Fly to an
   address, draw the facing cones + the "~N cameras point straight at this + ~M dashcams/day"
   card. The most visceral pillar — pairs with a "moving vs fixed" beat.
3. **"The walk to school / to the train"** — `Route`. Trace an A→B walk; overlay the live
   "seen" pulses as the walker passes cameras; end on the exposure tally. *(Shipped:
   `tools/reels/specs/walk-to-the-train.json` — a Times Square → Grand Central commute; the
   walker rides the clock and pulses each camera, the caption quotes the live analytical
   exposure `{cameras}` (~216), and a closing beat labels fixed = measured, moving = model.)*
4. **"Surveillance deserts & hotspots"** — `Neighborhoods` choropleth. Sweep the borough,
   call out the most- and least-watched neighborhoods with their per-km² density.
5. **"Who's watching the park?"** — `Institutions` view (now includes **parks & plazas**
   alongside schools/libraries). Rank card + fly to the most-/least-watched park. Ties to
   pillar #4 but subject-centric. *(Shipped: `tools/reels/specs/watching-the-park.json` —
   filters the markers to parks, flies to the most-watched one, and the caption quotes its
   name + count live via `{place}`/`{cameras}`. Needed a new `StepAction::Institutions`,
   applied by a dedicated `story_apply_institutions` system since `storymap_tick` is at
   Bevy's param limit; the left ranking panel is suppressed in reel mode.)*
6. **"A day of surveillance in 20 seconds"** — the **day clock** time-lapse: let the
   simulated day run (buses/taxis/mobile agents swell at rush hour), captioned with the
   hour. Pure motion; great as an evergreen loop.
7. **"A decade of watching"** — the existing `longitudinal` story
   (`crates/app-interactive/src/storymap.rs:190`), already a 5-scene composed tour. This
   is the *proof-of-concept reel* — it needs zero new scene authoring.
8. **"Who owns the cameras?"** — the **Operators** takeover (stack-by-operator animation).
   Visually distinctive; good for the "it's not one system, it's dozens of operators" beat.
   *(Shipped: `tools/reels/specs/who-owns-cameras.json` — Overview → Operators → hold; the
   towers rise off a recognizable Manhattan with each operator's live count labelled.)*

---

## 3. What we can reuse today vs what we must build

Grounded in the current tree (see `docs/` recon; file:line refs below).

### Reusable now
- **StoryMap engine** = the turnkey scripted-playback primitive. `StoryStep { caption,
  secs, action }` with `StepAction` variants `FlyTo{lat,lon,zoom}`, `Route{a,b}`,
  `Walkshed{lat,lon}`, `Operators`, `Future`, `Heatmap`, `Scene{…}`
  (`storymap.rs:15`). Per-step dwell `secs`; camera flies are a fixed **0.7 s smoothstep**
  (`main.rs:110`, `fly_camera` `main.rs:2937`). Triggered from the URL:
  `?story=tutorial` / `?story=longitudinal` (`main.rs:2806`, `url_story_is`
  `main.rs:2816`). While a story runs, map clicks are inert (`main.rs:3092`) — perfect for
  unattended capture.
- **URL levers:** `?story=…`, `?city=nyc` (forces `Neighborhoods` + citywide,
  `main.rs:1215`), `?debug=perf`.
- **Real WebGPU capture harness:** `tools/inspect/` drives **system Chrome**
  (`channel:'chrome'`) so the Bevy/WebGPU canvas renders for real. Flags: `--width/--height`,
  `--scale`, `--story`, `--wait`, `--wheel` (CDP zoom — the *one* input that reaches winit),
  `--eval` (DOM/CSS only), `--console` (browser logs), `--serve`. `fps.mjs`/`profile.mjs`
  already demonstrate "let the auto-playing clock run for N seconds, then sample."
- **Arbitrary vertical viewport:** no aspect is hardcoded; the canvas is
  `fit_canvas_to_parent` (`main.rs:943`) inside a flex `#stage` (`web/index.html:115`).
  Set the Playwright viewport to 1080×1920 and the surface follows.

### Gaps to build (the real work)
- **G1 — Frame/video capture + encode.** *None exists* (no ffmpeg, no `MediaRecorder`, no
  Playwright `recordVideo` anywhere). Need a screenshot-sequence loop (or CDP screencast /
  `recordVideo`) + **ffmpeg** stitch.
- **G2 — Data-driven stories from the URL.** Stories are hardcoded Rust functions with only
  two names recognized. A script can't author a new tour without recompiling. Need a
  URL/JS path to load a **story/reel spec** (JSON) at runtime.
- **G3 — Clock control from the web.** `SimClock { time_of_day (default 17.0), rate
  (default 36×), playing }` (`main.rs:82`) is settable only via the egui slider on web
  (native has env-var hooks, unusable in a browser). Need URL params, e.g.
  `?t=8.0&rate=240&play=1`, plus a way to *scrub* to a target hour for a time-lapse.
- **G4 — JS-readable result export.** The headline "~N cameras…" lives only as **egui
  pixels** (`ui.rs:1301`, `WalkshedState.summary` `main.rs:820`); there is no DOM/JSON/
  `window` export. Only world-build census `info!` logs reach the console. For captions
  that quote the *live* number, expose results (e.g. `window.__ourspace.lastResult` and/or
  a structured `console.log("REEL_STAT …")`).
- **G5 — Programmatic mode + address without clicks.** CDP mouse clicks don't reach winit,
  and there's no URL param for mode/address. The clean fix is to route *all* mode/address
  selection through the spec-driven story engine (G2) — `Walkshed`/`Route` already take
  lat/lon in `StepAction`; add `DirectCapture{lat,lon}` and `Neighborhoods{focus?}` scene
  actions.

---

## 4. Architecture

```
 spec (.json)  ──►  render.mjs (Node, tools/reels/)
                      │  1. build URL: ?reel=<spec-id> (or ?reelspec=<base64>)  [G2]
                      │  2. launch system Chrome via playwright-core (real WebGPU)
                      │  3. viewport 1080×1920; --eval hides masthead/footer for full-bleed
                      │  4. wait for boot (loading overlay clears — reuse inspect.mjs logic)
                      │  5. capture: screenshot loop @ fixed fps  OR  CDP screencast   [G1]
                      │       └─ app plays the scripted story deterministically
                      │  6. read REEL_STAT console lines → caption values             [G4]
                      ▼
                 frames/*.png  ──►  ffmpeg  ──►  silent 9:16 mp4                        [G1]
                      │
                      ▼
             post: burn captions (drawtext / ASS), add intro/outro card, mux audio     [G1]
                      ▼
                 reels/<name>.mp4   (+ .srt, + thumbnail .png)
```

**Determinism:** the baked real-day + seeded agents make the simulation reproducible; the
story engine's fixed `secs`/0.7 s flies make the camera reproducible. Capture at a fixed
frame cadence tied to *sim time*, not wall-clock, so a slow render still yields the same
frames (requires the capture loop to advance/step deterministically — see Phase 1).

**Why extend StoryMap rather than build a separate director:** it already composes exactly
the primitives a reel needs (camera fly + mode + overlay toggles + timed captions) and is
already URL-triggered and click-suppressing. Making it data-driven (G2) turns "author a
reel" into "write a JSON file" — which is the assignment.

---

## 5. The reel spec (script interface)

A reel spec is a JSON file (the deliverable a content author writes):

```jsonc
{
  "id": "watch-your-block-astor",
  "title": "How many cameras watch Astor Place?",
  "size": [1080, 1920],          // 9:16
  "fps": 30,
  "chrome": "hidden",            // hide masthead/footer for full-bleed
  "clock": { "t": 17.0, "rate": 36, "play": true },   // [G3]
  "steps": [                     // maps 1:1 to StoryStep/StepAction [G2]
    { "action": "Overview", "secs": 1.5, "caption": "New York City" },
    { "action": "FlyTo", "lat": 40.7300, "lon": -73.9910, "zoom": 2.6, "secs": 2.0,
      "caption": "Astor Place" },
    { "action": "Walkshed", "lat": 40.7300, "lon": -73.9910, "secs": 4.0,
      "caption": "Every camera within a 10-minute walk" },
    { "action": "Hold", "secs": 2.5, "caption": "{cameras} cameras watch this block" }
  ],
  "captionStyle": "lower-third",
  "audio": "assets/reel-audio/ambient-01.mp3",   // optional
  "outro": "brand-card"
}
```

- `{cameras}` is a **template token** resolved from the live `REEL_STAT` export (G4) at the
  step it appears — captions quote the real computed number, not a hardcoded guess.
- `render.mjs` serializes the spec to the URL (small specs base64 in `?reelspec=`; larger
  ones fetched by `?reel=<id>` from `web/dist/reels/<id>.json`), Chrome plays it, frames
  are captured, ffmpeg encodes, captions burn in.
- **Batch:** `render.mjs --all specs/` renders every spec; a `calendar.mjs` can emit a
  posting schedule + per-platform crops.

---

## 6. Phased implementation

**Phase 0 — Prove the pipeline (no Rust changes). ✅ DONE.**
- ✅ `tools/reels/render.mjs` (reuses `inspect.mjs`'s boot/serve + system-Chrome/WebGPU
  setup). Records via **CDP screencast** (not a screenshot loop — smoother, and frame
  timestamps are written into an ffmpeg `ffconcat` list so variable-rate capture resamples
  to a correct constant `--fps`). Injected CSS hides the HTML `<header>`/`<footer>`;
  `emulateMedia` forces reduced-motion off so the clock/flies animate.
- ✅ Renders `?story=longitudinal` (and `?story=tutorial`) at 1080×1920 → silent MP4. The
  app draws the story captions itself (egui on-canvas), so no burn-in needed in Phase 0.
- **Result:** `node tools/reels/render.mjs --story longitudinal` → a 42 s, 1080×1920, 30 fps
  MP4 (verified: 2096 frames, correct timing, clean boot). Capture cadence, vertical
  framing, WebGPU-in-Chrome stability over a 40 s+ run, and encode all validated.

**Phase 1 — Make it data-driven & live-captioned (the core Rust work).**
- ✅ **Clean-capture mode** (a hook Phase 0 revealed was needed first): `?reel=1` /
  `OURSPACE_REEL` sets `ReelMode` (`main.rs`), which makes `ui_panel` skip the right control
  panel and `storymap_ui` drop its transport buttons (caption stays, centered). Without it
  the landscape control panel crammed ~⅓ of the 9:16 frame. `render.mjs` sets `?reel=1` by
  default (`--no-reel-mode` opts out). *Verified in real Chrome: the recorded frame is now
  just map + date plate + caption.*
- ✅ **G2 — data-driven story specs (the linchpin).** A reel is now an authorable JSON
  spec (`tools/reels/specs/*.json`); `render.mjs --spec` base64url-encodes `{title, steps}`
  into `?reelspec=` and the app parses it (`storymap::from_spec_json`) into `StoryMap.steps`
  with no recompile. Added `StepAction::DirectCapture{lat,lon}` + `Neighborhoods{at?}` and
  their tick arms. Two supporting fixes fell out of verification:
  (a) **capture↔story sync** — the app logs `REEL_STORY_START secs=<n>` when the autostarted
  tour begins; `render.mjs` waits for it and records exactly the tour length (+tail), so the
  window can't drift against the story timeline (it did before, keying off a fixed settle).
  (b) **`Caption` is now a true hold** (skips the per-step baseline reset) so a "hold on the
  result" beat lingers on the walkshed/choropleth; and **`sync_mode` no longer wipes overlays
  during a story** (it was despawning the walkshed the instant the story set `Mode::Walkshed`
  — a latent bug that also affected the built-in tutorial). Verified: `watch-astor.json`
  renders Overview → fly-in → full walkshed (hull + in-shed camera rings + reachable streets)
  → hold, all correctly captioned and timed.
- ✅ **G3 — clock control.** Startup clock params `?t=<hour>&rate=<x>&play=<0|1>` (native:
  `OURSPACE_T`/`RATE`/`PLAY`) via `url_clock()`, and a `ClockScrub {from?, to}` step that
  sweeps the simulated clock across its dwell (the "day in N seconds" time-lapse, pillar #6;
  `to` may exceed 24 to scrub past midnight). Since reel mode hides the interactive time
  panel, the time of day is surfaced on the date plate (the readout that climbs during a
  scrub). A spec's `clock` block → `?t/rate/play`. Verified: `day-timelapse.json` sweeps
  6 AM → 10 PM over 14 s, the on-frame clock climbing 7:48 AM → 1:31 PM → 8:22 PM → 9:59 PM.
- ✅ **G4 — live-stat export + caption tokens.** The app fills `{cameras}` / `{cameras_raw}`
  / `{dashcams}` / `{minutes}` tokens in a step caption from the live result
  (`ui::fill_caption_tokens`), so a caption quotes the real computed number as it plays; and
  it emits a structured `REEL_STAT {json}` console line when a headline changes
  (`reel_stat_emit`), which `render.mjs` captures into a sidecar `<stem>.stats.json`.
  (Caption tokens are filled in-app rather than by render.mjs, since captions are drawn on
  the egui canvas — the console line is metadata/analytics + a JS-readable export.) Verified:
  `watch-astor.json`'s hold caption renders "~130 cameras watch this block" with
  `{"mode":"walkshed","cameras":130,...}` in the sidecar.
- **Exit criteria (met):** pillars #1–#3 + #6 render purely from a spec, captions quoting
  live numbers. **All core Phase-1 gaps (G2–G4 + clean mode) are done; the pipeline is
  end-to-end scriptable.**

**Phase 2 — Polish & scale.**
- ✅ **Reel-mode legibility (DirectCapture cones).** Two fixes, in order of impact:
  (1) **Zoom** — the cones are street-scale (camera ranges of tens of metres), but the
  DirectCapture step flew to `FLY_AREA_ZOOM` (2.6 m/px ≈ a 2.8 km-wide frame), so a cone was
  a speck. It now flies to a dedicated `FLY_DIRECT_ZOOM` (0.8 m/px ≈ a single-block frame) —
  the target dot, its camera ring, and the FOV cone are all clearly legible. The interactive
  address-pick paths were made mode-aware too, so live DirectCapture matches. (2) **Alpha** —
  cone fill bumped α 0.15 → 0.28 so it reads on a recorded frame. The direct-capture spec
  also leads its hold caption with the striking, plural-safe `{dashcams}` figure ("~11,285
  rideshare dashcams … far more than the fixed cameras that frame it") — the moving-vs-fixed
  contrast is the payload, since a single exact point is framed by only a camera or two.
  (Walkshed / Neighborhoods overlays read fine as-is.) *Verified: cone + target legible in
  the tight frame, caption renders, poster grabs the reveal.*
- ✅ **Reel-mode legibility (Operators takeover).** The Operators view bottom-anchors a
  per-column count label (CCTV / DOT / ALPR / ENF / MTA / RIDESHARE …), which collided with
  the reel's bottom-centred caption bar. In reel mode, while that overlay is active, the
  caption now anchors **top-centre** (clear of the left-anchored date plate), leaving the
  bottom strip for the operator labels. *Verified via `who-owns-cameras.json`: caption up
  top, all six operator towers + counts legible along the bottom.*
- ✅ **`--all` batch + poster/thumbnail export.** `render.mjs --all` renders every spec in
  `specs/` (each in its own child process for a clean WebGPU context), and every render
  drops a **poster** `<stem>.png` (a representative mid-tour frame at 0.65× the reel length
  via `ffmpeg -ss`, overridable with `--poster <s>`) next to the MP4 + `.stats.json`.
  *Verified: 4/4 specs render with stats + posters.*
- ✅ **Reel-mode framing (fly targeting).** `resolve_fly_target` reserved 340 px for the
  side panel and shifted every fly-to target right by `PANEL_HALF_PX` so it sat left of the
  panel. In reel mode the panel is hidden, so this over-zoomed and left-shifted every reel —
  worst on `Route`, whose `FlyTo::Fit` framed the walk into the left third with a big empty
  gutter. Now, in reel mode, no panel reserve and no shift: the target fills and centres the
  whole 9:16 frame. Fixes all reels (Point flies were off-centre too); interactive mode is
  unchanged. *Verified: the `walk-to-the-train` route now spans the frame, centred.*
- ✅ **Capture robustness (anti-throttle).** An offscreen/occluded headless page can get
  throttled to ~1 fps, which stretches the screencast timestamps into a multi-minute, choppy
  mp4 for a ~20 s tour. Launch now passes `--disable-renderer-backgrounding`,
  `--disable-background-timer-throttling`, `--disable-backgrounding-occluded-windows`,
  `--disable-features=CalculateNativeWinOcclusion` to keep the compositor at full rate.
  *(Frame rate can still dip under heavy local load — render one spec at a time when the
  machine is busy.)*
- **Audio bed + optional TTS voiceover** from a `voiceover`/`audio` field (script → TTS →
  mux). *Needs the user's track/script + brand — not autonomous.*
- **Branded intro/outro cards** + per-platform safe-area caption presets. *Needs brand
  assets — not autonomous.*
- ✅ **Motion polish (Ken-Burns push-in).** A reel-only `ken_burns_drift` system sheds a
  sliver of camera scale each frame (`KEN_BURNS_RATE` ≈ 1%/s) on a *settled* hold, so a
  static caption beat drifts gently inward instead of freezing. Narrowly gated: only in reel
  mode, only while a story is active with no fly in flight, and **suppressed during the
  `Operators` takeover** (its towers are laid out for the frozen viewport, so a zoom would
  crop them) and during `ClockScrub` (the time-lapse is already moving). Each fly resets the
  scale to its target, so the drift can't accumulate across beats. *Verified: the Astor
  walkshed pushes in over its hold (5:04→5:08 PM frames); the Operators reel framing is
  pixel-identical mid-vs-late (suppression holds).*
- ✅ **Operators RIDESHARE consistency (scheduling race).** The rideshare tower is a
  true-count column spawned via deferred `Commands`, so it only becomes queryable a frame or
  two after the takeover opens. `operators_layout` latched its lanes after one pass, so if it
  ran before the mesh was visible the RIDESHARE lane was silently dropped — flaky between
  renders (2,794 one run, absent the next). Fixed by having the layout also rebuild when the
  `OperatorMesh` count changes (`OperatorsLayout.meshes_seen`), so the late-arriving column
  reliably gets a lane. *Verified: RIDESHARE 2,794 present across repeated renders.*
- ✅ **Mobile-readable reel UI.** The date/time plate and the caption card were sized for a
  desktop side view and read too small on a phone-held 9:16 frame. In reel mode the plate
  (date 24→36, kicker 10→15, time 18→30 pt) and the caption card (body 16→30, title 12→20 pt,
  wider card + more padding) are scaled up; the Operators top-caption offset was bumped to
  clear the larger plate. Interactive/desktop sizes are unchanged.
- ✅ **Per-platform crops (`crop.mjs`).** `node tools/reels/crop.mjs --all` (or `--in
  out/<id>.mp4`) repurposes each finished 9:16 reel to **1:1** (Instagram feed) and **16:9**
  (X / YouTube) — ffmpeg-only, no WebGPU re-capture, so the whole batch runs in ~2 min.
  Default `fit` mode uses the **blurred-bars** technique (a zoomed, blurred copy of the frame
  fills the sides) so the *entire* vertical frame — date plate on top, caption/data callout
  on the bottom — survives; a naïve centre-crop would slice off exactly those callouts, which
  §8 forbids. `--mode crop` (centre-crop, motion-only reels) and `--mode letterbox` (solid
  bars) are opt-in; `--aspect W:H` targets any ratio. The reel's poster PNG is cropped to
  match. *Verified: the Astor walkshed and the layout-sensitive Operators takeover (top
  caption + six bottom towers incl. RIDESHARE 2,794) both survive intact in 1:1 and 16:9.*
- ✅ **Content-calendar generator (`calendar.mjs`).** `node tools/reels/calendar.mjs` reads
  every `specs/*.json` and emits `out/calendar.md` + `out/calendar.csv`: a posting schedule
  (one reel every `--cadence` days from `--start`, `--per-day` N), the per-platform aspect
  each post needs, the on-screen hook with `{cameras}`/`{dashcams}`/… tokens filled from the
  render's `.stats.json`, a ready-to-paste caption block (hook + payoff + the required sources
  / "simulated day" line), and a per-reel asset checklist (which 9:16 render + crops exist vs
  still need `render.mjs`/`crop.mjs`). Plans + reports only — it never posts. *Verified: 7
  reels scheduled across platforms with live numbers ("~130 cameras", "~11,285 dashcams").*
- Still to do (both need the user's assets, not autonomous): audio/voiceover and branded
  intro/outro cards — see the two entries above marked *not autonomous*.

---

## 7. Open technical questions
- **Capture method:** screenshot-loop (simple, deterministic, slower) vs CDP screencast /
  Playwright `recordVideo` (faster, but frame timing tied to wall-clock — worse for
  determinism). Lean screenshot-loop for reproducibility; revisit if too slow.
- **Sim-time vs wall-time capture:** for perfect reproducibility the app should advance the
  clock by a fixed Δ per captured frame (a "render/capture mode" that steps deterministically
  rather than free-running at 36×). Worth a small Rust hook in Phase 1.
- **Font/caption rendering:** burn via ffmpeg `drawtext`/ASS (portable) vs an HTML overlay
  composited before capture. ffmpeg keeps captions out of the WebGPU frame and easy to
  restyle per platform.

## 8. Ethics & accuracy guardrails (non-negotiable)
- **No real private homes as targets.** `DirectCapture`/`Walkshed` reels use public
  landmarks, civic institutions, or clearly fictional/representative addresses — never a
  real named individual's residence.
- **Label measured vs modeled** on-frame: fixed cameras are crowdsourced census + ML
  detections (recall-corrected); moving-dashcam counts and the day's traffic are a **model**.
  The `DirectCapture` "~M dashcams/day" figure especially must read as an estimate.
- **Cite sources** in the outro (OpenStreetMap, Amnesty Decode Surveillance NYC, NYC DOT,
  Parks Properties, TLC) — the app already carries this provenance line.
- **No live/real-time claims** — it's a simulated day on a fixed baked date.

## 9. First three reels to ship (after Phase 1)
1. **"A decade of watching"** — the `longitudinal` story, vertical (Phase 0 can ship this).
2. **"How many cameras watch Astor Place?"** — Walkshed pillar #1.
3. **"The cameras pointed at this address"** — DirectCapture pillar #2 (the new mode), on a
   representative/fictional apartment.
