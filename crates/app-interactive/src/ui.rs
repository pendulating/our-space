//! The egui control/results panel (native dev UI; the public web build will use
//! a DOM overlay per the plan).
//!
//! Information architecture: the right panel stays civic-simple — pick what you're
//! measuring (an *area* walkshed or an A→B *walk*), read the headline result in one
//! card, and reach the two everyday layers (neighborhoods, stack-by-operator). Every
//! power-user knob lives behind two clearly-labeled windows: **More layers** (heatmap,
//! equity, fields of view) and **Advanced** (the five sensor classes + their sliders,
//! playback speed, the simulation day).

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use sim_core::ConfidenceTier;

use crate::operators::{OperatorsView, COLS};
use crate::theme::{self, ui as pal};
use crate::{
    EguiWants, ExposureMode, HeatClass, Mode, Params, ResetRequested, RouteState, Sim, WalkLive,
    WalkshedState,
};

/// egui legend colors mirroring the map's heat gradient (paper low -> hot high):
/// light paper → pale amber → yellow → amber → orange. A caution ramp, never red.
const LEGEND: [egui::Color32; 6] = [
    egui::Color32::from_rgb(0xe4, 0xe4, 0xe7),
    egui::Color32::from_rgb(0xfd, 0xe6, 0x8a),
    egui::Color32::from_rgb(0xfa, 0xcc, 0x15),
    egui::Color32::from_rgb(0xf5, 0x9e, 0x0b),
    egui::Color32::from_rgb(0xf9, 0x73, 0x16),
    egui::Color32::from_rgb(0xea, 0x58, 0x0c),
];

/// The headline / accent color for every "big number" in the UI. A deep terracotta
/// (burnt orange) rather than the bright hazard-yellow accent: yellow on the white
/// paper plate / hero card barely read (~1.3:1), so the headline stat + the
/// "SIMULATED DAY" eyebrow now use this ~6:1-contrast clay instead. The yellow end of
/// the warning ramp is kept for the confidence tiers + heat ramp where it sits on ink.
const TERRACOTTA: egui::Color32 = egui::Color32::from_rgb(0xc2, 0x41, 0x0c);

/// Right side-panel width (CSS px); the citywide nudge keeps clear of it.
const PANEL_WIDTH_PX: f32 = 316.0;

/// World (ENU metres) anchors for the Manhattan→citywide nudge — one out in each outer
/// borough's empty space, ringing the island (Brooklyn SE, Queens E, the Bronx N, Staten
/// Island SW). Each sits well off Manhattan, so they're all off-screen in the default
/// island view; whichever one the visitor pans into frame is where the prompt appears,
/// labelled with that borough. `(eyebrow, east, north)`.
const CITYWIDE_NUDGE_ANCHORS: [(&str, f32, f32); 4] = [
    ("BROOKLYN", 2600.0, -12800.0),
    ("QUEENS", 4800.0, -4200.0),
    ("THE BRONX", 3600.0, 6200.0),
    ("STATEN ISLAND", -9500.0, -16500.0),
];

/// Navigate to the full five-borough build (`?city=nyc`). Web only — sets the query
/// string (which reloads into the citywide asset set); a no-op on native.
#[cfg(target_arch = "wasm32")]
fn go_citywide() {
    if let Some(w) = web_sys::window() {
        let _ = w.location().set_search("city=nyc");
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn go_citywide() {}

/// The Manhattan→citywide nudge, **pinned to map locations in the outer boroughs** rather
/// than the screen — it only surfaces once the visitor pans an outer borough into view,
/// and that act of panning out into the empty space *is* the prompt to open the full-city
/// build. A separate egui pass from `ui_panel` (no shared params); reads the camera to
/// project each borough anchor to a screen position and draws the box at the first one
/// that sits comfortably inside the visible map area (only one at a time, even when zoomed
/// out far enough to see two). Hidden in the citywide build.
pub fn citywide_nudge(
    mut contexts: EguiContexts,
    camera_q: bevy::prelude::Query<
        (&bevy::prelude::Camera, &bevy::prelude::GlobalTransform),
        bevy::prelude::With<bevy::prelude::Camera2d>,
    >,
    city: Option<bevy::prelude::Res<crate::CityScope>>,
    theme_ready: bevy::prelude::Res<crate::ThemeReady>,
) {
    if !theme_ready.0 {
        return;
    }
    if city.map(|c| c.citywide).unwrap_or(false) {
        return; // the full-city build is already here — no nudge
    }
    let Ok((camera, cam_gt)) = camera_q.single() else { return };
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let r = ctx.screen_rect();
    let map_w = r.width() - PANEL_WIDTH_PX;
    for (borough, ax, ay) in CITYWIDE_NUDGE_ANCHORS {
        let anchor = bevy::prelude::Vec3::new(ax, ay, 0.0);
        let Ok(screen) = camera.world_to_viewport(cam_gt, anchor) else { continue };
        // Only draw while the whole box fits inside the map area (left of the panel) with a
        // margin — so it reads as pinned in the outer-borough space, never clipped at an edge.
        if screen.x < 20.0
            || screen.x > map_w - 252.0
            || screen.y < 20.0
            || screen.y > r.height() - 160.0
        {
            continue;
        }
        egui::Area::new(egui::Id::new("citywide_nudge"))
            .fixed_pos(egui::pos2(screen.x, screen.y))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 235))
                    .stroke(egui::Stroke::new(1.0, pal::ZINC_300))
                    .inner_margin(egui::Margin::symmetric(13, 11))
                    .corner_radius(8)
                    .show(ui, |ui| {
                        ui.set_max_width(236.0);
                        ui.spacing_mut().item_spacing.y = 5.0;
                        ui.label(
                            egui::RichText::new(borough)
                                .font(theme::display(11.0))
                                .color(TERRACOTTA),
                        );
                        ui.label(
                            egui::RichText::new(
                                "This view is Manhattan only, kept light for speed. The outer \
                                 boroughs are empty here.",
                            )
                            .size(12.5)
                            .color(pal::ZINC_100),
                        );
                        let go = ui.add(
                            egui::Label::new(
                                egui::RichText::new("Open the full five-borough map →")
                                    .size(12.5)
                                    .strong()
                                    .color(TERRACOTTA),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if go.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if go.clicked() {
                            go_citywide();
                        }
                        ui.label(
                            egui::RichText::new("a powerful computer is recommended")
                                .size(10.5)
                                .italics()
                                .color(pal::ZINC_400),
                        );
                    });
            });
        break; // one nudge at a time — the first anchor in view wins
    }
}

/// Short label + ink color for a confidence tier — a warning ramp from the bright,
/// mapped certainty (yellow) down to the gray "machine" of pure speculation.
fn tier_style(tier: ConfidenceTier) -> (&'static str, egui::Color32) {
    match tier {
        ConfidenceTier::A => ("A · mapped", pal::YELLOW),
        ConfidenceTier::B => ("B · estimated", pal::AMBER),
        ConfidenceTier::C => ("C · modeled", pal::ORANGE),
        ConfidenceTier::D => ("D · speculative", pal::ZINC_400),
    }
}

