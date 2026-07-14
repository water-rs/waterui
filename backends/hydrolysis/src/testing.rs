//! Deterministic theme installation for Hydrolysis-backed tests.

use waterui::{
    Environment, Plugin,
    color::{ResolvedColor, Srgb},
    text::font::{FontWeight, ResolvedFont},
    theme::{ColorScheme, ColorSettings, FontSettings, Theme},
};

fn color(rgb: u32) -> ResolvedColor {
    ResolvedColor::from_srgb(Srgb::from_u32(rgb))
}

/// Installs every theme token required by Hydrolysis rendering.
pub fn install_theme(env: &mut Environment) {
    Theme::new()
        .color_scheme(ColorScheme::Light)
        .colors(
            ColorSettings::new()
                .background(color(0xFF_FF_FF))
                .surface(color(0xFF_FF_FF))
                .surface_variant(color(0xF3_F4_F6))
                .border(color(0xD1_D5_DB))
                .foreground(color(0x11_18_27))
                .muted_foreground(color(0x4B_55_63))
                .accent(color(0x25_63_EB))
                .accent_container(color(0xDB_EA_FE))
                .accent_foreground(color(0xFF_FF_FF))
                .tertiary(color(0x7C_3A_ED))
                .tertiary_container(color(0xED_E9_FE)),
        )
        .fonts(
            FontSettings::new()
                .body(ResolvedFont::new(16.0, FontWeight::Normal))
                .title(ResolvedFont::new(22.0, FontWeight::Normal))
                .headline(ResolvedFont::new(24.0, FontWeight::Normal))
                .subheadline(ResolvedFont::new(16.0, FontWeight::Medium))
                .caption(ResolvedFont::new(12.0, FontWeight::Normal))
                .footnote(ResolvedFont::new(11.0, FontWeight::Medium)),
        )
        .install(env);
}
