# our-space — 3D Map Mode (design notes)

> **Status: exploratory.** This captures the approach and the challenges for a
> future tilt/perspective ("3D") map mode. No implementation exists yet. The one
> load-bearing unknown — whether the current 2D scene can tilt as-is or needs a
> port to Bevy's 3D pipeline — is called out as a spike (§5) and gates everything
> downstream. See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the rendering stack and
> [`DESIGN.md`](DESIGN.md) for the visual system.

## 1. Goal & scope

A mode that **tilts the map into perspective** instead of the current strict
top-down view, and on the web build does so **in lockstep with the MapLibre
basemap** so the real NYC geography and our sim layers tilt together. "Tie into
MapLibre" is the crux: the basemap and the Bevy overlay must agree on one
perspective camera.

Two surfaces, deliberately kept at parity:
- **Native** (Bevy only): a tilted perspective view over the parchment background.
- **Web** (Bevy WebGPU canvas composited over a MapLibre WebGL basemap): the
  basemap pitches/rotates and the overlay matches it.

The motivating payoff for *this* app is less "pretty 3D buildings" and more
**extruding the surveillance layer into space** — FOV wedges becoming volumes,
fixed cameras sitting at mounting height, a walk route reading as a path through a
watched volume. That framing matters because it changes scope (§7, §8).

## 2. Current architecture (what we're tilting)

Today everything is **2D orthographic**, and the web composite is a simple
top-down affine:

- One camera: `commands.spawn((Camera2d, Transform::from_scale(Vec3::splat(6.0))))`
  (`main.rs:394`). No `Projection` component — implicit ortho. **1 world unit = 1 m**
  ENU (`world.rs:12`, `to_world`); `Transform.scale.x` = **meters per CSS pixel**.
- `camera_control` (`main.rs:697`): drag → `translation` (pan), scroll → `scale`
  (zoom, clamped 0.4–30 m/px). `handle_click` (`main.rs:776`) ray-picks via
  `viewport_to_world_2d`.
- **Web composite**: the Bevy canvas is transparent over a MapLibre GL basemap
  (DOM canvas behind, `z-index:0`, `pointer-events:none`). Each frame
  `sync_basemap` (`basemap.rs:37`) reads the camera and calls
  `ourspaceSetView(lon, lat, metersPerPx)` (extern `basemap.rs:21`; JS
  `web/index.html:192`), which does `map.jumpTo({center, zoom})` with
  `zoom = log2(78271.517·cos(lat) / metersPerPx)`. The basemap is initialized
  `pitch: 0` (`index.html:174`), `interactive: false` (`index.html:176`) — **Bevy
  owns all input**, MapLibre is a passive mirror. ENU→WGS84 via
  `EnuProjection::to_wgs84` (`sim-core/src/projection.rs:59`).
- **~21 sim layers**, all `Mesh2d` + `MeshMaterial2d<ColorMaterial>` (or `Text2d`),
  with `z` used as **painter order**, not height:

  | z | layer |
  |---|---|
  | −0.3 | equity choropleth |
  | 0.0 | streets |
  | 0.12 / 0.15 / 0.2 | walkshed edges / heatmap quad / ACE corridors + route line |
  | 0.5 | FOV wedges (per directional sensor) |
  | 1.0 | fixed-camera icons (3 merged meshes: CCTV/DOT/Flock) |
  | 1.6 | walkshed in-shed rings |
  | 2.4 / 2.6 / 2.7 | glasses peds / dashcam vehicles / ACE buses (`agents.rs:48-51`) |
  | 3.0 / 3.5 / 4.0 | A·B + walkshed center / capture pings / walker |
  | 5.0 | Operators-view column headers (`Text2d`) |

  There is **no** existing `Camera3d`, `Mesh3d`, `StandardMaterial`, or
  `PerspectiveProjection` anywhere — the app is pure 2D.

## 3. The core challenge

The web view is **two separate renderers**: Bevy on **WebGPU**, MapLibre on
**WebGL**, in two stacked DOM canvases. They cannot share a GL context or a depth
buffer. Top-down, aligning them is a 2-parameter affine (center + meters/px).
**Tilted, they must agree on a full perspective view-projection** — and the only
way to make two independent renderers agree is to **drive one camera from the
other**. So 3D mode is fundamentally a *camera-mirroring* problem, not a "set
pitch" toggle.