/// One-shot setup (runs in `Update`, before the egui pass): apply the dark theme +
/// install the display fonts, then flip `ThemeReady` so `ui_panel` starts drawing.
/// Doing this *before* the pass means the new fonts are baked by the pass's
/// `begin_pass`, so the poster `display` family exists the first time it's used.
pub fn setup_theme(
    mut contexts: EguiContexts,
    mut ready: ResMut<crate::ThemeReady>,
) {
    if ready.0 {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return; // egui context not created yet — retry next frame
    };
    apply_theme(ctx);
    ready.0 = true;
}

/// Apply the paper egui theme: light zinc surfaces, dark ink text, hazard-yellow
/// accents (the inverted `ZINC_*` ramp does most of the work). Installs the fonts.
fn apply_theme(ctx: &egui::Context) {
    let stroke = |c| egui::Stroke::new(1.0, c);
    let mut v = egui::Visuals::light();
    v.panel_fill = pal::ZINC_900; // #f4f4f5 light panel
    v.window_fill = pal::ZINC_900;
    v.faint_bg_color = pal::ZINC_800;
    v.extreme_bg_color = pal::ZINC_800;
    v.override_text_color = Some(pal::ZINC_100); // ink
    v.hyperlink_color = pal::YELLOW;
    v.window_stroke = stroke(pal::ZINC_700);
    v.widgets.noninteractive.bg_fill = pal::ZINC_900;
    v.widgets.noninteractive.bg_stroke = stroke(pal::ZINC_700);
    v.widgets.noninteractive.fg_stroke = stroke(pal::ZINC_100);
    v.widgets.inactive.bg_fill = pal::ZINC_800;
    v.widgets.inactive.weak_bg_fill = pal::ZINC_800;
    v.widgets.inactive.fg_stroke = stroke(pal::ZINC_300);
    v.widgets.hovered.bg_fill = pal::ZINC_700;
    v.widgets.hovered.weak_bg_fill = pal::ZINC_700;
    v.widgets.hovered.fg_stroke = stroke(pal::ZINC_100);
    // Active / pressed = the hazard yellow, with dark ink riding on top.
    v.widgets.active.bg_fill = pal::YELLOW;
    v.widgets.active.weak_bg_fill = pal::YELLOW;
    v.widgets.active.fg_stroke = stroke(pal::ZINC_100);
    v.selection.bg_fill = pal::YELLOW.gamma_multiply(0.42);
    v.selection.stroke = stroke(pal::YELLOW);
    ctx.set_visuals(v);
    theme::install_fonts(ctx);
}

/// A small all-caps section eyebrow used to break the slim panel into a couple of
/// scannable groups without a wall of separators.
fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .strong()
            .size(12.0)
            .color(pal::ZINC_400),
    );
    ui.add_space(3.0);
}

/// One address search box with a debounced GeoSearch autocomplete dropdown. Picking
/// a row commits it; `apply_geocode` then turns it into the walkshed center / A / B
/// and recomputes — map clicks still work alongside it.
fn address_field(
    ui: &mut egui::Ui,
    geo: &mut crate::geocode::Geocoder,
    field: crate::geocode::Field,
    hint: &str,
) {
    let mut picked: Option<crate::geocode::GeoResult> = None;
    let mut cleared = false;
    let (loading, results, resolved, no_match, empty) = {
        let f = geo.field_mut(field);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 28.0).max(48.0); // leave room for the ✕
            let resp = ui.add(
                egui::TextEdit::singleline(&mut f.query).hint_text(hint).desired_width(w),
            );
            if resp.changed() {
                f.on_edit();
            }
            if ui
                .add(egui::Button::new(egui::RichText::new("✕").size(12.0)).frame(false))
                .on_hover_text("Clear")
                .clicked()
            {
                cleared = true;
            }
        });
        (
            f.loading,
            f.results.clone(),
            f.resolved.is_some(),
            f.no_match,
            f.query.trim().is_empty(),
        )
    };
    if cleared {
        geo.field_mut(field).clear();
        geo.cleared = Some(field); // also drop the endpoint / walkshed in `apply_geocode`
        ui.add_space(3.0);
        return;
    }
    // Show pending candidates for an unresolved query (gated on results, not focus,
    // so clicking a row never races the text box losing focus).
    if !resolved && !results.is_empty() {
        egui::Frame::new()
            .fill(pal::ZINC_800)
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                for r in &results {
                    if ui
                        .selectable_label(false, egui::RichText::new(&r.label).size(12.0))
                        .clicked()
                    {
                        picked = Some(r.clone());
                    }
                }
            });
    } else if loading {
        ui.label(egui::RichText::new("searching…").weak().size(11.0));
    } else if no_match && !empty && !resolved {
        ui.label(
            egui::RichText::new("No matches in Manhattan.").size(11.0).color(pal::ORANGE),
        );
    }
    if let Some(r) = picked {
        {
            let f = geo.field_mut(field);
            f.query = r.label.clone();
            f.resolved = Some(r.clone());
            f.results.clear();
        }
        geo.picked = Some((field, r));
    }
    ui.add_space(3.0);
}

