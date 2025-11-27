//! # Theme System
//!
//! The theme system provides a **type-safe bundle of colors and fonts** that can be
//! installed into an [`Environment`] to style all WaterUI components consistently.
//!
//! ## Design Principles
//!
//! - **System neutral**: WaterUI doesn't impose default colors. Native backends inject
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
//! ```ignore
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
//! let system_scheme = binding(ColorScheme::Light);
//! Theme::new()
//!     .color_scheme(system_scheme.clone())
//!     .install(&mut env);
//!
//! // Customize specific colors
//! Theme::new()
//!     .colors(ColorSettings::new().accent(my_accent_color))
//!     .install(&mut env);
//! ```
//!
//! ### Using Theme Tokens in Views
//!
//! Theme tokens are unit structs that implement `Resolvable`. Use them directly
//! with view modifiers:
//!
//! ```ignore
//! use waterui::theme::color::{Foreground, Accent, Background};
//!
//! text("Hello, World!")
//!     .foreground(Foreground)  // Uses theme's foreground color
//!     .background(Background)  // Uses theme's background color
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
//! - `Accent` - Interactive elements, links
//! - `AccentForeground` - Text on accent backgrounds
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
//! Native backends should:
//! 1. Create a `Binding<ColorScheme>` that tracks system appearance
//! 2. Create `Computed<ResolvedColor>` signals that react to color scheme changes
//! 3. Install via `Theme::new().color_scheme(binding).colors(ColorSettings::new()...)`

use core::marker::PhantomData;

use nami::{Computed, SignalExt, impl_constant, signal::IntoSignal};
use waterui_core::{
    Environment,
    env::{Store, WithEnv},
    plugin::Plugin,
};

use crate::{
    View,
    color::ResolvedColor,
    text::font::{Body, Caption, Footnote, Headline, ResolvedFont, Subheadline, Title},
};

// ============================================================================
// ColorScheme - Light/Dark appearance preference
// ============================================================================

/// The color scheme preference for the UI.
///
/// This is used to switch between light and dark appearances. Native backends
/// typically bind this to the system appearance setting.
///
/// # Example
///
/// ```ignore
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorScheme {
    /// Light appearance (light backgrounds, dark text).
    #[default]
    Light,
    /// Dark appearance (dark backgrounds, light text).
    Dark,
}

impl_constant!(ColorScheme);

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
/// ```ignore
/// use waterui::theme::ColorSettings;
/// use waterui::color::Color;
///
/// let colors = ColorSettings::new()
///     .accent(my_accent_color)
///     .foreground(my_text_color);
/// ```
#[derive(Default)]
pub struct ColorSettings {
    background: Option<Computed<ResolvedColor>>,
    surface: Option<Computed<ResolvedColor>>,
    surface_variant: Option<Computed<ResolvedColor>>,
    border: Option<Computed<ResolvedColor>>,
    foreground: Option<Computed<ResolvedColor>>,
    muted_foreground: Option<Computed<ResolvedColor>>,
    accent: Option<Computed<ResolvedColor>>,
    accent_foreground: Option<Computed<ResolvedColor>>,
}

impl ColorSettings {
    /// Creates empty color settings with no overrides.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the background color.
    #[must_use]
    pub fn background(mut self, color: impl IntoSignal<ResolvedColor>) -> Self {
        self.background = Some(color.into_signal().computed());
        self
    }

    /// Sets the surface color (cards, sheets).
    #[must_use]
    pub fn surface(mut self, color: impl IntoSignal<ResolvedColor>) -> Self {
        self.surface = Some(color.into_signal().computed());
        self
    }

    /// Sets the surface variant color.
    #[must_use]
    pub fn surface_variant(mut self, color: impl IntoSignal<ResolvedColor>) -> Self {
        self.surface_variant = Some(color.into_signal().computed());
        self
    }

    /// Sets the border color.
    #[must_use]
    pub fn border(mut self, color: impl IntoSignal<ResolvedColor>) -> Self {
        self.border = Some(color.into_signal().computed());
        self
    }

    /// Sets the foreground color (text, icons).
    #[must_use]
    pub fn foreground(mut self, color: impl IntoSignal<ResolvedColor>) -> Self {
        self.foreground = Some(color.into_signal().computed());
        self
    }

    /// Sets the muted foreground color (secondary text).
    #[must_use]
    pub fn muted_foreground(mut self, color: impl IntoSignal<ResolvedColor>) -> Self {
        self.muted_foreground = Some(color.into_signal().computed());
        self
    }

    /// Sets the accent color (interactive elements).
    #[must_use]
    pub fn accent(mut self, color: impl IntoSignal<ResolvedColor>) -> Self {
        self.accent = Some(color.into_signal().computed());
        self
    }

