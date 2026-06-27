//! Web-only bridge that drives a MapLibre GL basemap (rendered in a DOM canvas
//! *behind* the transparent Bevy canvas) from the Bevy camera. Bevy stays the
//! input owner — our existing drag/zoom/click model is untouched — and MapLibre
//! is a passive display layer synced each frame to whatever the Bevy ortho
//! camera is looking at, so the NYC Human Geography basemap aligns under the
//! ENU-meter sim layers.
//!
//! The camera's `Transform.scale.x` is meters-per-CSS-pixel (that's exactly how
//! `camera_control` converts a pixel drag into a world pan), which is MapLibre's
//! pixel convention too — so we hand JS the center (lon/lat) + meters-per-pixel
//! and it computes the matching zoom.

use bevy::prelude::*;
use wasm_bindgen::prelude::*;

use sim_core::{EnuProjection, Vec2};

#[wasm_bindgen]
extern "C" {
    /// Defined in `web/index.html`; no-ops until the MapLibre map is ready.
    #[wasm_bindgen(js_namespace = window, js_name = ourspaceSetView)]
    fn set_view(lon: f64, lat: f64, meters_per_px: f64);

    /// Defined in `web/index.html`: show/hide the basemap DOM layer (and lazily
    /// create the MapLibre map the first time it's shown).
    #[wasm_bindgen(js_namespace = window, js_name = ourspaceSetBasemapVisible)]
    fn set_basemap_visible(visible: bool);

    /// Defined in `web/index.html` (synchronously, before the wasm loads):
    /// the page's `prefers-reduced-motion` flag.
    #[wasm_bindgen(js_namespace = window, js_name = ourspaceReducedMotion)]
    fn reduced_motion() -> bool;
}

/// Read the page's reduced-motion preference (the Operators view snaps instead of
/// animating when true).
pub fn prefers_reduced_motion() -> bool {
    reduced_motion()
}

/// Push the Bevy camera's center + scale to the MapLibre basemap each frame —
/// but only while the basemap is toggled on (`Params.basemap_on`). Visibility is
/// pushed to the DOM only on change (idempotent, avoids per-frame DOM writes); when
/// off we skip the view sync entirely so a hidden map does no work.
pub fn sync_basemap(
    params: Res<crate::Params>,
    cam: Query<&Transform, With<Camera2d>>,
    mut last_visible: Local<Option<bool>>,
    mut last_view: Local<Option<(f32, f32, f32)>>,
) {
    // Hidden under the heatmap and the neighborhood choropleth too — both are density
    // overviews that want a clean ground, and a translucent fill over the street basemap
    // shows its building footprints through (the same artifact the Bevy footprints had).
    let want_visible = params.basemap_on
        && !params.heatmap_on
        && !(params.neighborhoods_on && params.choropleth_on);
    if *last_visible != Some(want_visible) {
        set_basemap_visible(want_visible);
        *last_visible = Some(want_visible);
        *last_view = None; // force one push when (re)shown
    }
    if !want_visible {
        return;
    }
    let Ok(t) = cam.single() else { return };
    // Only push to MapLibre when the camera actually moved. The clock-driven
    // time-lapse moves agents, not the camera, so a viewer just watching the sim
    // holds a static view — without this guard we'd call `map.jumpTo` every frame
    // (a fresh JS object + a full MapLibre re-render each time), churning the JS heap
    // and keeping the basemap busy forever. Gated, a static map goes idle.
    let view = (t.translation.x, t.translation.y, t.scale.x);
    if *last_view == Some(view) {
        return;
    }
    *last_view = Some(view);
    let proj = EnuProjection::default();
    let (lat, lon) = proj.to_wgs84(Vec2::new(view.0 as f64, view.1 as f64));
    set_view(lon, lat, view.2 as f64);
}
