//! The egui control/results panel (native dev UI; the public web build will use
//! a DOM overlay per the plan).

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use sim_core::exposure::SourceKind;

use crate::{EguiWantsPointer, Params, ResetRequested, RouteState};

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
        .default_width(330.0)
        .show(ctx, |ui| {
            ui.heading("our-space");
            ui.label(egui::RichText::new("NYC sensing exposure · Manhattan").italics());
            ui.separator();

            ui.label("Left-click: set start (A), then destination (B).");
            ui.label("Right-drag: pan   ·   Scroll: zoom.");
            ui.separator();

            if let Some(s) = &route.summary {
                ui.heading(
                    egui::RichText::new(format!("~{} cameras could have captured you", s.headline_devices))
                        .color(egui::Color32::from_rgb(255, 170, 90))
                        .size(20.0),
                );
                ui.add_space(4.0);
                ui.label(format!("walk: {:.0} m  (~{:.0} min)", s.route_len_m, s.duration_s / 60.0));
                ui.label(format!("expected capture-events: {:.0}", s.total_expected_captures));
                ui.label(format!("route under surveillance: {:.1}%", s.fraction_surveilled * 100.0));
                let raw = s.tally.source(SourceKind::FixedCctv).distinct_devices;
                ui.label(
                    egui::RichText::new(format!(
                        "fixed CCTV detected: {raw} (headline applies recall ~0.63)"
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
            ui.checkbox(&mut params.show_fov, "Show camera fields of view");
            if ui.button("Reset route").clicked() {
                reset.0 = true;
            }

            ui.separator();
            ui.collapsing("About the data & its limits", |ui| {
                ui.label("Streets: OpenStreetMap via Overpass (ODbL).");
                ui.label("Cameras: Dahir et al. 2025, Stanford Digital Repository (CC BY 4.0).");
                ui.add_space(4.0);
                ui.label(
                    "Camera points are street-view detections (recall ~0.63), not surveyed \
                     device locations. This is a model estimate, not a surveillance map.",
                );
                ui.label("Your route never leaves this machine.");
                ui.add_space(4.0);
                ui.label("Coming next: ACE bus cameras, dashcams, smart glasses, and time-of-day.");
            });
        });

    wants.0 = ctx.wants_pointer_input() || ctx.is_pointer_over_area();
}