## 4. Recommended approach — mirror one perspective camera

Treat **one** camera state — `{center, zoom(=metersPerPx), pitch, bearing}` — as
the single source of truth, and reconstruct the *same* perspective camera in both
renderers each frame.

**Camera math** (MapLibre's model, to replicate in Bevy):
- MapLibre uses a fixed vertical **FOV ≈ 0.6435 rad (36.87°)** and
  `cameraToCenterDistance = 0.5 / tan(fov/2) · canvasHeightPx`. Convert to world
  meters with the existing `metersPerPx`: `dist_m = 0.5/tan(fov/2) · worldHeight_m`.
- Place the Bevy perspective camera at `center + dist_m · (back/up direction from
  pitch & bearing)`, `looking_at(center)`, with `Projection::Perspective { fov }`
  matched. At `pitch = 0` this reduces to today's straight-down view; the existing
  `metersPerPx` stays the zoom currency.
- Bearing rotates yaw about center; pitch tilts toward the horizon.

**Bridge extension** (web): `ourspaceSetView` gains `pitch, bearing` →
`map.jumpTo({center, zoom, pitch, bearing})`; raise `maxPitch` (default **60°**,
experimental up to **85°**) at map init.

**Camera ownership** — two viable directions, a real decision (§8):
- **(A) Bevy owns input** (consistent with today): Bevy computes pitch/bearing from
  user input and pushes all four params to MapLibre. Keeps the current model; we
  reimplement tilt/inertia in Bevy.
- **(B) MapLibre owns the 3D camera**: enable MapLibre interaction, read its
  battle-tested tilt/rotate/inertia via `move`/`pitch`/`rotate` events
  (`getCenter/getZoom/getPitch/getBearing`, or `getFreeCameraOptions()`), and push
  *that* into Bevy. Inverts today's "Bevy owns input" but gets polished 3D nav for
  free. (Picking/click-to-route would then need rethinking.)

## 5. The Bevy blocker — the spike that gates everything

**`Mesh2d` and `Text2d` render only on Bevy's 2D pipeline**, whose `Transparent2d`
phase sorts by `z` and whose depth handling assume **orthographic**. Attaching a
`Projection::Perspective` to the `Camera2d` *might* apply geometrically, but likely
produces transparency-sort and depth artifacts. **This is unverified** (the
research pass was cut short) and is the **decision gate**:

- **Cheap path (if it works):** swap the camera's projection to perspective and
  drive it per §4 — the existing ~21 layers tilt as-is. Lowest effort; needs a
  spike to confirm acceptable rendering.
- **Robust path (likely required):** port the sim layers to the **3D pipeline**:
  - `Mesh2d` → `Mesh3d`; `MeshMaterial2d<ColorMaterial>` → `MeshMaterial3d<StandardMaterial>`
    with `unlit: true` + `base_color`/`base_color_texture` (`ColorMaterial` is
    2D-only). The `merged_icon_quads` builder (`world.rs:33`) and every spawn site
    (`main.rs`, `agents.rs`, `operators.rs`) change.
  - `Text2d` has **no 3D equivalent** — the Operators-view headers (`operators.rs`)
    would need world-space billboarded text (render-to-texture quad, a billboard
    crate, or keep headers as a 2D/egui screen overlay drawn on top).
  - `z` painter-order → real height: flatten most layers to ~0, keep only tiny
    offsets for sort, or model genuine heights (camera poles, extruded wedges).

A half-day spike (perspective `Camera2d` vs a minimal `Camera3d`+`Mesh3d` quad over
the basemap) should settle which path we take before any real work.

## 6. Challenges & risks

- **No shared depth → overlay always on top.** Bevy's canvas composites over
  MapLibre; it cannot be occluded by MapLibre's 3D buildings. Fine for ground-plane
  markers; wrong for tall extruded geometry that should hide behind buildings.
  Mitigation: render buildings in Bevy too, or accept "x-ray" overlay.
- **Mercator vs ENU distortion under pitch.** The top-down sync matches scale at the
  view center (the `cos(lat)` term); ENU is a local tangent plane while MapLibre is
  Web Mercator, so they diverge slightly away from center. Pitch magnifies that
  divergence toward the tilted horizon. Acceptable at moderate pitch / city scale;
  worsens at high pitch and large extents.
- **z-as-painter-order becomes parallax.** The −0.3…5.0 stack would read as physical
  height in perspective (e.g. the walker floating "4 m" up). Small at city scale but
  should be deliberately flattened.
- **Camera ownership inversion** (§4 A vs B) ripples into input, picking, and the
  basemap sync direction.
- **Input model.** Need pitch/bearing controls (e.g. right-drag or modifier-drag to
  orbit/tilt) plus a 2D⇄3D toggle; `camera_control` (`main.rs:697`) currently maps
  both mouse buttons to pan.
- **Operators view assumes frozen 2D ortho.** `operators_layout` reads `scale.x` as
  m/px and lays columns into a screen-space rect (`operators.rs`). 3D and the
  Operators view are most likely **mutually exclusive** (entering Operators forces
  2D, or 3D is disabled while stacked).
- **3D-building data is unconfirmed.** MapLibre `fill-extrusion` needs height
  attributes (`render_height`) in the vector tiles. Whether the ArcGIS "NYC Human
  Geography" basemap carries them is **unknown — must check the style**. If absent,
  web "3D" is a *tilted flat basemap* (still meaningful) unless we extrude geometry
  ourselves in Bevy. Terrain DEM is low-value for flat Manhattan; globe projection
  is irrelevant at city scale.
- **Reduced motion / comfort.** A tilt animation should honor
  `prefers-reduced-motion` (already wired for the Operators view) and offer discrete
  pitch presets rather than only free-orbit.

## 7. Phased path

0. **Spike / decision gate (§5):** perspective-`Camera2d` vs `Camera3d`+`Mesh3d`.
   Output: the architecture decision. Everything below depends on it.
1. **Camera-mirror math + MapLibre tilt sync:** extend `ourspaceSetView` with
   pitch/bearing, raise `maxPitch`, and verify a pitched basemap tracks a pitched
   test camera at the view center.
2. **Bevy perspective camera:** the cheap projection swap *or* the `Mesh3d` port,
   per Phase 0.
3. **Input + UI:** pitch/bearing controls, a 2D/3D toggle in the panel, and
   reconciliation with the Operators view.
4. **(Optional) richer 3D:** building extrusions (if tile data supports), extruded
   FOV volumes / camera poles, sky + lighting — the "surveillance in space" payoff.

## 8. Open questions to resolve before building

- **Perspective-2D good enough, or commit to the `Mesh3d` port?** (Phase 0 answers
  the feasibility; this is the scope/appetite call.)
- **Camera ownership: Bevy-drives-MapLibre (A) or MapLibre-drives-Bevy (B)?**
- **Native 3D too, or web-only?** (Native has no basemap to tilt against.)
- **Goal: "tilt the existing map," or "extrude the surveillance into 3D"?** Very
  different effort and payoff.
- **Does the basemap style carry building heights** for `fill-extrusion`?
- **How does 3D coexist with the Operators view** — forced-2D, or disabled?

## References

- Code anchors: `main.rs:394` (camera), `main.rs:697` (`camera_control`),
  `basemap.rs:37` (`sync_basemap`) + externs `basemap.rs:21,26`,
  `web/index.html:174,176,192` (MapLibre init + bridge), `world.rs:12,33`
  (`to_world`, `merged_icon_quads`), `agents.rs:48-51` (z-order),
  `sim-core/src/projection.rs:59` (`to_wgs84`).
- MapLibre: [Map](https://maplibre.org/maplibre-gl-js/docs/API/classes/Map/),
  [CameraOptions](https://maplibre.org/maplibre-gl-js/docs/API/type-aliases/CameraOptions/),
  [MapOptions (maxPitch)](https://maplibre.org/maplibre-gl-js/docs/API/type-aliases/MapOptions/),
  [getFreeCameraOptions](https://maplibre.org/maplibre-gl-js/docs/API/classes/Map/),
  [Custom layer](https://maplibre.org/maplibre-gl-js/docs/API/interfaces/CustomLayerInterface/),
  [3D buildings](https://maplibre.org/maplibre-gl-js/docs/examples/display-buildings-in-3d/),
  [3D terrain](https://maplibre.org/maplibre-gl-js/docs/examples/3d-terrain/).