    /// Sets the accent foreground color (text on accent).
    #[must_use]
    pub fn accent_foreground(mut self, color: impl IntoSignal<ResolvedColor>) -> Self {
        self.accent_foreground = Some(color.into_signal().computed());
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
        if let Some(signal) = self.accent {
            install_color_signal::<color::Accent>(env, signal);
        }
        if let Some(signal) = self.accent_foreground {
            install_color_signal::<color::AccentForeground>(env, signal);
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
/// ```ignore
/// use waterui::theme::FontSettings;
/// use waterui::text::font::ResolvedFont;
///
/// let fonts = FontSettings::new()
///     .body(my_body_font)
///     .title(my_title_font);
/// ```
#[derive(Default)]
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
/// ```ignore
/// use waterui::theme::{Theme, ColorScheme, ColorSettings, FontSettings};
/// use nami::binding;
///
/// // Create a theme with reactive color scheme
/// let scheme = binding(ColorScheme::Light);
/// Theme::new()
///     .color_scheme(scheme)
///     .colors(ColorSettings::new().accent(my_accent))
///     .fonts(FontSettings::new().body(my_body_font))
///     .install(&mut env);
/// ```
#[derive(Default)]
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
            env.insert(ColorSchemeSignal(scheme));
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
// Color Tokens - Resolvable types for each color slot
// ============================================================================

/// Color token definitions.
///
/// These unit structs implement `Resolvable<Resolved = ResolvedColor>`, so they
/// can be used directly with view modifiers like `.foreground(Foreground)`.
pub mod color {
    use super::{Environment, ResolvedColor};
    use nami::{Signal, impl_constant};
    use waterui_core::resolve::Resolvable;

    macro_rules! define_color_token {
        ($name:ident, $doc:literal) => {
            #[doc = $doc]
            #[derive(Debug, Clone, Copy, Default)]
            pub struct $name;

            impl Resolvable for $name {
                type Resolved = ResolvedColor;

                fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
                    super::resolve_color_slot::<Self>(env)
                }
            }

            impl_constant!($name);
        };
    }

    define_color_token!(Background, "Primary background color.");
    define_color_token!(Surface, "Elevated surface color (cards, sheets).");
    define_color_token!(SurfaceVariant, "Alternate surface color.");
    define_color_token!(Border, "Border and divider color.");
    define_color_token!(Foreground, "Primary text and icon color.");
    define_color_token!(MutedForeground, "Secondary/dimmed text color.");
    define_color_token!(Accent, "Accent color for interactive elements.");
    define_color_token!(AccentForeground, "Foreground on accent backgrounds.");
}

// ============================================================================
// Internal: Storage and Resolution
// ============================================================================

/// Storage for the color scheme signal.
#[derive(Clone)]
struct ColorSchemeSignal(Computed<ColorScheme>);

/// Internal storage for a color signal in the environment.
#[derive(Clone)]
struct ColorSlotValue<T> {
    signal: Computed<ResolvedColor>,
    _marker: PhantomData<T>,
}

impl<T> ColorSlotValue<T> {
    fn new(signal: Computed<ResolvedColor>) -> Self {
        Self {
            signal,
            _marker: PhantomData,
        }
    }
}

/// Resolves a color slot by looking up the stored signal.
///
/// Returns a transparent fallback if no signal is installed. Native backends
/// should always install proper defaults.
fn resolve_color_slot<T: 'static>(env: &Environment) -> Computed<ResolvedColor> {
    env.get::<ColorSlotValue<T>>()
        .map(|v| v.signal.clone())
        .unwrap_or_else(|| {
            // Fallback: transparent (native should provide real defaults)
            Computed::constant(ResolvedColor {
                red: 0.0,
                green: 0.0,
                blue: 0.0,
                headroom: 0.0,
                opacity: 0.0,
            })
        })
}

// ============================================================================
// Public API for Native Backends (FFI)
// ============================================================================

/// Returns the current color scheme signal from the environment.
///
/// If no color scheme is installed, returns a constant `Light` signal.
#[must_use]
pub fn current_color_scheme(env: &Environment) -> Computed<ColorScheme> {
    env.get::<ColorSchemeSignal>()
        .map(|s| s.0.clone())
        .unwrap_or_else(|| Computed::constant(ColorScheme::Light))
}

/// Installs an explicit color signal for a specific slot.
///
/// This is primarily used by native backends (via FFI) to inject platform-specific
/// color signals that can update reactively (e.g., when dark mode toggles).
///
/// # Example
///
/// ```ignore
/// // Native backend creates a reactive color signal
/// let dark_mode_color: Computed<ResolvedColor> = native_create_signal();
///
/// // Install it for the Foreground slot
/// install_color_signal::<color::Foreground>(&mut env, dark_mode_color);
/// ```
pub fn install_color_signal<T: 'static>(env: &mut Environment, signal: Computed<ResolvedColor>) {
    env.insert(ColorSlotValue::<T>::new(signal));
}

/// Installs an explicit font signal for a specific slot.
///
/// This is primarily used by native backends (via FFI) to inject platform-specific
/// font signals. Uses `Store<T, Computed<ResolvedFont>>` to be compatible with
/// the existing font resolution system.
pub fn install_font_signal<T: 'static>(env: &mut Environment, signal: Computed<ResolvedFont>) {
    env.insert(Store::<T, Computed<ResolvedFont>>::new(signal));
}

/// Installs a color scheme signal.
///
/// This is used by native backends to inject a reactive color scheme that
/// tracks the system appearance setting.
pub fn install_color_scheme(env: &mut Environment, signal: Computed<ColorScheme>) {
    env.insert(ColorSchemeSignal(signal));
}

// ============================================================================
// ThemeProvider - For layering themes in a view subtree
// ============================================================================

/// Applies a theme to a subtree of views.
///
/// Use this to override theme values for a specific part of your UI:
///
/// ```ignore
/// ThemeProvider::new(
///     my_card_view,
///     Theme::new().color_scheme(ColorScheme::Dark)
/// )
/// ```
#[derive(Default)]
pub struct ThemeProvider<V> {
    content: V,
    theme: Theme,
}

impl<V> ThemeProvider<V> {
    /// Creates a new theme provider that wraps the given content.
    #[must_use]
    pub fn new(content: V, theme: Theme) -> Self {
        Self { content, theme }
    }
}

impl<V: View> View for ThemeProvider<V> {
    fn body(self, env: &Environment) -> impl View {
        let mut themed_env = env.clone();
        self.theme.install(&mut themed_env);
        WithEnv::new(self.content, themed_env)
    }
}
