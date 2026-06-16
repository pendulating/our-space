# our-space — Design

The visual and UX system behind **our-space**. For the technical architecture
(simulation core, data pipeline, rendering, web build) see
[`ARCHITECTURE.md`](ARCHITECTURE.md); for the original roadmap see
[`PLAN.md`](PLAN.md). The design ethos that grounds everything below lives in
[`../.impeccable.md`](../.impeccable.md).

## Survey of the Watched Commons — Visual Design

### Concept

The interface is a **warm parchment field-journal pierced by cold surveillance**. The product (`our·space`) estimates how much of a Manhattan walk is captured by cameras and sensing devices; per `.impeccable.md`, the design holds an earthy, organic, hand-drawn-cartography calm in **stark contrast** to the clinical surveillance it depicts. The chrome and base map *are* the human/natural world (paper, soil, moss, oak-gall ink); surveillance is the cold slate intrusion that visibly does not belong. The tension is the message — "not another dark-mode neon dashboard."

This contrast is enforced through a single shared palette across three surfaces, with a deliberate **warm-vs-cold split**: everything human, mapped, and grounded is earthy; everything that represents the machine (cameras, exposure-heat highs, ACE corridors, ALPR readers, the speculative tier) is cold steel/slate.

### Surfaces

The palette is implemented three times against the same hex values:

- **DOM/CSS chrome** — `web/index.html` `:root` custom properties (header cover-plate, footer credit-line, loader overlay).
- **egui control panel** — `crates/app-interactive/src/ui.rs`, `apply_theme()` (`egui::Visuals`), `tier_style()`, `LEGEND`.
- **Bevy map layers** — `crates/app-interactive/src/main.rs`, `ClearColor`, per-layer `Color::srgb_u8` materials, `HEAT_COLORS`, the equity ramp.

### Palette

#### Warm / human / organic

| Role | Hex | Where |
|---|---|---|
| Page — aged parchment | `#e3d7bf` | CSS `--page` |
| Map background — parchment | `#e7dcc4` | `ClearColor(Color::srgb_u8(0xe7,0xdc,0xc4))` |
| Surface — lighter plate | `#efe6d2` | CSS `--surface`; egui `window_fill` `0xef,0xe6,0xd2` |
| egui panel fill | `#e9dcc4` | egui `panel_fill` / `noninteractive.bg_fill` |
| Faint / extreme bg | `#ddcdaf` | egui `faint_bg_color`, `extreme_bg_color` |
| Inactive widget | `#dccba9` | egui `widgets.inactive` |
| Hovered widget | `#cbb78d` | egui `widgets.hovered` |
| Oak-gall ink (body text) | `#3a2e21` | CSS `--ink`; egui `override_text_color` / fg strokes `0x3a,0x2e,0x21` |
| Strong ink | `#241c12` | CSS `--ink-strong` |
| Faded marginalia (muted) | `#67563f` | CSS `--muted` |
| Engraved rule | `#c2b291` | CSS `--rule`; egui `window_stroke` / separator `0xc2,0xb2,0x91` |
| Rule 2 | `#b59c6f` | CSS `--rule-2` |
| Street linework — warm ink | `#8a7550` | `street_mat` |
| Terracotta / iron-gall pigment | `#a8541f` | CSS `--terra`; egui `widgets.active` + `selection.bg_fill` (`0xa8,0x54,0x1f`); route line `0xa8,0x54,0x1f` |
| Terracotta link | `#8c3f12` | CSS `--terra-link`; egui `hyperlink_color` / `selection.stroke` |
| Walker — burnt sienna | `#7a3b14` | `walker_mat` |
| B marker — deep terracotta | `#6e2f12` | `spawn_marker` (destination) |
| Walkshed streets — warm gold | `#c9892f` | reachable-edge mesh |
| Lichen (A marker / center / equity high) | `#4e6638` | A marker, walkshed center, equity `hi`; CSS `--lichen` |
| Headline result | `#9a4a17` (egui `0x9a,0x4a,0x17`) / `#9c7c6e` mid-heat | route + walkshed `ui.heading` color |

