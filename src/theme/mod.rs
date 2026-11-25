//! # Theme System
//!
//! The theme system provides a **type-safe bundle of colors and fonts** that can be
//! installed into an [`Environment`] to style all `WaterUI` components consistently.
//!
//! ## Design Principles
//!
//! - **System neutral**: `WaterUI` doesn't impose default colors. Native backends inject
//!   system colors, and the theme system just provides the wiring.
//! - **Reactive by design**: All theme values can be reactive (`Binding`, `Computed`)
//!   or static. When color scheme changes, all dependent colors update automatically.
//! - **Optional overrides**: Theme fields are optional. Only specified fields are
//!   installed; others retain their existing values or use native defaults.
//! - **Composable**: Theme is composed of smaller structs (`ColorSettings`, `FontSettings`)
//!   for easier maintenance and partial customization.
//!
//! ## For Users
//!
//! ### Quick Start
//!
//! ```rust
//! use waterui::color::Srgb;
//! use waterui::{Environment, theme::{Theme, ColorScheme, ColorSettings}};
//! use waterui_core::plugin::Plugin;
//! use nami::binding;
//!
//! let mut env = Environment::new();
//!
//! // Lock to dark mode (static)
//! Theme::new()
//!     .color_scheme(ColorScheme::Dark)
//!     .install(&mut env);
//!
//! // Or use a reactive binding that follows system preference
//! let system_scheme = binding::<ColorScheme>(ColorScheme::Light);
//! Theme::new()
//!     .color_scheme(system_scheme.clone())
//!     .install(&mut env);
//!
//! // Customize specific colors
//! Theme::new()
//!     .colors(ColorSettings::new().accent(Srgb::from_hex("#0066CC")))
//!     .install(&mut env);
//! ```
//!
//! ### Using Theme Tokens in Views
//!
//! Theme tokens are unit structs that implement `Resolvable`. Use them directly
//! with view modifiers:
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::theme::color::{Background, Foreground};
//!
//! let themed = text!("Hello, World!")
//!     .foreground(Foreground)  // Uses theme's foreground color
//!     .background(Background); // Uses theme's background color
//! ```
//!
//! ### Available Tokens
//!
//! **Color Scheme** (`theme::ColorScheme`):
//! - `Light` - Light appearance
//! - `Dark` - Dark appearance
//!
//! **Colors** (`theme::color::*`):
//! - `Background` - Primary background
//! - `Surface` - Elevated surfaces (cards, sheets)
//! - `SurfaceVariant` - Alternate surface color
//! - `Border` - Borders and dividers
//! - `Foreground` - Primary text and icons
//! - `MutedForeground` - Secondary/dimmed text
//! - `TertiaryForeground` - Tertiary text, fainter than `MutedForeground`
//! - `DisabledForeground` - Text and glyphs of a control or item that cannot be used
//! - `Accent`- Interactive elements, links
//! - `AccentContainer` - Container backgrounds associated with the accent color
//! - `AccentForeground` - Text on accent backgrounds
//! - `Tertiary` - Contrasting accent for complementary emphasis
//! - `TertiaryContainer` - Container backgrounds associated with the tertiary color
//! - `SelectionContainer` - Fill painted behind a selected item
//! - `SelectionForeground` - Content drawn on the selection container
//! - `UnemphasizedSelection` - Fill painted behind a selected item while its control is not focused
//! - `Error`- Destructive / error emphasis (badges, destructive buttons, validation)
//! - `ErrorForeground` - Content drawn on the error color
//!
//! **Fonts**: Use standard font tokens from `waterui::text::font`:
//! - `Body`, `Title`, `Headline`, `Subheadline`, `Caption`, `Footnote`
//!
//! ## For Maintainers
//!
//! ### How It Works
//!
//! 1. [`Theme`] composes [`ColorSettings`] and [`FontSettings`]
//! 2. Each settings struct holds optional `Computed<T>` signals
//! 3. Builder methods accept `impl IntoSignal<T>` - works with both
//!    static values and reactive bindings
//! 4. `Theme::install()` delegates to each settings struct's install method
//! 5. Only non-None fields are installed into the environment
//!
//! ### Native Backend Integration
//!
//! Native backends and style packages should:
//! 1. Create a `Binding<ColorScheme>` that tracks system appearance and install
//!    it with [`install_color_scheme`] (or `Theme::new().color_scheme(..)`)
//! 2. Install each scheme-dependent colour slot as a [`ColorDerivation`] with
//!    [`install_color_derivation`]: the slot keeps the derivation, so a
//!    colour scheme installed later on a subtree re-derives that subtree's
//!    slots from the subtree's scheme
//!
//! A colour given as a signal ([`ColorSettings`], [`install_color_signal`]) is
//! the same in every scheme and is left as it is by a later scheme.

