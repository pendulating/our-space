//! The single source of truth for the app's color palette + display fonts.
//!
//! **Paper theme.** A bright *white-paper* ground with dark "ink" linework. Two
//! semantic families carry the data layers, tuned for contrast on white paper:
//! **warm = surveillance** (a red→orange ramp — maroon CCTV, red plate-readers,
//! orange enforcement, amber DOT) and **cool blue = transit/infrastructure** (ACE
//! bus corridors, LinkNYC kiosks). The hazard-yellow/amber accents stay for UI
//! chrome (the headline number, confidence tiers, the heat ramp).
//!
//! The neutral `ZINC_*` ramp is **inverted from the Tailwind zinc scale** so the same
//! token usages that meant *dark ground / light text* under the old noalprs dark theme
//! now mean *white ground / dark ink*: here `ZINC_950` is the **white ground** and
//! `ZINC_100` is the **darkest ink** (i.e. each `ZINC_N` holds Tailwind zinc-(1000−N)).
//! Keep that in mind — a higher number is *lighter*, not darker. The warm accents
//! (`YELLOW`/`AMBER`/`ORANGE`/`STEEL`) are unchanged.
//!
//! Two parallel sets of the same tokens are exported because the app paints in two
//! color spaces: [`ui`] holds `egui::Color32` for the panel chrome, and [`map`]
//! holds Bevy `Color` for the mesh/material world layers. Keep them in lockstep.

use bevy::color::Srgba;
use bevy::prelude::Color;
use bevy_egui::egui;

/// egui chrome tokens (panels, cards, text, accents).
#[allow(dead_code)] // a palette holds the full token set even if not all are used yet
pub mod ui {
    use bevy_egui::egui::Color32;
    const fn c(r: u8, g: u8, b: u8) -> Color32 {
        Color32::from_rgb(r, g, b)
    }
    // Zinc neutrals — INVERTED ramp (paper): ZINC_950 = white ground … ZINC_100 = ink.
    pub const ZINC_950: Color32 = c(0xff, 0xff, 0xff); // white paper ground
    pub const ZINC_900: Color32 = c(0xf4, 0xf4, 0xf5);
    pub const ZINC_800: Color32 = c(0xe4, 0xe4, 0xe7);
    pub const ZINC_700: Color32 = c(0xd4, 0xd4, 0xd8);
    pub const ZINC_600: Color32 = c(0xa1, 0xa1, 0xaa);
    pub const ZINC_500: Color32 = c(0x71, 0x71, 0x7a);
    pub const ZINC_400: Color32 = c(0x52, 0x52, 0x5b);
    pub const ZINC_300: Color32 = c(0x3f, 0x3f, 0x46);
    pub const ZINC_100: Color32 = c(0x18, 0x18, 0x1b); // darkest ink
    // Warning ramp (signature → hot).
    pub const YELLOW: Color32 = c(0xfa, 0xcc, 0x15);
    pub const YELLOW_500: Color32 = c(0xea, 0xb3, 0x08);
    pub const AMBER: Color32 = c(0xf5, 0x9e, 0x0b);
    pub const ORANGE: Color32 = c(0xf9, 0x73, 0x16);
    // Surveillance ramp (warm, high-contrast on white): maroon CCTV → red plate
    // readers → orange enforcement → amber DOT. Distinct in hue *and* value so the
    // dense fixed-camera layers stay legible against the paper ground.
    pub const MAROON: Color32 = c(0x7f, 0x1d, 0x1d); // CCTV — the dense baseline, recedes by value
    pub const RED: Color32 = c(0xdc, 0x26, 0x26); // ALPR / Flock — the headline threat
    pub const ORANGE_600: Color32 = c(0xea, 0x58, 0x0c); // photo-enforcement
    pub const AMBER_700: Color32 = c(0xb4, 0x53, 0x09); // DOT traffic cams
    // Cool support (transit / infrastructure / machine).
    pub const STEEL: Color32 = c(0x7d, 0x97, 0xb8);
    pub const BLUE: Color32 = c(0x25, 0x63, 0xeb); // ACE bus corridors (transit)
    pub const SKY: Color32 = c(0x02, 0x84, 0xc7); // LinkNYC kiosks (infrastructure)
}

