//! Theme-token assembly for the runtime environment.
//!
//! [`install_theme_tokens`] installs the runtime's theme tokens in precedence
//! order: framework defaults < [`Style::install_tokens`](crate::Style) < the
//! application's own environment. Every runner calls it — the rendered
//! runners with their style, the semantic runtime with `None` — so the same
//! precedence holds whether or not a style package is present.
//!
//! The framework defaults are *not* a theme: they carry no widget theme and
//! no presentation decisions beyond the framework's baseline palette and
//! type scale. A `Style` may overwrite any of them, and the application
//! overwrites both.

use waterui::{
    Environment, Plugin,
    color::{Srgb, WorkingColor},
    theme::{ColorScheme, ColorSettings, FontSettings, Theme},
};

fn color(rgb: u32) -> WorkingColor {
    Srgb::from_u32(rgb).resolve()
}

/// Assembles the runtime environment's theme tokens in precedence order:
/// framework defaults < `Style::install_tokens` < the application's own
/// environment (water-rs/hydrolysis#203).
///
/// The style installs into an environment that already carries the
/// application's entries layered over the framework defaults, so a style
/// that reads the environment while installing — e.g. Material3's dynamic
/// colours binding `installed_color_scheme` — sees the application's
/// installed values. The application's entries are then layered back on top,
/// so a `Theme` it installed is never replaced.
pub(crate) fn install_theme_tokens(env: &mut Environment, style: Option<&dyn crate::Style>) {
    let mut defaults = Environment::new();
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
                .selection_foreground(color(0xFF_FF_FF))
                .error(color(0xDC_26_26))
                .error_foreground(color(0xFF_FF_FF)),
        )
        .fonts(FontSettings::default_scale())
        .install(&mut defaults);
    let mut styled = env.layered_on(&defaults);
    if let Some(style) = style {
        style.install_tokens(&mut styled);
    }
    *env = env.layered_on(&styled);
}

/// Installs the framework default tokens underneath whatever `env` already
/// carries.
///
/// This is the `None`-style arm of [`install_theme_tokens`]. External test
/// harnesses (`waterui-testing`) call this; the runners go through
/// [`install_theme_tokens`].
pub fn install_default_tokens(env: &mut Environment) {
    install_theme_tokens(env, None);
}