use nami::{Computed, SignalExt, signal::IntoSignal};
use waterui_core::{Environment, env::Store, plugin::Plugin};

use crate::{
    color::{Color, WorkingColor},
    text::font::{Body, Caption, FontSlot, Footnote, Headline, ResolvedFont, Subheadline, Title},
};

// ============================================================================
// ColorScheme - Light/Dark appearance preference
// ============================================================================

/// The color scheme preference for the UI.
///
/// Defined beside the colour tokens in `waterui_graphics::color` so component
/// crates can resolve it through [`CurrentColorScheme`] without depending on
/// this crate; installed and read here.
///
/// # Example
///
/// ```rust
/// use waterui::theme::{Theme, ColorScheme};
/// use nami::binding;
///
/// // Static: always dark
/// Theme::new().color_scheme(ColorScheme::Dark);
///
/// // Reactive: follows system
/// let system_scheme = binding(ColorScheme::Light);
/// Theme::new().color_scheme(system_scheme);
/// ```
pub use waterui_graphics::color::{ColorScheme, CurrentColorScheme};

// ============================================================================
// ColorSettings - All color overrides
// ============================================================================

/// Color settings for a theme.
///
/// All fields are optional. Only specified colors will be installed.
/// Use the builder pattern to set individual colors.
///
/// # Example
///
/// ```rust
/// use waterui::color::Srgb;
/// use waterui::theme::ColorSettings;
///
/// let colors = ColorSettings::new()
///     .accent(Srgb::from_hex("#0066CC"))
///     .foreground(Srgb::from_hex("#111111"));
/// ```
#[derive(Default, Debug)]
pub struct ColorSettings {
    background: Option<Computed<WorkingColor>>,
    surface: Option<Computed<WorkingColor>>,
    surface_variant: Option<Computed<WorkingColor>>,
    border: Option<Computed<WorkingColor>>,
    foreground: Option<Computed<WorkingColor>>,
    muted_foreground: Option<Computed<WorkingColor>>,
    tertiary_foreground: Option<Computed<WorkingColor>>,
    disabled_foreground: Option<Computed<WorkingColor>>,
    accent: Option<Computed<WorkingColor>>,
    accent_container: Option<Computed<WorkingColor>>,
    accent_foreground: Option<Computed<WorkingColor>>,
    tertiary: Option<Computed<WorkingColor>>,
    tertiary_container: Option<Computed<WorkingColor>>,
    selection_container: Option<Computed<WorkingColor>>,
    selection_foreground: Option<Computed<WorkingColor>>,
    unemphasized_selection: Option<Computed<WorkingColor>>,
    error: Option<Computed<WorkingColor>>,
    error_foreground: Option<Computed<WorkingColor>>,
    scrim: Option<Computed<WorkingColor>>,
}

impl ColorSettings {
    /// Creates empty color settings with no overrides.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the background color.
    #[must_use]
    pub fn background(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.background = Some(color.into_signal().computed());
        self
    }

    /// Sets the surface color (cards, sheets).
    #[must_use]
    pub fn surface(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.surface = Some(color.into_signal().computed());
        self
    }

    /// Sets the surface variant color.
    #[must_use]
    pub fn surface_variant(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.surface_variant = Some(color.into_signal().computed());
        self
    }

    /// Sets the border color.
    #[must_use]
    pub fn border(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.border = Some(color.into_signal().computed());
        self
    }

    /// Sets the foreground color (text, icons).
    #[must_use]
    pub fn foreground(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.foreground = Some(color.into_signal().computed());
        self
    }

    /// Sets the muted foreground color (secondary text).
    #[must_use]
    pub fn muted_foreground(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.muted_foreground = Some(color.into_signal().computed());
        self
    }

    /// Sets the tertiary foreground color (text fainter than the muted tier).
    #[must_use]
    pub fn tertiary_foreground(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.tertiary_foreground = Some(color.into_signal().computed());
        self
    }

