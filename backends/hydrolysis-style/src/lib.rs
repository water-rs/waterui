//! Renderer-independent style and widget-theme contract for Hydrolysis.
//!
//! Style packages (such as `hydrolysis-m3` and `hydrolysis-glass`) depend on
//! this crate to implement [`Style`] and [`theme::WidgetTheme`] without
//! pulling in the Hydrolysis renderer, Cherenkov GPU/CPU painters, `winit`,
//! or Android JNI host dependencies.

pub mod theme;

use waterui_core::Environment;

/// A presentation style for the rendered Hydrolysis runtime: the type a style
/// package implements, and what `hydrolysis::run(app, env, style)` accepts.
///
/// A `Style` is the runtime's widget theme plus the package's token set:
/// the runtime assembles the environment's theme tokens with framework
/// defaults beneath [`Self::install_tokens`] beneath the application's own
/// environment ([`theme::install_theme_tokens`]), and finally owns the
/// style itself and hands `&dyn` [`WidgetTheme`] to the
/// layout and encode contexts. The [`theme`] module documents the whole
/// contract a style package targets.
///
/// The semantic runtime (`SemanticRuntime`) takes no `Style`: widget
/// structure, roles, labels, values, states and actions never depend on one.
/// A theme read during view build or patch is a compile error by
/// construction — build and patch contexts cannot reach a theme.
pub trait Style: theme::WidgetTheme + 'static {
    /// Installs the style package's colour, font and other environment
    /// tokens. The environment already carries the application's entries
    /// layered over the framework defaults, so an installed value can be
    /// read here (e.g. the application's colour scheme), and the
    /// application's own entries still win in the assembled environment.
    fn install_tokens(&self, env: &mut Environment);
}

/// The drawing vocabulary of the style contract: the `Recorder` a
/// [`WidgetTheme`] and its looks record into, `Paint`,
/// the shapes, the colour, material and animation types, and the `kurbo`
/// geometry the [`theme`] signatures name. A style package draws with these,
/// so it records with the same recorder the renderer consumes.
pub use cherenkov_record as draw;
pub use theme::*;