#### Cold / surveillance / the machine

| Role | Hex | Where |
|---|---|---|
| The machine — slate intrusion | `#345169` | CSS `--cold`; egui tier-D + correlation/live-count text (`0x34,0x51,0x69`) |
| CCTV — cold panopticon ink | `#2a3a52` | `cctv_mat` |
| ALPR — steel plate-reader | `#41607e` | `alpr_mat` |
| ACE corridor — cold steel | `#7287a4` | `ace_mat` |
| Camera FOV cone | `rgba(0.11,0.21,0.40,0.34)` | `wedge_mat` |
| Walkshed in-shed camera ring | `rgba(0.16,0.30,0.50,0.85)` | `ring_mat` |

The `.machine`/`code` CSS class and inline `<code>` render in `--mono` at `--cold` — the "machine voice" is literally colored slate even in the chrome.

### Typography

Three typefaces, loaded from Google Fonts in `web/index.html` (`Fraunces` opsz 9–144 @ 400/600/900; `Spectral` 400/500/600 + italic 400; `IBM Plex Mono` 400/500):

| Token | Stack | Role |
|---|---|---|
| `--display` | `"Fraunces", Georgia, serif` | Headline `h1`, `.seal`, `.chip`, `#ov-title` — the journal's engraved display voice |
| `--body` | `"Spectral", Georgia, serif` | Body copy, subtitle (italic), footer (italic) — humanist book serif |
| `--mono` | `"IBM Plex Mono", ui-monospace, monospace` | `code`/`.machine` — the cold "machine voice," set in `--cold` |

The masthead `h1` is Fraunces 900 with a `clamp(22px,3.4vw,34px)` size, `-0.01em` tracking, and a `0 1px 0 rgba(255,250,238,.55)` letterpress highlight; the `·` separator (`.dot`) is terracotta. The `.seal` is uppercase Fraunces 600 at `.14em` tracking, terracotta-outlined, rotated `-1.5deg` like a hand-stamped mark.

Note: the **egui panel does not use these fonts** — it renders in egui's built-in font (the source sets `Visuals` only, never a `FontDefinitions`). The custom type system is DOM-only; the egui panel carries the palette and tier coding rather than the typefaces.

### Confidence tiers

Estimates carry an honest A→D confidence tier (`sim_core::ConfidenceTier`), color-coded consistently between `ui.rs` `tier_style()` and the CSS footer `.chip` classes. Tier **D shares the cold surveillance slate** — "speculative = the machine":

| Tier | egui label | egui ink (`tier_style`) | CSS chip (`--*`) |
|---|---|---|---|
| A | `A · mapped` | `#4e6638` lichen | `.chip.a` = `--lichen` `#4e6638` |
| B | `B · estimated` | `#7a5d18` ochre | `.chip.b` = `--ochre` `#7a5d18` |
| C | `C · modeled` | `#a8501f` clay | `.chip.c` = `--clay` `#a8501f` |
| D | `D · speculative` | `#345169` slate | `.chip.d` = `--cold` `#345169` |

A→C climb a warm earth ramp (moss → ochre → clay); D drops to cold slate, signaling that fully speculative sensing (smart glasses) belongs to the machine, not the surveyed ground.

### Map layers

Every Bevy layer (`main.rs`) is colored to land on one side of the warm/cold split, drawn over the parchment `ClearColor` `#e7dcc4`:

| Layer | Color | Side | Source |
|---|---|---|---|
| Streets | `#8a7550` warm ink | warm | `street_mat` |
| Route line | `#a8541f` terracotta ink | warm | `line_mat` |
| Walker dot | `#7a3b14` burnt sienna | warm | `walker_mat` |
| A marker | `#4e6638` lichen | warm | `spawn_marker` |
| B marker | `#6e2f12` deep terracotta | warm | `spawn_marker` |
| Walkshed reachable streets | `#c9892f` warm gold | warm | walkshed edge mesh |
| Walkshed center | `#4e6638` lichen | warm | `center_mat` |
| Fixed CCTV | `#2a3a52` panopticon ink | cold | `cctv_mat` |
| ALPR plate readers (squares) | `#41607e` steel | cold | `alpr_mat` |
| ACE corridors (teal) | `#7287a4` cold steel | cold | `ace_mat` |
| Camera field-of-view cone | `rgba(0.11,0.21,0.40,0.34)` | cold | `wedge_mat` |
| Walkshed in-shed camera ring | `rgba(0.16,0.30,0.50,0.85)` | cold | `ring_mat` |