    /// Sets the disabled foreground color (text of an unusable control or item).
    #[must_use]
    pub fn disabled_foreground(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.disabled_foreground = Some(color.into_signal().computed());
        self
    }

    /// Sets the accent color (interactive elements).
    #[must_use]
    pub fn accent(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.accent = Some(color.into_signal().computed());
        self
    }

    /// Sets the accent container color.
    #[must_use]
    pub fn accent_container(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.accent_container = Some(color.into_signal().computed());
        self
    }

    /// Sets the accent foreground color (text on accent).
    #[must_use]
    pub fn accent_foreground(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.accent_foreground = Some(color.into_signal().computed());
        self
    }

    /// Sets the tertiary accent color.
    #[must_use]
    pub fn tertiary(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.tertiary = Some(color.into_signal().computed());
        self
    }

    /// Sets the tertiary container color.
    #[must_use]
    pub fn tertiary_container(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.tertiary_container = Some(color.into_signal().computed());
        self
    }

    /// Sets the fill painted behind a selected item.
    #[must_use]
    pub fn selection_container(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.selection_container = Some(color.into_signal().computed());
        self
    }

    /// Sets the foreground drawn on the selection container.
    #[must_use]
    pub fn selection_foreground(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.selection_foreground = Some(color.into_signal().computed());
        self
    }

    /// Sets the fill painted behind a selected item while its control is not focused.
    #[must_use]
    pub fn unemphasized_selection(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.unemphasized_selection = Some(color.into_signal().computed());
        self
    }

    /// Sets the error color (destructive / error emphasis).
    #[must_use]
    pub fn error(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.error = Some(color.into_signal().computed());
        self
    }

    /// Sets the foreground drawn on the error color.
    #[must_use]
    pub fn error_foreground(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.error_foreground = Some(color.into_signal().computed());
        self
    }

    /// Sets the scrim color (the dimming backdrop behind a modal presentation).
    #[must_use]
    pub fn scrim(mut self, color: impl IntoSignal<WorkingColor>) -> Self {
        self.scrim = Some(color.into_signal().computed());
        self
    }

    /// Installs the color settings into the environment.
    /// Only non-None fields are installed.
    fn install(self, env: &mut Environment) {
        if let Some(signal) = self.background {
            install_color_signal::<color::Background>(env, signal);
        }
        if let Some(signal) = self.surface {
            install_color_signal::<color::Surface>(env, signal);
        }
        if let Some(signal) = self.surface_variant {
            install_color_signal::<color::SurfaceVariant>(env, signal);
        }
        if let Some(signal) = self.border {
            install_color_signal::<color::Border>(env, signal);
        }
        if let Some(signal) = self.foreground {
            install_color_signal::<color::Foreground>(env, signal);
        }
        if let Some(signal) = self.muted_foreground {
            install_color_signal::<color::MutedForeground>(env, signal);
        }
        if let Some(signal) = self.tertiary_foreground {
            install_color_signal::<color::TertiaryForeground>(env, signal);
        }
        if let Some(signal) = self.disabled_foreground {
            install_color_signal::<color::DisabledForeground>(env, signal);
        }
        if let Some(signal) = self.accent {
            install_color_signal::<color::Accent>(env, signal);
        }
        if let Some(signal) = self.accent_container {
            install_color_signal::<color::AccentContainer>(env, signal);
        }
        if let Some(signal) = self.accent_foreground {
            install_color_signal::<color::AccentForeground>(env, signal);
        }
        if let Some(signal) = self.tertiary {
            install_color_signal::<color::Tertiary>(env, signal);
        }
        if let Some(signal) = self.tertiary_container {
            install_color_signal::<color::TertiaryContainer>(env, signal);
        }
        if let Some(signal) = self.selection_container {
            install_color_signal::<color::SelectionContainer>(env, signal);
        }
        if let Some(signal) = self.selection_foreground {
            install_color_signal::<color::SelectionForeground>(env, signal);
        }
        if let Some(signal) = self.unemphasized_selection {
            install_color_signal::<color::UnemphasizedSelection>(env, signal);
        }
        if let Some(signal) = self.error {
            install_color_signal::<color::Error>(env, signal);
        }
        if let Some(signal) = self.error_foreground {
            install_color_signal::<color::ErrorForeground>(env, signal);
        }
        if let Some(signal) = self.scrim {
            install_color_signal::<color::Scrim>(env, signal);
        }
    }
}

