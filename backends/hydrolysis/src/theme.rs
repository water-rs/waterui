//! Hydrolysis's theme contract: what a style package implements, and how the
//! runtime assembles the environment's theme tokens.
//!
//! A style package (Material 3, a glass theme, a test theme) is a
//! [`Style`](crate::Style): a [`WidgetTheme`] plus a token installer. The
//! contract is everything a style package names from Hydrolysis:
//!
//! - [`WidgetTheme`], the widget chrome the renderer lays out and draws from:
//!   motion policies, per-widget metrics and the `draw_*` methods;
//! - the state the renderer hands a theme on every draw:
//!   [`WidgetInteractionState`] with its [`PressWaves`], and
//!   [`RadioIndicatorState`];
//! - the motion and metric token tables a theme returns, such as
//!   [`InteractionMotion`], [`ProgressMotion`] and [`ButtonMetrics`];
//! - the engine-provided chrome primitives a theme describes interactive
//!   chrome with instead of drawing it — [`Surface`], [`CornerShape`],
//!   [`Elevation`], [`StateLayer`], [`PressFeedback`] and
//!   [`FocusIndicator`], bundled per widget part as [`Chrome`] — which
//!   Hydrolysis records once and animates through live operands;
//! - the presentation plugins a style package installs on composed controls:
//!   [`InteractionStyle`], [`InteractionFocusBinding`],
//!   [`ModalInteraction`], and the [`IconOnlyButtonLabel`] marker the
//!   renderer installs for a theme to read.
//!
//! Drawing goes through the render-target-neutral recording vocabulary of
//! `cherenkov-record` ([`Recorder`](cherenkov_record::Recorder),
//! [`Paint`](cherenkov_record::Paint),
//! [`MaterialRegistry`](cherenkov_record::MaterialRegistry)), never through a
//! render engine, so a theme renders on every target Hydrolysis draws
//! through. A style package depends on Hydrolysis, the framework crates whose
//! configuration types the contract names, and `cherenkov-record`.
//!
//! # Token assembly
//!
//! `install_theme_tokens` installs the runtime's theme tokens in precedence
//! order: framework defaults < [`Style::install_tokens`](crate::Style) < the
//! application's own environment. Every runner calls it — the rendered
//! runners with their style, the semantic runtime with `None` — so the same
//! precedence holds whether or not a style package is present.
//!
//! The framework defaults are *not* a theme: they carry no widget theme and
//! no presentation decisions beyond the framework's baseline palette and
//! type scale. A `Style` may overwrite any of them, and the application
//! overwrites both.

mod chrome;
mod widget;

pub use chrome::{
    Border, Chrome, CornerShape, Corners, Elevation, FocusIndicator, PressFeedback, Ripple,
    StateLayer, Surface,
};
pub use widget::{
    BadgeMetrics, ButtonMetrics, ButtonSpec, DividerMetrics, FocusRing, InputFieldMetrics,
    InteractionFocusBinding, InteractionMotion, InteractionStyle, ListDividerMetrics, ListMetrics,
    ListRowMetrics, ListSectionMetrics, ListTrailingControlMetrics, MAX_PRESS_WAVES,
    ModalInteraction, NavigationMetrics, NavigationMotion, PickerMetrics, PressWave, PressWaves,
    ProgressIndicatorStyle, ProgressMetrics, ProgressMotion, RadioIndicatorState,
    RadioSelectionMotion, SliderMetrics, SliderValueIndicatorMetrics, StepperEnd, StepperMetrics,
    TabItemLayout, TableMetrics, TabsMetrics, TextCaretMotion, TextContextMenuMetrics,
    ToggleMetrics, WidgetInteractionState, WidgetTheme,
};

/// Marks the subtree of a button label that resolved to an icon-only
/// presentation.
///
/// The backend installs it into the icon-only label's environment so a
/// theme's resolvable [`WidgetTheme::button_label_color`] can paint the
/// standard icon button's content color rather than the filled text
/// button's — `ButtonStyle::Automatic` resolves to a different variant for
/// the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "field-less metadata marker inserted by name"
)]
pub struct IconOnlyButtonLabel;

impl waterui::Plugin for IconOnlyButtonLabel {}

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
                .error_foreground(color(0xFF_FF_FF))
                // Scrim: black at 32% over the content beneath a modal layer.
                .scrim(Srgb::BLACK.resolve().with_alpha(0.32)),
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
/// This is the `None`-style arm of `install_theme_tokens`. External test
/// harnesses (`waterui-testing`) call this; the runners go through
/// `install_theme_tokens`.
pub fn install_default_tokens(env: &mut Environment) {
    install_theme_tokens(env, None);
}