pub fn ui_panel(
    mut contexts: EguiContexts,
    route: Res<RouteState>,
    mut params: ResMut<Params>,
    mut reset: ResMut<ResetRequested>,
    mut wants: ResMut<EguiWants>,
    sim: Option<bevy::prelude::Res<Sim>>,
    walk_live: bevy::prelude::Res<WalkLive>,
    walkshed: bevy::prelude::Res<WalkshedState>,
    mut ov: ResMut<OperatorsView>,
    pick: bevy::prelude::Res<crate::NeighborhoodPick>,
    nbhd_live: bevy::prelude::Res<crate::NeighborhoodLive>,
    mut clock: ResMut<crate::SimClock>,
    date: bevy::prelude::Res<crate::SimDate>,
    (mut more_open, mut advanced_open, alpr_makers): (
        bevy::prelude::Local<bool>,
        bevy::prelude::Local<bool>,
        bevy::prelude::Res<crate::AlprMakerBreakdown>,
    ),
    theme_ready: bevy::prelude::Res<crate::ThemeReady>,
    mut geocoder: ResMut<crate::geocode::Geocoder>,
) {
    let corr = sim.as_ref().and_then(|s| s.equity_corr);
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    // The display fonts are installed by `setup_theme` (in Update, before this pass).
    // Skip drawing until they're live, so the poster `display` family always exists
    // by the time the masthead/headline request it (egui panics otherwise).
    if !theme_ready.0 {
        wants.pointer = ctx.wants_pointer_input() || ctx.is_pointer_over_area();
        wants.keyboard = ctx.wants_keyboard_input();
        return;
    }

    // The simulated day, parked large at the map's top-left (over the canvas). One baked real
    // day drives the buses + taxis, so this names which day you're watching. Non-interactive,
    // on a faint paper plate so the date reads over the colorful choropleth too.
    egui::Area::new(egui::Id::new("sim_date"))
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(14.0, 12.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 220))
                .inner_margin(egui::Margin::symmetric(11, 7))
                .corner_radius(7)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(
                        egui::RichText::new("SIMULATED DAY")
                            .font(theme::display(10.0))
                            .color(TERRACOTTA),
                    );
                    ui.label(
                        egui::RichText::new(&date.label)
                            .font(theme::display(24.0))
                            .color(pal::ZINC_100),
                    );
                });
        });

    // (The Manhattan→citywide nudge is its own pass, `citywide_nudge`, pinned to a map
    // location in the outer boroughs so it only surfaces when you pan out there.)

    egui::SidePanel::right("panel")
        // Fixed width: a resizable panel + an `available_width()`-sized child (the
        // address box) feed back into each other and the panel grows without bound.
        .resizable(false)
        .exact_width(PANEL_WIDTH_PX)
        .show(ctx, |ui| {
            // ---- masthead ----
            // The page's HTML <header> already carries the "our·space" wordmark, so the
            // panel leads with the question instead (no duplicate title).
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("How watched is a place in Manhattan?")
                    .font(theme::display(20.0))
                    .color(pal::ZINC_100),
            );
            ui.add_space(10.0);

            // ---- what am I measuring? (the one decision the visitor makes) ----
            ui.horizontal(|ui| {
                // Re-clicking the mode you're already in clears its placed area/walk — a
                // toggle-off affordance (same effect as Reset, scoped to the active mode).
                // Only when something is actually placed, so an idle re-click is a no-op.
                let in_walkshed = params.mode == Mode::Walkshed;
                let in_route = params.mode == Mode::Route;
                let in_neigh = params.mode == Mode::Neighborhoods;
                let area_present = walkshed.summary.is_some();
                let route_present = route.a.is_some() || route.summary.is_some();
                let mut r_walk = ui.selectable_value(&mut params.mode, Mode::Walkshed, "My area");
                let mut r_route = ui.selectable_value(&mut params.mode, Mode::Route, "A walk A→B");
                let mut r_neigh =
                    ui.selectable_value(&mut params.mode, Mode::Neighborhoods, "Neighborhoods");
                if in_walkshed && area_present {
                    r_walk = r_walk.on_hover_text("Click again to clear your area");
                }
                if in_route && route_present {
                    r_route = r_route.on_hover_text("Click again to clear your A→B walk");
                }
                if in_neigh {
                    r_neigh = r_neigh.on_hover_text("Click again to leave Neighborhoods");
                }
                if (in_walkshed && area_present && r_walk.clicked())
                    || (in_route && route_present && r_route.clicked())
                {
                    reset.0 = true;
                }
                // Re-clicking the active Neighborhoods chip leaves the mode (back to the
                // default "My area"). `selectable_value` re-asserts Neighborhoods on click,
                // so flip it back when it was already active.
                if in_neigh && r_neigh.clicked() {
                    params.mode = Mode::Walkshed;
                }
            });
            // The neighborhood-density view is a mode now (not an EXPLORE checkbox): keep the
            // layer flag in lockstep with the mode so every choropleth system follows the
            // selector, and the My-area / walk inputs disappear while it's active.
            params.neighborhoods_on = matches!(params.mode, Mode::Neighborhoods);
            ui.label(
                egui::RichText::new(match params.mode {
                    Mode::Walkshed => {
                        "Search an address or click the map. Every camera within a 10-minute walk."
                    }
                    Mode::Route => "Search a start and destination, or click the map to set A then B.",
                    Mode::Neighborhoods => {
                        "Camera density by neighborhood. Hover a region for its breakdown."
                    }
                })
                .weak()
                .size(12.0),
            );
            ui.add_space(6.0);

            // ---- address search (type a place; or click the map) ----
            use crate::geocode::Field;
            match params.mode {
                Mode::Walkshed => {
                    address_field(ui, &mut geocoder, Field::Walkshed, "Search an address or place");
                    if ui
                        .add(egui::Button::new(egui::RichText::new("🎲  surprise me").size(12.0)).frame(false))
                        .on_hover_text("Drop the study area on a random Manhattan corner")
                        .clicked()
                    {
                        geocoder.random_walkshed = true;
                    }
                }
                Mode::Route => {
                    address_field(ui, &mut geocoder, Field::Start, "Start (A)");
                    address_field(ui, &mut geocoder, Field::Dest, "Destination (B)");
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Button::new(egui::RichText::new("⇅  swap A / B").size(12.0)).frame(false))
                            .on_hover_text("Reverse start and destination")
                            .clicked()
                        {
                            geocoder.swap = true;
                        }
                        if ui
                            .add(egui::Button::new(egui::RichText::new("🎲  random walk").size(12.0)).frame(false))
                            .on_hover_text("Route between two random Manhattan corners")
                            .clicked()
                        {
                            geocoder.random_route = true;
                        }
                    });
                }
                Mode::Neighborhoods => {
                    // No address search in this mode — just the scope toggle. The hovered
                    // region's breakdown lands in the result card below.
                    ui.checkbox(&mut params.neighborhoods_all, "Include all five boroughs");
                }
            }
            ui.add_space(8.0);

            // ---- the result card (the hero) ----
            egui::Frame::new()
                .fill(pal::ZINC_800)
                .stroke(egui::Stroke::new(1.0, pal::ZINC_700))
                .inner_margin(egui::Margin::same(14))
                .corner_radius(10)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match params.mode {
                        Mode::Walkshed => result_walkshed(ui, &route, &walkshed),
                        Mode::Route => {
                            result_route(ui, &mut params, &route, &walk_live, clock.time_of_day)
                        }
                        Mode::Neighborhoods => {
                            result_neighborhoods(ui, sim.as_deref(), &pick, &nbhd_live)
                        }
                    }
                });

            ui.add_space(12.0);

            // ---- everyday layers (one tap from the main view) ----
            section(ui, "EXPLORE");
            if ui
                .selectable_label(ov.active, "Stack sensors by operator")
                .on_hover_text("Pull every sensor off the map into one column per company")
                .clicked()
            {
                ov.active = !ov.active;
            }
            if ov.active {
                ui.indent("ops", |ui| {
                    ui.label(
                        egui::RichText::new("Every sensor, sorted by who runs it.")
                            .italics()
                            .weak()
                            .size(12.0),
                    );
                    for &c in &COLS {
                        ui.label(
                            egui::RichText::new(format!("{}: {}", c.label(), c.gloss()))
                                .weak()
                                .small(),
                        );
                        // The ALPR column bands by manufacturer — name the strata with
                        // a matching color swatch so the tower's bands are legible.
                        if c == crate::operators::OperatorCol::Flock && !alpr_makers.0.is_empty() {
                            ui.indent("alpr_makers", |ui| {
                                for (m, n) in &alpr_makers.0 {
                                    ui.horizontal(|ui| {
                                        let c = m.color().to_srgba();
                                        let sw = egui::Color32::from_rgb(
                                            (c.red * 255.0) as u8,
                                            (c.green * 255.0) as u8,
                                            (c.blue * 255.0) as u8,
                                        );
                                        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                                        ui.painter().rect_filled(rect, 1.5, sw);
                                        ui.label(
                                            egui::RichText::new(format!("{}  ·  {}", m.label(), n))
                                                .small()
                                                .color(pal::ZINC_300),
                                        );
                                    });
                                }
                            });
                        }
                    }
                });
            }

            // Camera-density choropleth ("the heatmap") — an Explore toggle that lives
            // inside Neighborhoods mode, parallel to the operators stack. Off the bare
            // hover-browser so the default citywide view stays light.
            if matches!(params.mode, Mode::Neighborhoods) {
                if ui
                    .selectable_label(params.choropleth_on, "Camera-density heatmap")
                    .on_hover_text(
                        "Shade each neighborhood by cameras per km² (with live counts when zoomed in)",
                    )
                    .clicked()
                {
                    params.choropleth_on = !params.choropleth_on;
                }
            }

            ui.add_space(12.0);
            ui.separator();

            // ---- "In 5 years…" — the speculative-future surveillance layers (AI smart
            // glasses + sidewalk delivery robots), grouped behind one switch, off by
            // default. `set_future` flips both layers together; StoryMaps reuse it. ----
            ui.add_space(4.0);
            let fut = params.future_on;
            let btn = egui::Button::new(
                egui::RichText::new(if fut { "🔮  In 5 years…  ·  on" } else { "🔮  In 5 years…" })
                    .size(13.0)
                    .strong()
                    .color(if fut { pal::ZINC_950 } else { pal::ZINC_300 }),
            )
            .fill(if fut { pal::ORANGE } else { pal::ZINC_900 })
            .min_size(egui::vec2(ui.available_width(), 26.0));
            if ui
                .add(btn)
                .on_hover_text(
                    "Speculative near-future surveillance: smart glasses on pedestrians and \
                     sidewalk delivery-robot cameras. Neither is common in NYC yet. \
                     Off by default.",
                )
                .clicked()
            {
                params.set_future(!fut);
            }
            ui.label(
                egui::RichText::new(if fut {
                    "Showing speculative layers: smart glasses + delivery robots."
                } else {
                    "Add the speculative layers: smart glasses + sidewalk delivery robots."
                })
                .size(10.5)
                .weak(),
            );
            ui.add_space(8.0);

            // ---- the two menus that hold everything else ----
            ui.horizontal(|ui| {
                if ui
                    .button("More layers…")
                    .on_hover_text("Citywide heatmap, equity overlay, fields of view")
                    .clicked()
                {
                    *more_open = !*more_open;
                }
                if ui
                    .button("Advanced…")
                    .on_hover_text("Sensor classes, playback speed, the simulation day")
                    .clicked()
                {
                    *advanced_open = !*advanced_open;
                }
            });
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                if ui.button("Reset").clicked() {
                    reset.0 = true;
                }
                ui.label(
                    egui::RichText::new("Drag: pan · Scroll: zoom · WASD: pan")
                        .weak()
                        .small(),
                );
            });

            ui.add_space(6.0);
            ui.collapsing("About the data & its limits", |ui| about_data(ui));
        });

    // ---- "More layers" window: the citywide / overlay views ----
    if *more_open {
        let mut open = true;
        egui::Window::new("More layers")
            .open(&mut open)
            .resizable(false)
            .default_width(308.0)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-340.0, 16.0))
            .show(ctx, |ui| {
                section(ui, "CITYWIDE HEATMAP");
                ui.checkbox(&mut params.heatmap_on, "Show exposure heatmap");
                if params.heatmap_on {
                    egui::ComboBox::from_label("layer")
                        .selected_text(params.heatmap_class.label())
                        .show_ui(ui, |ui| {
                            for c in [HeatClass::Fixed, HeatClass::Ace, HeatClass::Dashcam, HeatClass::Total] {
                                ui.selectable_value(&mut params.heatmap_class, c, c.label());
                            }
                        });
                    ui.horizontal(|ui| {
                        ui.label("low");
                        for c in LEGEND {
                            ui.colored_label(c, "■");
                        }
                        ui.label("high");
                    });
                    ui.label(
                        egui::RichText::new(
                            "Expected devices per minute, as a field over space. Updates with the \
                             chosen hour and slider settings.",
                        )
                        .weak()
                        .size(11.0),
                    );
                }

                ui.add_space(8.0);
                section(ui, "EQUITY OVERLAY");
                ui.checkbox(&mut params.equity_on, "Show neighborhood diversity");
                if params.equity_on {
                    ui.label(
                        egui::RichText::new(
                            "Block-group Shannon diversity (dim = homogeneous, bright = diverse)",
                        )
                        .weak()
                        .size(11.0),
                    );
                    if let Some(r) = corr {
                        ui.label(
                            egui::RichText::new(format!(
                                "In this data, diversity vs. detected cameras correlate r = {r:+.2}."
                            ))
                            .color(egui::Color32::from_rgb(0x34, 0x51, 0x69)),
                        );
                    }
                    ui.label(
                        egui::RichText::new(
                            "Dahir et al. (2025): cameras are most common in racially diverse \
                             neighborhoods. Diversity predicts cameras more than crime does. A \
                             published correlation, not causation for any one block.",
                        )
                        .weak()
                        .size(11.0),
                    );
                }

                ui.add_space(8.0);
                section(ui, "ON THE MAP");
                ui.checkbox(&mut params.basemap_on, "Street basemap")
                    .on_hover_text("Draw the MapLibre NYC street map under the sim (web only; off by default)");
                ui.checkbox(&mut params.outline_on, "Manhattan outline")
                    .on_hover_text("Trace the borough coastline as a frame around the street network");
                ui.checkbox(&mut params.buildings_on, "Building footprints")
                    .on_hover_text("Every Manhattan building as a flat dark fill under the streets");
                ui.checkbox(&mut params.parks_on, "Parks")
                    .on_hover_text("NYC parks as a flat green fill under the streets (NYC Parks Properties)");
                ui.checkbox(&mut params.plazas_on, "Pedestrian plazas")
                    .on_hover_text("NYC DOT pedestrian plazas as a hatched concrete fill (Astor Place, etc.)");
                ui.checkbox(&mut params.landmarks_on, "Landmark buildings (3D)")
                    .on_hover_text("Notable buildings (Empire State, One WTC, the Cloisters…) as 2.5D massing for orientation");
                ui.checkbox(&mut params.linknyc_on, "LinkNYC kiosks")
                    .on_hover_text(
                        "1,225 LinkNYC Wi-Fi/phone kiosks (sky-blue). Not cameras, but each \
                         becomes a surveillance node once you connect to its free Wi-Fi \
                         (device MAC, session data, location).",
                    );
                ui.checkbox(&mut params.show_ace, "Show ACE bus-lane corridors (blue)")
                    .on_hover_text(
                        "Trace the ACE bus-lane routes as blue ribbons. Off by default. The \
                         moving ACE buses still run without them.",
                    );
                if params.show_ace {
                    // Surveillance *evidence*, intertwined with the ACE layer: these
                    // corridors aren't passive paint — the bus lanes they trace generate
                    // hundreds of thousands of camera/agent violations and tens of
                    // millions in fines. The dataset (nc67-uf89) has no coordinates, so
                    // it rides the ACE toggle as a narrative stat.
                    egui::Frame::new()
                        .fill(pal::ZINC_800)
                        .corner_radius(6)
                        .inner_margin(egui::Margin::same(7))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("⚖ Surveillance evidence")
                                    .strong()
                                    .size(11.5)
                                    .color(pal::ORANGE),
                            );
                            ui.label(
                                egui::RichText::new(format!(
                                    "Manhattan's ACE bus lanes have logged \
                                     {} violations and {} in fines.",
                                    crate::group_thousands(crate::ACE_BUS_LANE_VIOLATIONS),
                                    crate::compact_usd(crate::ACE_BUS_LANE_FINES_USD),
                                ))
                                .size(11.0),
                            );
                            ui.label(
                                egui::RichText::new("NYC Open Parking & Camera Violations · nc67-uf89")
                                    .size(9.0)
                                    .italics()
                                    .weak(),
                            );
                        });
                }
                ui.checkbox(&mut params.show_fov, "Show camera fields of view");
            });
        if !open {
            *more_open = false;
        }
    }

    // ---- "Advanced" window: the sensing-class rig + playback + the sim day ----
    if *advanced_open {
        let mut open = true;
        egui::Window::new("Advanced")
            .open(&mut open)
            .resizable(false)
            .default_width(330.0)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-340.0, 16.0))
            .show(ctx, |ui| {
                section(ui, "SENSING CLASSES");
                ui.label(
                    egui::RichText::new(
                        "Which moving and fixed sensors the estimate counts, and the per-device \
                         assumptions behind each. Defaults are sensible. Tune only if curious.",
                    )
                    .weak()
                    .size(11.0),
                );
                ui.add_space(4.0);
                ui.checkbox(&mut params.ace_on, "ACE bus cameras  (A · mapped)");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut params.dashcam_on, "Rideshare cams");
                    ui.add_enabled(
                        params.dashcam_on,
                        egui::Slider::new(&mut params.dashcam_penetration, 0.0..=1.0)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    );
                });
                ui.label(
                    egui::RichText::new("density follows real Uber/Lyft trips (TLC)")
                        .weak()
                        .size(11.0),
                );
                // Smart glasses + delivery robots are the speculative layers behind the
                // "In 5 years…" button; their density is tunable here once it's on.
                let fut = params.future_on;
                ui.add_enabled_ui(fut, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Smart glasses (G)");
                        ui.add(
                            egui::Slider::new(&mut params.glasses_per_1000, 0.0..=50.0)
                                .text("/1k peds"),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Delivery robots (D)");
                        ui.add(
                            egui::Slider::new(&mut params.robots_density, 0.0..=8.0).text("/min"),
                        );
                    });
                });
                ui.label(
                    egui::RichText::new(if fut {
                        "G + D · speculative. Not yet in NYC; robot density follows the \
                         Robotability Score (IRL-CT)"
                    } else {
                        "G + D · speculative. Enable with the “In 5 years…” button"
                    })
                    .weak()
                    .size(11.0),
                );
                ui.horizontal(|ui| {
                    ui.checkbox(&mut params.tesla_on, "Tesla cameras");
                    ui.add_enabled(
                        params.tesla_on,
                        egui::Slider::new(&mut params.tesla_density, 0.0..=12.0).text("/min"),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "C · always-on Sentry/Autopilot; density from private NYS DMV Tesla \
                         registrations by ZIP (~29k in NYC; +7% of FHVs are Teslas)",
                    )
                    .weak()
                    .size(11.0),
                );
                ui.add_space(4.0);
                ui.checkbox(&mut params.show_agents, "Animate moving sensors on the map");
                ui.label(
                    egui::RichText::new(
                        "clay = rideshare · slate = glasses · buses = ACE · violet = robots · \
                         crimson = Teslas; buses + taxis replay the day's real schedule",
                    )
                    .weak()
                    .size(11.0),
                );

                ui.add_space(10.0);
                section(ui, "PLAYBACK SPEED");
                ui.horizontal(|ui| {
                    let mut rate = clock.rate;
                    if ui
                        .add(
                            egui::Slider::new(&mut rate, crate::SIM_RATE_MIN..=crate::SIM_RATE_MAX)
                                .logarithmic(true)
                                .show_value(false),
                        )
                        .changed()
                    {
                        clock.rate = rate;
                    }
                    let mins = 86400.0 / (clock.rate * 60.0);
                    ui.label(
                        egui::RichText::new(if mins >= 1.0 {
                            format!("1 day ≈ {:.0} min", mins)
                        } else {
                            format!("1 day ≈ {:.0} s", mins * 60.0)
                        })
                        .weak(),
                    );
                });

                ui.add_space(10.0);
                section(ui, "SIMULATION DAY");
                ui.label(egui::RichText::new(&date.label).strong());
                ui.label(
                    egui::RichText::new(
                        "The clock replays this real day's actual trips: MTA ACE buses (real GTFS \
                         timetable) and rideshare (real NYC TLC records). Baked into the build; \
                         changing it means re-baking with another date.",
                    )
                    .weak()
                    .size(11.0),
                );
            });
        if !open {
            *advanced_open = false;
        }
    }

    // ---- bottom-center live day clock (hidden while the Operators view is up) ----
    if !ov.active {
        egui::Area::new(egui::Id::new("time_bar"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -18.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal::ZINC_900)
                    .stroke(egui::Stroke::new(1.5, pal::ZINC_700))
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .corner_radius(8)
                    .show(ui, |ui| {
                        ui.set_width(440.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button(if clock.playing { "  Pause  " } else { "  Play  " })
                                .clicked()
                            {
                                clock.playing = !clock.playing;
                            }
                            let h = clock.time_of_day.floor() as i32;
                            let m = (clock.time_of_day.fract() * 60.0).floor() as i32;
                            // Monospace so the digits don't reflow as the clock ticks
                            // (the egui analog of `tabular-nums`).
                            ui.label(
                                egui::RichText::new(format!("{:02}:{:02}", h, m))
                                    .monospace()
                                    .strong()
                                    .size(18.0),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new("drag the bar to scrub · speed in Advanced")
                                    .weak()
                                    .size(11.0),
                            );
                        });
                        // The stylized bar — drag anywhere on it to scrub the time.
                        let (resp, painter) = ui
                            .allocate_painter(egui::vec2(440.0, 30.0), egui::Sense::click_and_drag());
                        paint_time_bar(&painter, resp.rect, clock.time_of_day);
                        if resp.dragged() || resp.clicked() {
                            if let Some(pos) = resp.interact_pointer_pos() {
                                let t = ((pos.x - resp.rect.left()) / resp.rect.width())
                                    .clamp(0.0, 1.0);
                                clock.time_of_day = (t as f64 * 24.0).clamp(0.0, 24.0 - 1e-6);
                                clock.playing = false; // scrubbing pauses the clock
                            }
                        }
                    });
            });
    }

    wants.pointer = ctx.wants_pointer_input() || ctx.is_pointer_over_area();
    wants.keyboard = ctx.wants_keyboard_input();
}

