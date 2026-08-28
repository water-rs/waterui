//! Deterministic environment setup for Hydrolysis-backed tests.

use waterui::{
    Environment, Plugin,
    color::{ResolvedColor, Srgb},
    text::font::{FontWeight, ResolvedFont},
    theme::{ColorScheme, ColorSettings, FontSettings, Theme},
};
use waterui_core::{AnyView, Native};
use waterui_map::MapConfig;

fn color(rgb: u32) -> ResolvedColor {
    ResolvedColor::from_srgb(Srgb::from_u32(rgb))
}

/// Declares Hydrolysis's own semantic map as the test platform's map bridge.
///
/// `waterui::realization::install` gives way to a backend that already
/// registered a map bridge, and Hydrolysis is such a backend: it renders
/// `Native<MapConfig>` itself, with the accessibility surface the semantic
/// tests assert on. Registering that bridge here says so through the public
/// mechanism — before the harness runs `realization::install` — where a marker
/// type on the config used to say it.
///
/// Without this, a test binary whose feature unification pulls in
/// `waterui/map-gpu` gets the GPU realization installed over Hydrolysis's, and
/// every map test then dies on the `MapGpuOptions` an application, not a
/// harness, is supposed to supply.
fn install_map_bridge(env: &mut Environment) {
    env.insert_hook::<MapConfig, AnyView>(|_env, config| AnyView::new(Native::new(config)));
}

/// Installs every theme token required by Hydrolysis rendering, and the
/// component bridges the backend realizes natively.
pub fn install_theme(env: &mut Environment) {
    install_map_bridge(env);
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
                .tertiary_container(color(0xED_E9_FE))
                .selection_container(color(0x25_63_EB))
                .selection_foreground(color(0xFF_FF_FF)),
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