// ============================================================================
// FontSettings - All font overrides
// ============================================================================

/// Font settings for a theme.
///
/// All fields are optional. Only specified fonts will be installed.
/// Use the builder pattern to set individual fonts.
///
/// # Example
///
/// ```rust
/// use waterui::text::font::{FontWeight, ResolvedFont};
/// use waterui::theme::FontSettings;
///
/// let fonts = FontSettings::new()
///     .body(ResolvedFont::new(16.0, FontWeight::Normal))
///     .title(ResolvedFont::new(28.0, FontWeight::Bold));
/// ```
#[derive(Default, Debug)]
pub struct FontSettings {
    body: Option<Computed<ResolvedFont>>,
    title: Option<Computed<ResolvedFont>>,
    headline: Option<Computed<ResolvedFont>>,
    subheadline: Option<Computed<ResolvedFont>>,
    caption: Option<Computed<ResolvedFont>>,
    footnote: Option<Computed<ResolvedFont>>,
}

impl FontSettings {
    /// Creates empty font settings with no overrides.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates the full default type scale, with every slot set to its
    /// [`FontSlot::DEFAULT`].
    ///
    /// This is the scale every backend installs when the application theme
    /// leaves a font slot unset; use [`FontSettings::new`] to start from an
    /// empty override set instead.
    #[must_use]
    pub fn default_scale() -> Self {
        Self::new()
            .body(<Body as FontSlot>::DEFAULT)
            .title(<Title as FontSlot>::DEFAULT)
            .headline(<Headline as FontSlot>::DEFAULT)
            .subheadline(<Subheadline as FontSlot>::DEFAULT)
            .caption(<Caption as FontSlot>::DEFAULT)
            .footnote(<Footnote as FontSlot>::DEFAULT)
    }

    /// Sets the body font.
    #[must_use]
    pub fn body(mut self, font: impl IntoSignal<ResolvedFont>) -> Self {
        self.body = Some(font.into_signal().computed());
        self
    }

    /// Sets the title font.
    #[must_use]
    pub fn title(mut self, font: impl IntoSignal<ResolvedFont>) -> Self {
        self.title = Some(font.into_signal().computed());
        self
    }

    /// Sets the headline font.
    #[must_use]
    pub fn headline(mut self, font: impl IntoSignal<ResolvedFont>) -> Self {
        self.headline = Some(font.into_signal().computed());
        self
    }

    /// Sets the subheadline font.
    #[must_use]
    pub fn subheadline(mut self, font: impl IntoSignal<ResolvedFont>) -> Self {
        self.subheadline = Some(font.into_signal().computed());
        self
    }

    /// Sets the caption font.
    #[must_use]
    pub fn caption(mut self, font: impl IntoSignal<ResolvedFont>) -> Self {
        self.caption = Some(font.into_signal().computed());
        self
    }

    /// Sets the footnote font.
    #[must_use]
    pub fn footnote(mut self, font: impl IntoSignal<ResolvedFont>) -> Self {
        self.footnote = Some(font.into_signal().computed());
        self
    }

    /// Installs the font settings into the environment.
    /// Only non-None fields are installed.
    fn install(self, env: &mut Environment) {
        if let Some(signal) = self.body {
            install_font_signal::<Body>(env, signal);
        }
        if let Some(signal) = self.title {
            install_font_signal::<Title>(env, signal);
        }
        if let Some(signal) = self.headline {
            install_font_signal::<Headline>(env, signal);
        }
        if let Some(signal) = self.subheadline {
            install_font_signal::<Subheadline>(env, signal);
        }
        if let Some(signal) = self.caption {
            install_font_signal::<Caption>(env, signal);
        }
        if let Some(signal) = self.footnote {
            install_font_signal::<Footnote>(env, signal);
        }
    }
}

// ============================================================================
// Theme - Composes all settings
// ============================================================================