/// The walkshed result inside the hero card.
fn result_walkshed(ui: &mut egui::Ui, route: &RouteState, walkshed: &WalkshedState) {
    if let Some(w) = &walkshed.summary {
        ui.label(
            egui::RichText::new(format!("~{} cameras", w.cameras_corrected.round() as u32))
                .font(theme::display(30.0))
                .color(TERRACOTTA),
        );
        ui.label(egui::RichText::new("watch your area").size(15.0).color(pal::ZINC_300));
        ui.add_space(6.0);
        ui.label(format!(
            "within a {:.0}-minute walk · {} street segments reachable",
            w.max_minutes, w.reachable_edges
        ));
        ui.label(
            egui::RichText::new(if (w.cameras_corrected - w.cameras_raw as f64).abs() > 0.5 {
                format!("{} detected, recall-corrected", w.cameras_raw)
            } else {
                format!("{} mapped (crowdsourced census + ML detections)", w.cameras_raw)
            })
            .weak()
            .size(12.0),
        );
        if let Some(hint) = &walkshed.status {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(hint).italics().weak().size(12.0));
        }
    } else {
        ui.label(
            egui::RichText::new(if route.status.is_empty() {
                "Click a point to map its 10-minute walkshed."
            } else {
                &route.status
            })
            .strong(),
        );
    }
}