#### Heatmap gradient (`HEAT_COLORS`)

The citywide exposure heatmap runs a **warm-low → cold-high** ramp — exposure literally cooling toward steel as it intensifies. The same six stops are mirrored in the egui `LEGEND` (rendered as `■` swatches between "low" and "high" labels):

| Stop | Hex | Read |
|---|---|---|
| 0 (low) | `#dccca4` | warm parchment |
| 1 | `#cba968` | gold |
| 2 | `#b88a3e` | ochre |
| 3 | `#9c7c6e` | clay/transition |
| 4 | `#5e6f8c` | cooling slate |
| 5 (high) | `#2c4763` | cold deep slate |

#### Equity choropleth

The neighborhood-diversity overlay (`rebuild_equity`) interpolates block-group Shannon entropy along a **washed-clay → lichen** ramp at 0.55 alpha: `lo = #cdb98f` (homogeneous) → `hi = #4e6638` (diverse). The comment makes the thesis explicit — the warm/diverse ground is precisely what the cold surveillance light bleaches and clusters on (the Dahir et al. correlation), so the choropleth stays warm to set up the cold cameras stacked on top.

### Interaction and motion

- **Pan/zoom/click** — drag pans; scroll zooms; WASD/arrows pan (surfaced in the panel as "Drag: pan · Scroll: zoom · WASD/arrows: pan"). Zoom uses `ZOOM_PER_NOTCH = 0.06`, `ZOOM_PIXEL_DIVISOR = 160.0`, factor clamped to `[0.86, 1.16]` per event, and scale clamped to `ZOOM_MIN = 0.4`..`ZOOM_MAX = 30.0`. Click places A then B (route mode) or a single walkshed seed; `EguiWants` gates pointer/keyboard so panel interaction never leaks to the map.
- **Live walk tally** — as the animated walker passes fixed cameras, the panel shows `▸ passed N fixed cameras so far this walk` in slate `#345169` (the running count is itself "the machine" voice).
- **Self-inking compass loader** — the `#overlay` boot screen draws an SVG compass (`.compass`) that *inks itself in*: `.ink-path` strokes (circle `r=26`, `--len:164`; diamond needle, `--len:88`) animate via `@keyframes ink` from `stroke-dashoffset: var(--len)` to `0` over `1.8s cubic-bezier(.22,.61,.36,1) infinite alternate`, in terracotta `--terra`. Static `.ink-tick` cardinal marks sit at `.6` opacity. The overlay fades (`opacity .6s`, then `display:none` after 650ms) once the WASM module resolves. `@media (prefers-reduced-motion: reduce)` freezes the path fully inked. A WebGPU capability check (`'gpu' in navigator`) replaces the loader with a graceful "This survey needs WebGPU" fallback rather than animating.

### Chrome treatment

The header is the journal's **cover-plate**: `--surface` ground, `2px` `--rule` bottom border with a `--rule-2` drop-shadow, plus a `header::before` riso mis-register — a `2px` terracotta hairline offset 3px right at `.5` opacity, evoking misaligned letterpress. The footer is the **engraving credit-line**: italic `--muted` Spectral, inset rule, carrying the A–D `.chip` seals and full data provenance (OpenStreetMap ODbL, Dahir et al. 2025 CC BY 4.0 recall ~0.63, DeFlock ALPR via OSM, MTA GTFS + NY OpenData ACE, US Census ACS diversity, NYC TLC rideshare density), with smart glasses flagged as a *scenario*.

### Design source files

- `web/index.html`
- `crates/app-interactive/src/ui.rs`
- `crates/app-interactive/src/main.rs`
- `.impeccable.md`