/// Bevy world tokens (streets, markers, agents, heatmap, choropleth).
#[allow(dead_code)] // a palette holds the full token set even if not all are used yet
pub mod map {
    use super::*;
    const fn c(r: u8, g: u8, b: u8) -> Color {
        Color::Srgba(Srgba {
            red: r as f32 / 255.0,
            green: g as f32 / 255.0,
            blue: b as f32 / 255.0,
            alpha: 1.0,
        })
    }
    /// Translucent variant for cones / rings.
    pub const fn ca(r: u8, g: u8, b: u8, alpha: f32) -> Color {
        Color::Srgba(Srgba {
            red: r as f32 / 255.0,
            green: g as f32 / 255.0,
            blue: b as f32 / 255.0,
            alpha,
        })
    }
    // Zinc neutrals — INVERTED ramp (paper): ZINC_950 = white ground … ZINC_300 = ink.
    pub const ZINC_950: Color = c(0xff, 0xff, 0xff); // white paper ground
    pub const ZINC_900: Color = c(0xf4, 0xf4, 0xf5);
    pub const ZINC_800: Color = c(0xe4, 0xe4, 0xe7);
    pub const ZINC_700: Color = c(0xd4, 0xd4, 0xd8);
    pub const ZINC_600: Color = c(0xa1, 0xa1, 0xaa);
    pub const ZINC_500: Color = c(0x71, 0x71, 0x7a);
    pub const ZINC_400: Color = c(0x52, 0x52, 0x5b);
    pub const ZINC_300: Color = c(0x3f, 0x3f, 0x46); // darkest ink (map)
    pub const YELLOW: Color = c(0xfa, 0xcc, 0x15);
    pub const YELLOW_500: Color = c(0xea, 0xb3, 0x08);
    pub const AMBER: Color = c(0xf5, 0x9e, 0x0b);
    pub const ORANGE: Color = c(0xf9, 0x73, 0x16);
    // Surveillance ramp (warm, high-contrast on white) — see the `ui` mirror above.
    pub const MAROON: Color = c(0x7f, 0x1d, 0x1d); // CCTV — dense baseline
    pub const RED: Color = c(0xdc, 0x26, 0x26); // ALPR / Flock — headline threat
    pub const ORANGE_600: Color = c(0xea, 0x58, 0x0c); // photo-enforcement
    pub const AMBER_700: Color = c(0xb4, 0x53, 0x09); // DOT traffic cams
    pub const STEEL: Color = c(0x7d, 0x97, 0xb8);
    pub const STEEL_DEEP: Color = c(0x5e, 0x7e, 0xa8);
    pub const BLUE: Color = c(0x25, 0x63, 0xeb); // ACE bus corridors (transit)
    pub const SKY: Color = c(0x02, 0x84, 0xc7); // LinkNYC kiosks (infrastructure)
    /// Delivery robots — the one off-palette outlier, marking the speculative tier.
    pub const ROBOT_VIOLET: Color = c(0x9a, 0x8f, 0xc0);
    /// Tesla operator/agent — amber-600, a deeper warning than rideshare amber.
    pub const TESLA_AMBER: Color = c(0xd9, 0x77, 0x06);
}

/// The egui font family used for poster-weight headlines + the big "~N" number.
pub const DISPLAY: &str = "display";

/// Install Host Grotesk as egui's body face (Regular) and a separate `display`
/// family (ExtraBold) — matching the web shell's `host-grotesk` Adobe Fonts set, so
/// the in-canvas UI and the page chrome share one typographic voice. Call once,
/// after the visuals are set.
pub fn install_fonts(ctx: &egui::Context) {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "host_grotesk".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/HostGrotesk-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "host_grotesk_bold".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/HostGrotesk-ExtraBold.ttf"
        ))),
    );
    // Body: Host Grotesk Regular first in the proportional family (egui defaults
    // remain as fallback for any missing glyph).
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "host_grotesk".to_owned());
    // Display: Host Grotesk ExtraBold, falling back to the regular cut for any
    // missing glyph.
    fonts.families.insert(
        egui::FontFamily::Name(DISPLAY.into()),
        vec!["host_grotesk_bold".to_owned(), "host_grotesk".to_owned()],
    );
    ctx.set_fonts(fonts);
}

/// A `FontId` in the poster `display` family at `size`.
pub fn display(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(DISPLAY.into()))
}