/// The neighborhood-density result inside the hero card: the breakdown for whichever
/// region the cursor is over (fixed cameras by source + live mobile sensors), or a prompt
/// to hover one. Moved here from the old EXPLORE checkbox now that density is its own mode.
fn result_neighborhoods(
    ui: &mut egui::Ui,
    sim: Option<&Sim>,
    pick: &crate::NeighborhoodPick,
    nbhd_live: &crate::NeighborhoodLive,
) {
    let hovered = sim.and_then(|s| pick.0.and_then(|i| s.neighborhoods.get(i).map(|n| (i, n))));
    let Some((i, n)) = hovered else {
        ui.label(
            egui::RichText::new("Hover a neighborhood on the map for its breakdown.")
                .strong(),
        );
        return;
    };
    ui.label(
        egui::RichText::new(format!("{} · {}", n.name, n.borough))
            .font(theme::display(20.0))
            .color(TERRACOTTA),
    );
    ui.label(format!("{} fixed cameras  ·  {:.0} per km²", n.total, n.density));
    egui::Grid::new("nbhd_breakdown")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("CCTV (census)");
            ui.label(format!("{}", n.cctv));
            ui.end_row();
            ui.label("DOT");
            ui.label(format!("{}", n.dot));
            ui.end_row();
            ui.label("ALPR");
            ui.label(format!("{}", n.alpr));
            ui.end_row();
        });
    // Mobile sensors inside this neighborhood right now (sampled live from the moving
    // agents; updates as the clock runs).
    let m = nbhd_live.get(i);
    if m.total() > 0 {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("+ {} mobile sensors here now", m.total()))
                .color(pal::YELLOW)
                .strong(),
        );
        egui::Grid::new("nbhd_mobile").num_columns(2).striped(true).show(ui, |ui| {
            let row = |ui: &mut egui::Ui, label: &str, v: u32| {
                if v > 0 {
                    ui.label(label);
                    ui.label(format!("{}", v));
                    ui.end_row();
                }
            };
            row(ui, "rideshare dashcams", m.rideshare);
            row(ui, "ACE buses", m.bus);
            row(ui, "Tesla cameras", m.tesla);
            row(ui, "delivery robots", m.robot);
            row(ui, "smart glasses", m.glasses);
        });
        ui.label(egui::RichText::new("updates as the day clock runs").weak().size(11.0));
    }
}