/// A theme configuration composed of color scheme, colors, and fonts.
///
/// Use the builder pattern to configure what to override. Only specified
/// values are installed; others retain existing values.
///
/// # Example
///
/// ```rust
/// use nami::binding;
/// use waterui::Environment;
/// use waterui::color::Srgb;
/// use waterui::text::font::{FontWeight, ResolvedFont};
/// use waterui::theme::{ColorScheme, ColorSettings, FontSettings, Theme};
/// use waterui_core::plugin::Plugin;
///
/// let mut env = Environment::new();
///
/// // Create a theme with reactive color scheme
/// let scheme = binding::<ColorScheme>(ColorScheme::Light);
/// Theme::new()
///     .color_scheme(scheme)
///     .colors(ColorSettings::new().accent(Srgb::from_hex("#0066CC")))
///     .fonts(FontSettings::new().body(ResolvedFont::new(16.0, FontWeight::Normal)))
///     .install(&mut env);
/// ```
#[derive(Default, Debug)]
pub struct Theme {
    color_scheme: Option<Computed<ColorScheme>>,
    colors: Option<ColorSettings>,
    fonts: Option<FontSettings>,
}

impl Theme {
    /// Creates a new empty theme with no overrides.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the color scheme (light/dark).
    ///
    /// Accepts any value that implements `IntoSignal<ColorScheme>`:
    /// - Static: `ColorScheme::Dark`
    /// - Reactive: `binding(ColorScheme::Light)`
    #[must_use]
    pub fn color_scheme(mut self, scheme: impl IntoSignal<ColorScheme>) -> Self {
        self.color_scheme = Some(scheme.into_signal().computed());
        self
    }

    /// Sets the color settings.
    #[must_use]
    pub fn colors(mut self, colors: ColorSettings) -> Self {
        self.colors = Some(colors);
        self
    }

    /// Sets the font settings.
    #[must_use]
    pub fn fonts(mut self, fonts: FontSettings) -> Self {
        self.fonts = Some(fonts);
        self
    }
}

impl Plugin for Theme {
    /// Installs this theme into the environment.
    ///
    /// Only non-None fields are installed. Existing values for unspecified
    /// fields remain unchanged.
    fn install(self, env: &mut Environment) {
        // Install color scheme if specified
        if let Some(scheme) = self.color_scheme {
            install_color_scheme(env, scheme);
        }

        // Install color settings if specified
        if let Some(colors) = self.colors {
            colors.install(env);
        }

        // Install font settings if specified
        if let Some(fonts) = self.fonts {
            fonts.install(env);
        }
    }
}

// ============================================================================
// Color Tokens, Derivations, Slot Storage, and Tint (defined in waterui-color)
// ============================================================================

pub use waterui_graphics::color::{
    ColorDerivation, ColorSlot, Tint, current_color_scheme, install_color_derivation,
    install_color_scheme, install_color_signal, installed_color_scheme, installed_color_signal,
    resolve_tint, token as color,
};

/// Installs an explicit font signal for a specific slot.
///
/// This is primarily used by native backends (via FFI) to inject platform-specific
/// font signals. Uses `Store<T, Computed<ResolvedFont>>` to be compatible with
/// the existing font resolution system.
pub fn install_font_signal<T: 'static>(env: &mut Environment, signal: Computed<ResolvedFont>) {
    env.insert(Store::<T, Computed<ResolvedFont>>::new(signal));
}

// ============================================================================
// ForegroundOverride - Plugin for .foreground() view modifier
// ============================================================================

/// A plugin that overrides the foreground color for a view subtree.
///
/// This is used by the `.foreground()` view modifier to inject a custom
/// foreground color into the environment.
#[derive(Debug)]
pub struct ForegroundOverride {
    color: Color,
}

impl ForegroundOverride {
    /// Creates a new foreground override with the specified color.
    pub fn new(color: impl IntoSignal<Color> + 'static) -> Self {
        Self {
            color: Color::from_signal(color),
        }
    }
}

