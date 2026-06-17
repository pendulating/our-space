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
}

/// Push the Bevy camera's center + scale to the MapLibre basemap each frame.
pub fn sync_basemap(cam: Query<&Transform, With<Camera2d>>) {
    let Ok(t) = cam.single() else { return };
    let proj = EnuProjection::default();
    let (lat, lon) = proj.to_wgs84(Vec2::new(t.translation.x as f64, t.translation.y as f64));
    set_view(lon, lat, t.scale.x as f64);
}