/// The A→B route result inside the hero card (research estimate vs. live walk).
fn result_route(
    ui: &mut egui::Ui,
    params: &mut Params,
    route: &RouteState,
    walk_live: &WalkLive,
    time_of_day: f64,
) {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut params.exposure_mode, ExposureMode::Analytical, "Research estimate");
        ui.selectable_value(&mut params.exposure_mode, ExposureMode::Narrative, "Live walk");
    });
    ui.add_space(6.0);

    let Some(s) = &route.summary else {
        ui.label(
            egui::RichText::new(if route.status.is_empty() {
                "Click the map to set a start point."
            } else {
                &route.status
            })
            .strong(),
        );
        return;
    };

    ui.label(
        egui::RichText::new(format!(
            "walk: {:.0} m  (~{:.0} min) · departing {:02}:{:02}",
            s.route_len_m,
            s.duration_s / 60.0,
            time_of_day.floor() as i32,
            (time_of_day.fract() * 60.0).floor() as i32,
        ))
        .weak()
        .size(12.0),
    );

    match params.exposure_mode {
        ExposureMode::Analytical => {
            ui.label(
                egui::RichText::new(format!("~{} devices", s.headline_devices))
                    .font(theme::display(30.0))
                    .color(TERRACOTTA),
            );
            ui.label(egui::RichText::new("could have captured you").size(15.0).color(pal::ZINC_300));
            ui.add_space(6.0);
            egui::Grid::new("breakdown").num_columns(4).striped(true).show(ui, |ui| {
                ui.label(egui::RichText::new("source").strong());
                ui.label(egui::RichText::new("tier").strong());
                ui.label(egui::RichText::new("devices").strong());
                ui.label(egui::RichText::new("P(seen)").strong());
                ui.end_row();
                for b in &s.breakdown {
                    let (tname, color) = tier_style(b.tier);
                    ui.label(b.kind.label());
                    ui.colored_label(color, tname);
                    ui.label(format!("~{:.1}", b.devices));
                    ui.label(format!("{:.0}%", b.p_at_least_one * 100.0));
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!(
                    "{:.0} expected capture-events · {:.1}% of walk under fixed CCTV",
                    s.total_expected_frames,
                    s.fraction_surveilled * 100.0
                ))
                .weak()
                .size(11.0),
            );
        }
        ExposureMode::Narrative => {
            let live = walk_live.count
                + walk_live.mobile_vehicle
                + walk_live.mobile_glasses
                + walk_live.mobile_bus
                + walk_live.mobile_robot
                + walk_live.mobile_tesla;
            ui.label(
                egui::RichText::new(format!("~{live} devices"))
                    .font(theme::display(30.0))
                    .color(TERRACOTTA),
            );
            ui.label(egui::RichText::new("saw you this walk").size(15.0).color(pal::ZINC_300));
            ui.add_space(6.0);
            let robot_violet = egui::Color32::from_rgb(0x9a, 0x8f, 0xc0);
            egui::Grid::new("live_breakdown").num_columns(2).striped(true).show(ui, |ui| {
                ui.label("fixed cameras");
                ui.colored_label(pal::ZINC_400, format!("{}", walk_live.count));
                ui.end_row();
                ui.label("rideshare dashcams");
                ui.colored_label(pal::AMBER, format!("{}", walk_live.mobile_vehicle));
                ui.end_row();
                ui.label("smart glasses");
                ui.colored_label(pal::ZINC_300, format!("{}", walk_live.mobile_glasses));
                ui.end_row();
                ui.label("ACE buses");
                ui.colored_label(pal::STEEL, format!("{}", walk_live.mobile_bus));
                ui.end_row();
                if walk_live.mobile_robot > 0 {
                    ui.label("delivery robots");
                    ui.colored_label(robot_violet, format!("{}", walk_live.mobile_robot));
                    ui.end_row();
                }
                if walk_live.mobile_tesla > 0 {
                    ui.label("Tesla cameras");
                    ui.colored_label(pal::ORANGE, format!("{}", walk_live.mobile_tesla));
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "A single stochastic walk, one Monte-Carlo sample of the model. \
                     Switch to Research estimate for the reproducible figure.",
                )
                .weak()
                .size(11.0),
            );
        }
    }
}

/// The "About the data & its limits" body (collapsed by default in the panel).
fn about_data(ui: &mut egui::Ui) {
    // One source entry: a bold name + a weaker, wrapped description underneath.
    let src = |ui: &mut egui::Ui, name: &str, desc: &str| {
        ui.label(egui::RichText::new(name).strong().size(12.0).color(pal::ZINC_100));
        ui.label(egui::RichText::new(desc).size(11.5).color(pal::ZINC_400));
        ui.add_space(5.0);
    };

    section(ui, "WHERE THE DATA COMES FROM");
    src(ui, "Streets", "OpenStreetMap via Overpass (ODbL).");
    src(
        ui,
        "Fixed CCTV",
        "Amnesty Decode Surveillance NYC crowdsourced census (CC BY-NC-ND 4.0) + Dahir et al. \
         2025 detections (CC BY 4.0), aggregated & de-duplicated.",
    );
    src(
        ui,
        "DOT traffic cams",
        "NYC DOT (nyctmc.org). Locations only; the camera images themselves are not used.",
    );
    src(
        ui,
        "ALPR plate readers",
        "DeFlock crowdsourced license-plate readers via OSM (owl icon).",
    );
    src(
        ui,
        "Enforcement cams (ENF)",
        "Speed / bus-lane / red-light cameras from NYC DOT 'PHOTO ENFORCED' signs.",
    );
    src(
        ui,
        "ACE buses",
        "Real MTA GTFS timetable for the simulation day (data.ny.gov ACE route list).",
    );
    src(
        ui,
        "Rideshare cams",
        "Real NYC TLC trip records for the day, replayed at their actual pickup times.",
    );

    ui.add_space(2.0);
    section(ui, "HOW SOURCES ARE COMBINED");
    ui.label(
        egui::RichText::new(
            "Fixed cameras within ~15 m across sources merge into one physical node. A camera \
             the CCTV census, DOT, and an enforcement sign all record counts once in the \
             headline. The per-source rows still show each source's overlapping attestations.",
        )
        .size(11.5)
        .color(pal::ZINC_400),
    );

    ui.add_space(6.0);
    section(ui, "WHAT'S REAL, WHAT'S MODELED");
    ui.label(
        egui::RichText::new(
            "Real: how the estimate varies through the day. The ACE term follows the day's actual \
             MTA timetable; the rideshare term its real per-minute TLC trip volume.",
        )
        .size(11.5)
        .color(pal::ZINC_400),
    );
    ui.add_space(3.0);
    ui.label(
        egui::RichText::new(
            "Modeled: per-device rates (cameras-per-vehicle, bus-camera reach) are assumptions; \
             adjust the sliders. Smart glasses (D) are fully speculative. Fixed-camera points are \
             street-view detections, not surveyed devices.",
        )
        .size(11.5)
        .color(pal::ZINC_400),
    );

    ui.add_space(6.0);
    ui.label(
        egui::RichText::new("A model estimate, not a surveillance map. Your route stays on this machine.")
            .italics()
            .size(11.5)
            .color(pal::ZINC_300),
    );
}

/// A day↔night sky color for the time bar at `hour` (cold night → warm dawn →
/// bright midday → warm dusk → cold night), interpolated between keypoints.
fn sky_color(hour: f64) -> egui::Color32 {
    // Re-keyed for the dark theme: deep night → amber dawn → a muted steel "day"
    // (so midday isn't a bright patch on the near-black bar) → orange dusk → night.
    const KEYS: [(f64, [u8; 3]); 7] = [
        (0.0, [0x0f, 0x14, 0x20]),
        (5.0, [0x0f, 0x14, 0x20]),
        (6.5, [0xca, 0xa2, 0x3a]),
        (12.0, [0x5b, 0x6b, 0x86]),
        (17.5, [0xc2, 0x74, 0x1f]),
        (19.5, [0x0f, 0x14, 0x20]),
        (24.0, [0x0f, 0x14, 0x20]),
    ];
    let h = hour.rem_euclid(24.0);
    let mut i = 0;
    while i + 1 < KEYS.len() && h > KEYS[i + 1].0 {
        i += 1;
    }
    let (h0, c0) = KEYS[i];
    let (h1, c1) = KEYS[(i + 1).min(KEYS.len() - 1)];
    let f = if h1 > h0 { ((h - h0) / (h1 - h0)) as f32 } else { 0.0 };
    let ch = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round() as u8;
    egui::Color32::from_rgb(ch(c0[0], c1[0]), ch(c0[1], c1[1]), ch(c0[2], c1[2]))
}

/// Paint the stylized day bar: a day↔night gradient, hour ticks/labels, and a
/// sun (day) / moon (night) marker gliding to the current time.
fn paint_time_bar(painter: &egui::Painter, rect: egui::Rect, time_of_day: f64) {
    let ink = pal::ZINC_300;
    let rule = pal::ZINC_700;
    // Gradient: vertical slices colored by their hour.
    let slices = 64;
    for i in 0..slices {
        let t0 = i as f32 / slices as f32;
        let t1 = (i + 1) as f32 / slices as f32;
        let hour = ((t0 + t1) * 0.5 * 24.0) as f64;
        let x0 = rect.left() + t0 * rect.width();
        let x1 = rect.left() + t1 * rect.width();
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom())),
            0.0,
            sky_color(hour),
        );
    }
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, rule),
        egui::StrokeKind::Inside,
    );
    // Hour ticks + labels (00 / 06 / 12 / 18 / 24).
    for h in [0, 6, 12, 18, 24] {
        let x = rect.left() + (h as f32 / 24.0) * rect.width();
        painter.line_segment(
            [egui::pos2(x, rect.bottom() - 5.0), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, ink),
        );
        painter.text(
            egui::pos2(x, rect.bottom() - 6.0),
            egui::Align2::CENTER_BOTTOM,
            format!("{h:02}"),
            egui::FontId::proportional(9.0),
            ink,
        );
    }
    // Sun / moon marker gliding to the current time.
    let mx = rect.left() + (time_of_day as f32 / 24.0) * rect.width();
    let my = rect.center().y;
    painter.line_segment(
        [egui::pos2(mx, rect.top()), egui::pos2(mx, rect.bottom())],
        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0xf4, 0xf4, 0xf5, 90)),
    );
    let day = (6.0..18.0).contains(&time_of_day);
    let (fill, halo) = if day {
        (
            pal::YELLOW,      // sun: hazard gold
            pal::AMBER,       // amber halo
        )
    } else {
        (
            pal::ZINC_300,    // moon: pale zinc
            pal::STEEL,       // steel halo
        )
    };
    painter.circle_filled(egui::pos2(mx, my), 8.0, fill);
    painter.circle_stroke(egui::pos2(mx, my), 8.0, egui::Stroke::new(1.5, halo));
    if !day {
        // Carve a crescent by overlaying a sky-colored disc.
        painter.circle_filled(egui::pos2(mx + 3.5, my - 1.0), 6.5, sky_color(time_of_day));
    }
}