impl Plugin for ForegroundOverride {
    fn install(self, env: &mut Environment) {
        let resolved = self.color.resolve(env);
        install_color_signal::<color::Foreground>(env, resolved);
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::cell::Cell;
    use nami::{Binding, Signal, SignalExt};
    use waterui_core::resolve::Resolvable;
    use waterui_graphics::color::{AccentColor, Color, Working};

    use super::*;

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "resolves opaque black"))]
    fn an_uninstalled_color_slot_resolves_opaque_black() {
        let env = Environment::new();
        let resolved = color::Foreground.resolve(&env).snapshot();
        assert_eq!(resolved.components, WorkingColor::BLACK.components);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "so it is the light scheme"))]
    fn an_uninstalled_color_scheme_is_light() {
        let scheme = current_color_scheme(&Environment::new()).snapshot();
        assert_eq!(scheme, ColorScheme::Light);
    }

    #[test]
    fn tint_resolves_to_the_nearest_tint_else_the_accent() {
        let accent = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
        let outer = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
        let inner = WorkingColor::new([0.0, 1.0, 0.0, 1.0]);
        let mut env = Environment::new();
        install_color_signal::<color::Accent>(&mut env, nami::constant(accent).computed());
        let channels = |env: &Environment| resolve_tint(env).resolve(env).snapshot().components;

        assert_eq!(channels(&env), accent.components);
        let outer_env = env.extending(Tint::new(Color::new(Working(outer))));
        assert_eq!(channels(&outer_env), outer.components);
        let inner_env = outer_env.extending(Tint::new(Color::new(Working(inner))));
        assert_eq!(channels(&inner_env), inner.components);
        assert_eq!(channels(&outer_env), outer.components);
    }

    #[test]
    fn tint_signal_updates_in_place() {
        let source = Binding::container(WorkingColor::BLACK);
        let tint = Tint::new(source.map(|color| Color::new(Working(color))));
        let env = Environment::new().extending(tint.clone());
        let resolved = resolve_tint(&env).resolve(&env);
        assert!(resolved.snapshot().components[0].abs() < f32::EPSILON);

        source.set(WorkingColor::new([1.0, 0.0, 0.0, 1.0]));

        assert!((resolved.snapshot().components[0] - 1.0).abs() < f32::EPSILON);
        assert_eq!(Tint::inherited(&env), Some(&tint), "the tint is unchanged");
    }

    #[test]
    fn color_modifiers_accept_tokens_palette_structs_and_signals() {
        use crate::ViewExt as _;
        use crate::background::Glass;
        use crate::border::Border;
        use crate::color::{Blue, Oklch, P3, Red, Srgb};
        use crate::component::button;
        use crate::component::list::row;
        use crate::shape::{Rectangle, ShapeExt as _};
        use crate::text::text;
        use crate::theme::color::{Accent, SurfaceVariant};
        use crate::widget::avatar;
        use waterui_icon::IconGlyph;

        let selected = Binding::bool(false);
        let signal = selected.select(Color::from(Accent), Color::from(SurfaceVariant));
        let binding_color = Binding::container(Color::from(Srgb::new(1.0, 0.0, 0.0)));
        let computed_color = binding_color.computed();

        // Palette structs and theme tokens work directly across modifiers.
        let _ = text("a").foreground(Accent);
        let _ = text("b").foreground(Red);
        let _ = text("c").foreground(Srgb::new(0.0, 0.0, 1.0));
        let _ = text("d").foreground(P3::new(0.2, 0.4, 0.8));
        let _ = text("e").foreground(Oklch::new(0.6, 0.15, 250.0));
        let _ = text("f").foreground(signal.clone());
        let _ = text("g").tint(Accent);
        let _ = text("h").tint(signal.clone());
        let _ = text("i").border(Blue, 1.0);
        let _ = text("j").border(signal.clone(), 1.0);
        let _ = Border::new(signal.clone(), 2.0);
        let _ = text("k").background(signal.clone());
        let _ = text("l").background(binding_color);
        let _ = text("m").background(computed_color);
        let _ = Glass::regular().tint(signal.clone());
        let _ = Rectangle.fill(signal.clone());
        let _ = Rectangle.fill(Red);
        let _ = button("Inbox").action(|| {}).badge(3_u32).color(Accent);
        let _ = button("Inbox")
            .action(|| {})
            .badge(3_u32)
            .color(Srgb::new(1.0, 0.0, 0.0));
        let _ = button("Inbox")
            .action(|| {})
            .badge(3_u32)
            .color(signal.clone());
        let _ = avatar("Ada").ring(Accent, 2.0).ring(signal.clone(), 2.0);
        let _ = row("Key", "Val")
            .value_color(Accent)
            .value_color(signal.clone());
        let _ = IconGlyph::new('\u{e88a}', "MaterialIcons-Regular")
            .color(Accent)
            .color(signal);

        // ForegroundOverride resolves a reactive signal in place.
        let active = Binding::bool(false);
        let light = WorkingColor::new([0.1, 0.2, 0.3, 1.0]);
        let dark = WorkingColor::new([0.9, 0.8, 0.7, 1.0]);
        let mut env = Environment::new();
        ForegroundOverride::new(
            active.select(Color::new(Working(dark)), Color::new(Working(light))),
        )
        .install(&mut env);
        let fg = color::Foreground.resolve(&env);
        assert_eq!(fg.snapshot().components, light.components);
        active.set(true);
        assert_eq!(fg.snapshot().components, dark.components);
    }

    #[test]
    fn tier_tokens_install_and_mirror_into_graphics_keys() {
        use waterui_graphics::color::{
            DisabledForegroundColor, TertiaryForegroundColor, UnemphasizedSelectionColor,
        };

        let red = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
        let green = WorkingColor::new([0.0, 1.0, 0.0, 1.0]);
        let blue = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
        let mut env = Environment::new();
        Theme::new()
            .colors(
                ColorSettings::new()
                    .tertiary_foreground(red)
                    .disabled_foreground(green)
                    .unemphasized_selection(blue),
            )
            .install(&mut env);

        let channel = |color: WorkingColor| color.components;
        assert_eq!(
            channel(color::TertiaryForeground.resolve(&env).snapshot()),
            channel(red)
        );
        assert_eq!(
            channel(color::DisabledForeground.resolve(&env).snapshot()),
            channel(green)
        );
        assert_eq!(
            channel(color::UnemphasizedSelection.resolve(&env).snapshot()),
            channel(blue)
        );
        assert_eq!(
            channel(Color::new(TertiaryForegroundColor).resolve(&env).snapshot()),
            channel(red)
        );
        assert_eq!(
            channel(Color::new(DisabledForegroundColor).resolve(&env).snapshot()),
            channel(green)
        );
        assert_eq!(
            channel(
                Color::new(UnemphasizedSelectionColor)
                    .resolve(&env)
                    .snapshot()
            ),
            channel(blue)
        );
    }

    /// A scheme installed on a subtree re-derives a per-scheme slot there,
    /// the graphics mirror included, and the slot keeps following its
    /// per-scheme signals; a slot installed as a signal is left as it is.
    #[test]
    fn subtree_scheme_rederives_per_scheme_slots() {
        use waterui_graphics::color::BackgroundColor;

        let red = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
        let green = WorkingColor::new([0.0, 1.0, 0.0, 1.0]);
        let blue = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
        let light = Binding::container(red);
        let dark = Binding::container(blue);
        let mut env = Environment::new();
        install_color_scheme(&mut env, Computed::constant(ColorScheme::Light));
        install_color_derivation::<color::Background>(
            &mut env,
            ColorDerivation::per_scheme(light, dark.clone()),
        );
        install_color_signal::<color::Accent>(&mut env, Computed::constant(green));

        let mut subtree = env.clone();
        install_color_scheme(&mut subtree, Computed::constant(ColorScheme::Dark));

        let channels = |env: &Environment| color::Background.resolve(env).snapshot().components;
        assert_eq!(channels(&env), red.components);
        assert_eq!(channels(&subtree), blue.components);
        assert_eq!(
            Color::new(BackgroundColor)
                .resolve(&subtree)
                .snapshot()
                .components,
            blue.components
        );
        assert_eq!(
            color::Accent.resolve(&subtree).snapshot().components,
            green.components
        );

        let resolved = color::Background.resolve(&subtree);
        dark.set(green);
        assert_eq!(resolved.snapshot().components, green.components);
    }

    #[test]
    fn graphics_accent_color_tracks_installed_theme_signal() {
        let accent = Binding::container(WorkingColor::BLACK);
        let mut env = Environment::new();
        install_color_signal::<color::Accent>(&mut env, accent.computed());
        let resolved = Color::new(AccentColor).resolve(&env);
        let observed_red = Rc::new(Cell::new(0.0));
        let captured_red = Rc::clone(&observed_red);
        let _guard = resolved.watch(move |context| {
            captured_red.set(context.into_value().components[0]);
        });

        accent.set(WorkingColor::new([1.0, 0.0, 0.0, 1.0]));

        assert!((observed_red.get() - 1.0).abs() < f32::EPSILON);
        assert!((resolved.snapshot().components[0] - 1.0).abs() < f32::EPSILON);
    }
}
