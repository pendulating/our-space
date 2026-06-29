//! A reusable "movable panel" pattern for floating egui overlays.
//!
//! Most floating overlays here are *pinned* with `egui::Area::anchor(...)`. A
//! **movable** overlay instead owns its top-left position in [`MovablePanels`] (a
//! Bevy resource keyed by a stable id). The pattern:
//!
//!   1. seed the position once from a default screen anchor,
//!   2. draw a **drag grip** the user can grab to reposition the whole panel, and
//!   3. clamp the result to the viewport so a panel can never be lost off-screen
//!      (and re-clamp on window resize).
//!
//! Inner widgets keep their own pointer handling — the grip is the *only* drag
//! surface — so a panel whose body is itself interactive (e.g. the time scrubber,
//! which drags to scrub) keeps working unchanged. Adopt it by replacing an
//! anchored `egui::Area { frame … }` with a single [`movable_panel`] call.

use std::collections::HashMap;

use bevy::prelude::Resource;
use bevy_egui::egui;

use crate::theme::ui as pal;

/// Persistent top-left positions for movable panels, keyed by a stable string id.
/// One resource holds every movable overlay; insert it once and pass it to the UI
/// system that draws them.
#[derive(Resource, Default)]
pub struct MovablePanels {
    pos: HashMap<&'static str, egui::Pos2>,
}

/// Height (px) of the drag-grip strip drawn at the top of a movable panel.
const GRIP_H: f32 = 13.0;

/// Show a movable floating panel and return its body's value.
///
/// * `id` — stable id (the egui Area id and the [`MovablePanels`] key).
/// * `width` — content width (px); the grip + body span it.
/// * `seed` — initial top-left given the screen rect, used until the panel has a
///   stored position (i.e. the first frame, or after a reset that clears the store).
/// * `frame` — the egui frame (fill / stroke / rounding / margin) to wrap content in.
/// * `add_contents` — the panel body, drawn just below the grip.
pub fn movable_panel<R>(
    ctx: &egui::Context,
    store: &mut MovablePanels,
    id: &'static str,
    width: f32,
    seed: impl FnOnce(egui::Rect) -> egui::Pos2,
    frame: egui::Frame,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let screen = ctx.screen_rect();
    let area_id = egui::Id::new(("movable", id));
    // Until the user drags the panel it sits at its default (the seed), recomputed
    // from the *live* screen each frame so it tracks resizes and never freezes at a
    // first-frame placeholder size. Once moved, the stored absolute position wins.
    let moved = store.pos.contains_key(id);
    let mut pos = store.pos.get(id).copied().unwrap_or_else(|| seed(screen));

    // The grip drives the move; we apply its delta *after* the show so the panel
    // re-renders at the new position next frame (how egui's own movable areas work).
    let mut drag_delta = egui::Vec2::ZERO;
    let area = egui::Area::new(area_id)
        .order(egui::Order::Middle)
        .fixed_pos(pos)
        .constrain(true)
        .show(ctx, |ui| {
            frame
                .show(ui, |ui| {
                    ui.set_width(width);
                    // Drag grip — the only surface that moves the panel.
                    let grip = ui
                        .allocate_response(egui::vec2(width, GRIP_H), egui::Sense::drag());
                    let active = grip.hovered() || grip.dragged();
                    paint_grip(ui.painter(), grip.rect, active);
                    if grip.dragged() {
                        drag_delta = grip.drag_delta();
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    } else if grip.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    ui.add_space(2.0);
                    add_contents(ui)
                })
                .inner
        });

    // Persist only once the panel has actually been moved: apply the drag and keep it
    // clamped fully on-screen (also re-clamps a moved panel after a window resize). An
    // untouched panel never writes to the store, so a first-frame placeholder screen
    // can't poison its position.
    if grip_dragged(drag_delta) || moved {
        let size = area.response.rect.size();
        pos += drag_delta;
        pos.x = pos.x.clamp(screen.left(), (screen.right() - size.x).max(screen.left()));
        pos.y = pos.y.clamp(screen.top(), (screen.bottom() - size.y).max(screen.top()));
        store.pos.insert(id, pos);
    }

    area.inner
}

/// Did the grip move this frame? (A non-zero drag delta.)
fn grip_dragged(delta: egui::Vec2) -> bool {
    delta != egui::Vec2::ZERO
}

/// Paint the grip: a centered row of dots, brighter when hovered/dragged.
fn paint_grip(painter: &egui::Painter, rect: egui::Rect, active: bool) {
    let color = if active { pal::ZINC_400 } else { pal::ZINC_600 };
    let (n, gap) = (6, 4.0_f32);
    let total = (n - 1) as f32 * gap;
    let y = rect.center().y;
    let x0 = rect.center().x - total / 2.0;
    for i in 0..n {
        painter.circle_filled(egui::pos2(x0 + i as f32 * gap, y), 1.3, color);
    }
}