/// The StoryMap UI: a "▶ Tutorial" launcher (bottom-left) when idle, and a floating
/// caption bar with transport controls (bottom-center, clear of the right panel) while
/// a tour plays. A separate egui pass from `ui_panel` so it owns no shared params.
pub fn storymap_ui(mut contexts: EguiContexts, mut story: ResMut<crate::storymap::StoryMap>) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    if !story.active {
        egui::Area::new(egui::Id::new("storymap-launch"))
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(16.0, -16.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let tut = ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("▶  Tutorial")
                                    .size(13.0)
                                    .strong()
                                    .color(pal::ZINC_950),
                            )
                            .fill(pal::ORANGE),
                        )
                        .on_hover_text("Play a guided tour of the map");
                    if tut.clicked() {
                        story.start("Tutorial", crate::storymap::tutorial());
                    }
                    let longi = ui
                        .add(egui::Button::new(
                            egui::RichText::new("🕰  10 years").size(13.0).strong(),
                        ))
                        .on_hover_text(
                            "A longitudinal story: sparse 2015 → saturated today → the \
                             speculative '+5 years' layer",
                        );
                    if longi.clicked() {
                        story.start("A decade of watching", crate::storymap::longitudinal());
                    }
                });
            });
        return;
    }
    // Read the display values out before the mutating transport controls borrow `story`.
    let (idx, n, title) = (story.idx, story.steps.len(), story.title);
    let caption = story.current().map(|s| s.caption).unwrap_or("");
    let paused = story.paused;
    // Floating, anchored bottom-center but shifted left to clear the right panel.
    egui::Area::new(egui::Id::new("storymap-bar"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(-170.0, -18.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(pal::ZINC_950)
                .stroke(egui::Stroke::new(1.0, pal::ZINC_800))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_max_width(560.0);
                    ui.label(
                        egui::RichText::new(format!("{title}  ·  {} / {n}", idx + 1))
                            .strong()
                            .size(12.0)
                            .color(pal::ORANGE),
                    );
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(caption).size(14.0));
                    ui.add_space(7.0);
                    ui.horizontal(|ui| {
                        if ui.button(if paused { "▶  Play" } else { "⏸  Pause" }).clicked() {
                            story.paused = !paused;
                        }
                        if ui.add_enabled(idx > 0, egui::Button::new("‹  Back")).clicked() {
                            story.prev();
                        }
                        if ui.button("Next  ›").clicked() {
                            story.next();
                        }
                        if ui.button("✕  Exit").clicked() {
                            story.stop();
                        }
                    });
                });
        });
}

