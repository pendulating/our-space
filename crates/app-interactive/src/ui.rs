//! The egui control/results panel (native dev UI; the public web build will use
//! a DOM overlay per the plan).

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use sim_core::ConfidenceTier;

use crate::{
    EguiWants, ExposureMode, HeatClass, Mode, Params, ResetRequested, RouteState, Sim, WalkLive,
    WalkshedState,
};

/// egui legend colors mirroring the map's heat gradient (warm low -> cold high).
const LEGEND: [egui::Color32; 6] = [
    egui::Color32::from_rgb(0xdc, 0xcc, 0xa4),
    egui::Color32::from_rgb(0xcb, 0xa9, 0x68),
    egui::Color32::from_rgb(0xb8, 0x8a, 0x3e),
    egui::Color32::from_rgb(0x9c, 0x7c, 0x6e),
    egui::Color32::from_rgb(0x5e, 0x6f, 0x8c),
    egui::Color32::from_rgb(0x2c, 0x47, 0x63),
];

/// Short label + ink color for a confidence tier. Note tier D shares the cold
/// surveillance slate: "speculative = the machine".
fn tier_style(tier: ConfidenceTier) -> (&'static str, egui::Color32) {
    match tier {
        ConfidenceTier::A => ("A · mapped", egui::Color32::from_rgb(0x4e, 0x66, 0x38)),
        ConfidenceTier::B => ("B · estimated", egui::Color32::from_rgb(0x7a, 0x5d, 0x18)),
        ConfidenceTier::C => ("C · modeled", egui::Color32::from_rgb(0xa8, 0x50, 0x1f)),
        ConfidenceTier::D => ("D · speculative", egui::Color32::from_rgb(0x34, 0x51, 0x69)),
    }
}

