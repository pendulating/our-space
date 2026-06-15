//! The egui control/results panel (native dev UI; the public web build will use
//! a DOM overlay per the plan).

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use sim_core::ConfidenceTier;

use crate::{EguiWantsPointer, Params, ResetRequested, RouteState};

/// Short label + accent color for a confidence tier.
fn tier_style(tier: ConfidenceTier) -> (&'static str, egui::Color32) {
    match tier {
        ConfidenceTier::A => ("A · mapped", egui::Color32::from_rgb(90, 210, 130)),
        ConfidenceTier::B => ("B · estimated", egui::Color32::from_rgb(230, 200, 80)),
        ConfidenceTier::C => ("C · modeled", egui::Color32::from_rgb(240, 150, 70)),
        ConfidenceTier::D => ("D · speculative", egui::Color32::from_rgb(210, 110, 110)),
    }
}

pub fn ui_panel(
    mut contexts: EguiContexts,
    route: Res<RouteState>,
    mut params: ResMut<Params>,
    mut reset: ResMut<ResetRequested>,
    mut wants: ResMut<EguiWantsPointer>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    egui::SidePanel::right("panel")
        .default_width(350.0)
        .show(ctx, |ui| {
            ui.heading("our-space");
            ui.label(egui::RichText::new("NYC sensing exposure · Manhattan").italics());
            ui.separator();
            ui.label("Left-click: start (A), then destination (B).");
            ui.label("Right-drag: pan   ·   Scroll: zoom.");
            ui.separator();

            // ---- results ----
            if let Some(s) = &route.summary {
                ui.heading(
                    egui::RichText::new(format!("~{} devices could have captured you", s.headline_devices))
                        .color(egui::Color32::from_rgb(255, 170, 90))
                        .size(20.0),
                );
                ui.label(format!(
                    "walk: {:.0} m  (~{:.0} min)  ·  departing {:02}:00",
                    s.route_len_m,
                    s.duration_s / 60.0,
                    params.departure_hour as i32,
                ));
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
                ui.checkbox(&mut params.dashcam_on, "Dashcams");
                ui.add_enabled(
                    params.dashcam_on,
                    egui::Slider::new(&mut params.dashcam_penetration, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut params.glasses_on, "Smart glasses");
                ui.add_enabled(
                    params.glasses_on,
                    egui::Slider::new(&mut params.glasses_per_1000, 0.0..=50.0)
                        .text("/1k peds"),
                );
            });

            ui.separator();
            ui.checkbox(&mut params.show_ace, "Show ACE corridors (teal)");
            ui.checkbox(&mut params.show_fov, "Show camera fields of view");
            if ui.button("Reset route").clicked() {
                reset.0 = true;
            }

            ui.separator();
            ui.collapsing("About the data & its limits", |ui| {
                ui.label("Streets: OpenStreetMap via Overpass (ODbL).");
                ui.label("Fixed CCTV: Dahir et al. 2025, Stanford (CC BY 4.0; recall ~0.63).");
                ui.label("ACE corridors: MTA GTFS + data.ny.gov ACE route list.");
                ui.add_space(4.0);
                ui.label(
                    "Dashcams (C) and smart glasses (D) are SCENARIO estimates, not \
                     measurements — adjust the sliders. Fixed-camera points are street-view \
                     detections, not surveyed devices.",
                );
                ui.label("A model estimate, not a surveillance map. Your route stays on this machine.");
            });
        });

    wants.0 = ctx.wants_pointer_input() || ctx.is_pointer_over_area();
}