/// Shared modal section: when the clicked camera was merged with co-located cameras from
/// *other* layers (so it counts once, not several times, in the headline), name those
/// other sources. No-op for a single-source camera. `also` comes from the pin's
/// `also_sources`, filled in `build_world` from the `group_sensors` clustering.
fn cross_source_note(ui: &mut egui::Ui, also: &[&'static str]) {
    if also.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("CROSS-SOURCE CONFIRMED")
            .font(theme::display(11.0))
            .color(TERRACOTTA),
    );
    ui.label(
        egui::RichText::new(format!(
            "Also mapped here by {}. One physical camera, counted once in the headline.",
            also.join(", ")
        ))
        .size(11.0)
        .color(pal::ZINC_100),
    );
}

/// Per-camera metadata modal for a clicked ALPR — DeFlock's crowdsourced maker/operator
/// plus deep-links to OpenStreetMap and DeFlock. `SelectedAlpr` is set by
/// `handle_click` (clicking on/near a reader); closing the window clears it.
pub fn alpr_modal(
    mut contexts: EguiContexts,
    dir: Res<crate::AlprDirectory>,
    mut sel: ResMut<crate::SelectedAlpr>,
) {
    let Some(i) = sel.0 else { return };
    let Some(pin) = dir.0.get(i) else {
        sel.0 = None;
        return;
    };
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let mut open = true;
    egui::Window::new("ALPR · license-plate reader")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .default_width(284.0)
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(16.0, 92.0))
        .show(ctx, |ui| {
            // Maker headline (the stratify-by-operator key).
            let maker = pin.manufacturer.as_deref().unwrap_or("Unknown maker");
            ui.label(egui::RichText::new(maker).strong().size(15.0).color(pal::RED));
            if let Some(op) = &pin.operator {
                ui.label(egui::RichText::new(format!("Operator · {op}")).size(12.0));
            }
            ui.label(
                egui::RichText::new(match pin.heading_deg {
                    Some(h) => format!("Faces ~{h:.0}° (compass bearing)"),
                    None => "Direction not mapped".to_owned(),
                })
                .size(12.0)
                .color(pal::ZINC_400),
            );
            cross_source_note(ui, &pin.also_sources);
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(2.0);
            ui.hyperlink_to(
                "Open in OpenStreetMap ↗",
                format!("https://www.openstreetmap.org/node/{}", pin.osm_id),
            );
            ui.hyperlink_to(
                "Open in DeFlock ↗",
                format!(
                    "https://maps.deflock.org/?lat={:.5}&lng={:.5}&zoom=18",
                    pin.lat, pin.lon
                ),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Crowdsourced via DeFlock → OpenStreetMap (ODbL).")
                    .size(9.5)
                    .italics()
                    .weak(),
            );
        });
    if !open {
        sel.0 = None;
    }
}

/// Per-camera provenance modal for a clicked fixed CCTV — its census source (Amnesty
/// crowdsource vs Dahir ML detection) plus a Google Street View deep-link (the exact
/// Dahir panorama where it was detected, or the location otherwise). `SelectedCctv` is
/// set by `handle_click`; closing the window clears it.
pub fn cctv_modal(
    mut contexts: EguiContexts,
    dir: Res<crate::CctvDirectory>,
    mut sel: ResMut<crate::SelectedCctv>,
) {
    use sim_core::assets::CctvSource;
    let Some(i) = sel.0 else { return };
    let Some(pin) = dir.0.get(i) else {
        sel.0 = None;
        return;
    };
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let mut open = true;
    egui::Window::new("CCTV · fixed camera")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .default_width(284.0)
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(16.0, 92.0))
        .show(ctx, |ui| {
            let (title, gloss) = match pin.source {
                CctvSource::Amnesty => (
                    "Amnesty · Decode Surveillance NYC",
                    "Crowdsourced street-view camera census (count at this intersection).",
                ),
                CctvSource::Dahir => (
                    "Dahir et al. · ML street-view detection",
                    "Detected automatically in a Google Street View panorama.",
                ),
            };
            ui.label(egui::RichText::new(title).strong().size(14.0).color(pal::MAROON));
            ui.label(egui::RichText::new(gloss).size(11.0));
            if let (Some(y), Some(m)) = (pin.year, pin.month) {
                ui.label(
                    egui::RichText::new(format!("Panorama captured {y}-{m:02}"))
                        .size(11.5)
                        .color(pal::ZINC_400),
                );
            }
            if let Some(h) = pin.heading_deg {
                ui.label(
                    egui::RichText::new(format!("Capture bearing ~{h:.0}°"))
                        .size(11.5)
                        .color(pal::ZINC_400),
                );
            }
            cross_source_note(ui, &pin.also_sources);
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(2.0);
            // Dahir points link to the exact panorama; otherwise drop into Street View
            // at the camera's coordinates.
            let sv = match &pin.panoid {
                Some(p) => format!("https://www.google.com/maps/@?api=1&map_action=pano&pano={p}"),
                None => format!(
                    "https://www.google.com/maps/@?api=1&map_action=pano&viewpoint={:.6},{:.6}",
                    pin.lat, pin.lon
                ),
            };
            ui.hyperlink_to("Open in Google Street View ↗", sv);
            ui.label(
                egui::RichText::new("Sample-point estimate, not a surveyed device coordinate.")
                    .size(9.5)
                    .italics()
                    .weak(),
            );
        });
    if !open {
        sel.0 = None;
    }
}