/// Apply the warm "field-journal" egui theme (light base, parchment-tinted).
fn apply_theme(ctx: &egui::Context) {
    let rgb = egui::Color32::from_rgb;
    let mut v = egui::Visuals::light();
    v.panel_fill = rgb(0xe9, 0xdc, 0xc4);
    v.window_fill = rgb(0xef, 0xe6, 0xd2);
    v.faint_bg_color = rgb(0xdd, 0xcd, 0xaf);
    v.extreme_bg_color = rgb(0xdd, 0xcd, 0xaf);
    v.override_text_color = Some(rgb(0x3a, 0x2e, 0x21));
    v.hyperlink_color = rgb(0x8c, 0x3f, 0x12);
    v.window_stroke = egui::Stroke::new(1.0, rgb(0xc2, 0xb2, 0x91));
    let sep = egui::Stroke::new(1.0, rgb(0xc2, 0xb2, 0x91));
    v.widgets.noninteractive.bg_fill = rgb(0xe9, 0xdc, 0xc4);
    v.widgets.noninteractive.bg_stroke = sep;
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, rgb(0x3a, 0x2e, 0x21));
    v.widgets.inactive.bg_fill = rgb(0xdc, 0xcb, 0xa9);
    v.widgets.inactive.weak_bg_fill = rgb(0xdc, 0xcb, 0xa9);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, rgb(0x3a, 0x2e, 0x21));
    v.widgets.hovered.bg_fill = rgb(0xcb, 0xb7, 0x8d);
    v.widgets.hovered.weak_bg_fill = rgb(0xcb, 0xb7, 0x8d);
    v.widgets.active.bg_fill = rgb(0xa8, 0x54, 0x1f);
    v.widgets.active.weak_bg_fill = rgb(0xa8, 0x54, 0x1f);
    v.selection.bg_fill = rgb(0xa8, 0x54, 0x1f).gamma_multiply(0.35);
    v.selection.stroke = egui::Stroke::new(1.0, rgb(0x8c, 0x3f, 0x12));
    ctx.set_visuals(v);
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
    mut themed: bevy::prelude::Local<bool>,
) {
    let corr = sim.as_ref().and_then(|s| s.equity_corr);
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    if !*themed {
        apply_theme(ctx);
        *themed = true;
    }

    egui::SidePanel::right("panel")
        .default_width(350.0)
        .show(ctx, |ui| {
            ui.heading("our-space");
            ui.label(egui::RichText::new("NYC sensing exposure · Manhattan").italics());
            ui.separator();

            // ---- mode ----
            ui.horizontal(|ui| {
                ui.selectable_value(&mut params.mode, Mode::Route, "Walk A→B");
                ui.selectable_value(&mut params.mode, Mode::Walkshed, "10-min walkshed");
            });
            match params.mode {
                Mode::Route => ui.label("Click to set start (A), then destination (B); watch the walk."),
                Mode::Walkshed => ui.label("Click one point to map everything within a 10-minute walk."),
            };
            ui.label(egui::RichText::new("Drag: pan · Scroll: zoom · WASD/arrows: pan.").weak());
            ui.separator();

            // ---- results: walkshed mode ----
            if params.mode == Mode::Walkshed {
                if let Some(w) = &walkshed.summary {
                    ui.heading(
                        egui::RichText::new(format!("~{} cameras watch your area", w.cameras_corrected.round() as u32))
                            .color(egui::Color32::from_rgb(0x9a, 0x4a, 0x17))
                            .size(22.0),
                    );
                    ui.label(format!(
                        "within a {:.0}-minute walk ({} street segments reachable)",
                        w.max_minutes, w.reachable_edges
                    ));
                    ui.label(
                        egui::RichText::new(
                            if (w.cameras_corrected - w.cameras_raw as f64).abs() > 0.5 {
                                format!("{} detected (recall-corrected)", w.cameras_raw)
                            } else {
                                format!("{} mapped (crowdsourced census + ML detections)", w.cameras_raw)
                            },
                        )
                        .weak(),
                    );
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
                ui.separator();
            }

            // ---- results: route mode ----
            if params.mode == Mode::Route {
            // Exposure-mode selector: the reproducible estimate vs the live walk.
            ui.horizontal(|ui| {
                ui.selectable_value(&mut params.exposure_mode, ExposureMode::Analytical, "Research estimate");
                ui.selectable_value(&mut params.exposure_mode, ExposureMode::Narrative, "Live walk");
            });
            ui.add_space(4.0);

            if let Some(s) = &route.summary {
                ui.label(format!(
                    "walk: {:.0} m  (~{:.0} min)  ·  departing {:02}:00",
                    s.route_len_m,
                    s.duration_s / 60.0,
                    params.departure_hour as i32,
                ));

                match params.exposure_mode {
                    ExposureMode::Analytical => {
                        ui.heading(
                            egui::RichText::new(format!("~{} devices could have captured you", s.headline_devices))
                                .color(egui::Color32::from_rgb(0x9a, 0x4a, 0x17))
                                .size(22.0),
                        );
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
                            .weak(),
                        );
                    }
                    ExposureMode::Narrative => {
                        let live = walk_live.count + walk_live.mobile_vehicle + walk_live.mobile_glasses;
                        ui.heading(
                            egui::RichText::new(format!("~{live} devices saw you this walk"))
                                .color(egui::Color32::from_rgb(0x9a, 0x4a, 0x17))
                                .size(22.0),
                        );
                        ui.add_space(6.0);
                        let slate = egui::Color32::from_rgb(0x34, 0x51, 0x69);
                        egui::Grid::new("live_breakdown").num_columns(2).striped(true).show(ui, |ui| {
                            ui.label("fixed cameras");
                            ui.colored_label(slate, format!("{}", walk_live.count));
                            ui.end_row();
                            ui.label("rideshare dashcams");
                            ui.colored_label(egui::Color32::from_rgb(0xa8, 0x50, 0x1f), format!("{}", walk_live.mobile_vehicle));
                            ui.end_row();
                            ui.label("smart glasses");
                            ui.colored_label(slate, format!("{}", walk_live.mobile_glasses));
                            ui.end_row();
                        });
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(
                                "a single stochastic walk — a Monte-Carlo sample of the model. \
                                 Switch to Research estimate for the reproducible figure.",
                            )
                            .weak()
                            .size(11.0),
                        );
                    }
                }
            } else {
                ui.label(
                    egui::RichText::new(if route.status.is_empty() {
                        "Click the map to set a start point."
                    } else {
                        &route.status
                    })
                    .strong(),
                );
            }

            ui.separator();

            // ---- time + scenario controls ----
            ui.label(egui::RichText::new("Time of day").strong());
            ui.add(
                egui::Slider::new(&mut params.departure_hour, 0.0..=23.0)
                    .step_by(1.0)
                    .custom_formatter(|v, _| format!("{:02}:00", v as i32)),
            );

            ui.add_space(6.0);
            ui.label(egui::RichText::new("Sensing classes").strong());
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
            ui.horizontal(|ui| {
                ui.checkbox(&mut params.glasses_on, "Smart glasses");
                ui.add_enabled(
                    params.glasses_on,
                    egui::Slider::new(&mut params.glasses_per_1000, 0.0..=50.0)
                        .text("/1k peds"),
                );
            });
            ui.add_space(4.0);
            ui.checkbox(&mut params.show_agents, "Animate moving dashcams & glasses");
            ui.label(
                egui::RichText::new("clay = rideshare dashcams · slate = glasses wearers; density scales with hour + sliders")
                    .weak()
                    .size(11.0),
            );
            } // end route-mode controls

            ui.separator();
            ui.label(egui::RichText::new("Citywide heatmap").strong());
            ui.checkbox(&mut params.heatmap_on, "Show exposure heatmap");
            if params.heatmap_on {
                egui::ComboBox::from_label("layer")
                    .selected_text(params.heatmap_class.label())
                    .show_ui(ui, |ui| {
                        for c in [
                            HeatClass::Fixed,
                            HeatClass::Ace,
                            HeatClass::Dashcam,
                            HeatClass::Total,
                        ] {
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
                        "expected devices/min of presence, as a field over space — live at the \
                         chosen hour + slider settings",
                    )
                    .weak(),
                );
            }

            ui.separator();
            ui.label(egui::RichText::new("Equity overlay").strong());
            ui.checkbox(&mut params.equity_on, "Show neighborhood diversity");
            if params.equity_on {
                ui.label(
                    egui::RichText::new("Block-group Shannon diversity (dim = homogeneous, bright = diverse)")
                        .weak(),
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
                        "Dahir et al. (2025): cameras are most prevalent in racially diverse \
                         neighborhoods — diversity predicts cameras more than crime does, and \
                         %Black is negatively associated after controls. This is a published \
                         correlation, not causation for any one block.",
                    )
                    .weak(),
                );
            }

            ui.separator();
            ui.checkbox(&mut params.show_ace, "Show ACE corridors (teal)");
            ui.checkbox(&mut params.show_fov, "Show camera fields of view");
            if ui.button("Reset route").clicked() {
                reset.0 = true;
            }

            ui.separator();
            ui.collapsing("About the data & its limits", |ui| {
                ui.label("Streets: OpenStreetMap via Overpass (ODbL).");
                ui.label(
                    "Fixed CCTV: Amnesty Decode Surveillance NYC crowdsourced census \
                     (CC BY-NC-ND 4.0) + Dahir et al. 2025 detections (CC BY 4.0), \
                     aggregated & de-duplicated.",
                );
                ui.label("DOT traffic cams (triangles): NYC DOT (nyctmc.org) — locations only, images not used.");
                ui.label("ACE corridors: MTA GTFS + data.ny.gov ACE route list.");
                ui.label("ALPR readers (squares): DeFlock crowdsourced plate readers via OSM.");
                ui.label("Rideshare cams: density from NYC TLC Uber/Lyft trips, by taxi zone.");
                ui.add_space(4.0);
                ui.label(
                    "Rideshare-camera density is real (TLC trips); the camera-per-vehicle rate \
                     is still an assumption — adjust the slider. Smart glasses (D) are fully \
                     speculative. Fixed-camera points are street-view detections, not surveyed devices.",
                );
                ui.label("A model estimate, not a surveillance map. Your route stays on this machine.");
            });
        });

    wants.pointer = ctx.wants_pointer_input() || ctx.is_pointer_over_area();
    wants.keyboard = ctx.wants_keyboard_input();
}
