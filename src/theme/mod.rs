//! Theme management built on the resolver/environment pattern.
//!
//! A theme is a bundle of [`Color`](crate::color::Color) and
//! [`Font`](crate::text::font::Font) tokens. Tokens are resolvable, so widgets
//! can grab them just like the built-in typography styles. `ThemeProvider`
//! installs a [`Theme`] into the [`Environment`](crate::Environment) and
//! registers every token override as a signal, keeping downstream views in sync.
use core::marker::PhantomData;

use nami::Computed;
use waterui_core::{
    Environment,
    env::{Store, WithEnv},
};

use crate::{
    View,
    color::{Color, ResolvedColor},
    text::font::{Body, Caption, Font, Headline, ResolvedFont, Subheadline, Title},
};

/// Top-level theme data grouped into colors and typography.
#[derive(Debug, Clone)]
pub struct Theme {
    colors: ThemeColors,
    typography: ThemeTypography,
}

impl Theme {
    /// Creates a new theme from color and typography groups.
    #[must_use]
    pub const fn new(colors: ThemeColors, typography: ThemeTypography) -> Self {
        Self { colors, typography }
    }

    /// Returns the light WaterUI theme.
    #[must_use]
    pub fn light() -> Self {
        Self::new(ThemeColors::light(), ThemeTypography::system())
    }

    /// Returns the dark WaterUI theme.
    #[must_use]
    pub fn dark() -> Self {
        Self::new(ThemeColors::dark(), ThemeTypography::system())
    }

    /// Accesses the color palette.
    #[must_use]
    pub const fn colors(&self) -> &ThemeColors {
        &self.colors
    }

    /// Accesses the typography tokens.
    #[must_use]
    pub const fn typography(&self) -> &ThemeTypography {
        &self.typography
    }

    /// Obtains mutable access to the color palette.
    pub fn colors_mut(&mut self) -> &mut ThemeColors {
        &mut self.colors
    }

    /// Obtains mutable access to the typography tokens.
    pub fn typography_mut(&mut self) -> &mut ThemeTypography {
        &mut self.typography
    }

    /// Applies a [`ThemeLayer`] override.
    #[must_use]
    pub fn layer(mut self, overrides: ThemeLayer) -> Self {
        if let Some(colors) = overrides.colors {
            self.colors = colors;
        }
        if let Some(typography) = overrides.typography {
            self.typography = typography;
        }
        self
    }

    pub fn install(self, env: &mut Environment) {
        todo!()
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::light()
    }
}

/// Color palette used by [`Theme`].
#[derive(Debug, Clone)]
pub struct ThemeColors {
    background: Color,
    surface: Color,
    surface_variant: Color,
    border: Color,
    foreground: Color,
    muted_foreground: Color,
    accent: Color,
    accent_foreground: Color,
}

impl ThemeColors {
    /// Creates a palette from explicit tokens.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        background: Color,
        surface: Color,
        surface_variant: Color,
        border: Color,
        foreground: Color,
        muted_foreground: Color,
        accent: Color,
        accent_foreground: Color,
    ) -> Self {
        Self {
            background,
            surface,
            surface_variant,
            border,
            foreground,
            muted_foreground,
            accent,
            accent_foreground,
        }
    }

    /// Palette optimized for light surfaces.
    #[must_use]
    pub fn light() -> Self {
        Self::new(
            Color::srgb_u32(0xF8FAFC),
            Color::srgb_u32(0xFFFFFF),
            Color::srgb_u32(0xE2E8F0),
            Color::srgb_u32(0xCBD5F5),
            Color::srgb_u32(0x0F172A),
            Color::srgb_u32(0x475569),
            Color::srgb_u32(0x2563EB),
            Color::srgb_u32(0xF8FAFC),
        )
    }

    /// Palette optimized for dark surfaces.
    #[must_use]
    pub fn dark() -> Self {
        Self::new(
            Color::srgb_u32(0x020617),
            Color::srgb_u32(0x0F172A),
            Color::srgb_u32(0x1E293B),
            Color::srgb_u32(0x334155),
            Color::srgb_u32(0xF8FAFC),
            Color::srgb_u32(0xCBD5F5),
            Color::srgb_u32(0x38BDF8),
            Color::srgb_u32(0x020617),
        )
    }

    #[must_use]
    pub const fn background(&self) -> &Color {
        &self.background
    }

    #[must_use]
    pub const fn surface(&self) -> &Color {
        &self.surface
    }

    #[must_use]
    pub const fn surface_variant(&self) -> &Color {
        &self.surface_variant
    }

    #[must_use]
    pub const fn border(&self) -> &Color {
        &self.border
    }

    #[must_use]
    pub const fn foreground(&self) -> &Color {
        &self.foreground
    }

    #[must_use]
    pub const fn muted_foreground(&self) -> &Color {
        &self.muted_foreground
    }

    #[must_use]
    pub const fn accent(&self) -> &Color {
        &self.accent
    }

    #[must_use]
    pub const fn accent_foreground(&self) -> &Color {
        &self.accent_foreground
    }
}

/// Typography palette used by [`Theme`].
#[derive(Debug, Clone)]
pub struct ThemeTypography {
    body: Font,
    title: Font,
    headline: Font,
    subheadline: Font,
    caption: Font,
}

impl ThemeTypography {
    /// Creates a typography set from explicit fonts.
    #[must_use]
    pub const fn new(
        body: Font,
        title: Font,
        headline: Font,
        subheadline: Font,
        caption: Font,
    ) -> Self {
        Self {
            body,
            title,
            headline,
            subheadline,
            caption,
        }
    }

    /// Typography backed by the default WaterUI text resolvers.
    #[must_use]
    pub fn system() -> Self {
        Self::new(
            Font::new(Body),
            Font::new(Title),
            Font::new(Headline),
            Font::new(Subheadline),
            Font::new(Caption),
        )
    }

    #[must_use]
    pub const fn body(&self) -> &Font {
        &self.body
    }

    #[must_use]
    pub const fn title(&self) -> &Font {
        &self.title
    }

    #[must_use]
    pub const fn headline(&self) -> &Font {
        &self.headline
    }

    #[must_use]
    pub const fn subheadline(&self) -> &Font {
        &self.subheadline
    }

    #[must_use]
    pub const fn caption(&self) -> &Font {
        &self.caption
    }
}

/// Partial theme overrides.
#[derive(Debug, Clone, Default)]
pub struct ThemeLayer {
    colors: Option<ThemeColors>,
    typography: Option<ThemeTypography>,
}

impl ThemeLayer {
    /// Creates an empty layer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            colors: None,
            typography: None,
        }
    }

    /// Replaces the entire color palette.
    #[must_use]
    pub fn colors(mut self, colors: ThemeColors) -> Self {
        self.colors = Some(colors);
        self
    }

    /// Replaces the entire typography palette.
    #[must_use]
    pub fn typography(mut self, typography: ThemeTypography) -> Self {
        self.typography = Some(typography);
        self
    }
}

impl From<Theme> for ThemeLayer {
    fn from(theme: Theme) -> Self {
        Self {
            colors: Some(theme.colors),
            typography: Some(theme.typography),
        }
    }
}

/// Retrieves the current theme stored in the environment.
#[must_use]
pub fn theme(env: &Environment) -> Theme {
    env.get::<Theme>().cloned().unwrap_or_else(Theme::default)
}

/// Environment tokens where the active colors are stored as signals.
#[derive(Clone)]
struct ThemeColorValue<T> {
    value: Computed<ResolvedColor>,
    _marker: PhantomData<T>,
}

impl<T> ThemeColorValue<T> {
    fn new(value: Computed<ResolvedColor>) -> Self {
        Self {
            value,
            _marker: PhantomData,
        }
    }

    fn computed(&self) -> Computed<ResolvedColor> {
        self.value.clone()
    }
}

fn resolve_color_token<T>(env: &Environment) -> Computed<ResolvedColor>
where
    T: ThemeColorKey,
{
    env.get::<ThemeColorValue<T>>()
        .map(ThemeColorValue::computed)
        .unwrap_or_else(|| T::fallback().resolve(env))
}

/// Marker trait implemented by every color token.
pub trait ThemeColorKey: Copy + 'static {
    /// Extracts this token's color from a palette.
    fn select(colors: &ThemeColors) -> &Color;

    /// Default color when the environment has no overrides yet.
    fn fallback() -> Color {
        let defaults = Theme::default();
        Self::select(defaults.colors()).clone()
    }
}

/// Color token definitions.
pub mod color {
    use super::{Color, Environment, ResolvedColor, ThemeColors};
    use nami::{Signal, impl_constant};
    use waterui_core::resolve::Resolvable;

    macro_rules! define_color_token {
        ($name:ident, $getter:ident, $doc:literal) => {
            #[doc = $doc]
            #[derive(Debug, Clone, Copy, Default)]
            pub struct $name;

            impl super::ThemeColorKey for $name {
                fn select(colors: &ThemeColors) -> &Color {
                    colors.$getter()
                }
            }

            impl Resolvable for $name {
                type Resolved = ResolvedColor;
                fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
                    super::resolve_color_token::<Self>(env)
                }
            }

            impl_constant!($name);
        };
    }

    define_color_token!(Background, background, "Primary background color token.");
    define_color_token!(Surface, surface, "Elevated or card surface color token.");
    define_color_token!(
        SurfaceVariant,
        surface_variant,
        "Variant surface color token used for alternate backgrounds."
    );
    define_color_token!(Border, border, "Divider/border color token.");
    define_color_token!(Foreground, foreground, "Primary text/icon color token.");
    define_color_token!(
        MutedForeground,
        muted_foreground,
        "Dimmed text/icon color token."
    );
    define_color_token!(Accent, accent, "Interactive accent color token.");
    define_color_token!(
        AccentForeground,
        accent_foreground,
        "Foreground color token to pair with [`Accent`]."
    );
}

fn install_color<T>(env: &mut Environment, colors: &ThemeColors, parent: &Environment)
where
    T: ThemeColorKey,
{
    let computed = T::select(colors).clone().resolve(parent);
    env.insert(ThemeColorValue::<T>::new(computed));
}

fn install_typography<T>(env: &mut Environment, font: &Font, parent: &Environment)
where
    T: 'static,
{
    let resolved = font.clone().resolve(parent);
    env.insert(Store::<T, Computed<ResolvedFont>>::new(resolved));
}

/// Installs an explicit resolved color signal for the given token.
pub fn install_color_signal_for<T>(env: &mut Environment, value: Computed<ResolvedColor>)
where
    T: ThemeColorKey,
{
    env.insert(ThemeColorValue::<T>::new(value));
}

/// Installs an explicit resolved typography signal for the given token.
pub fn install_typography_signal_for<T>(env: &mut Environment, value: Computed<ResolvedFont>)
where
    T: 'static,
{
    env.insert(Store::<T, Computed<ResolvedFont>>::new(value));
}

/// Provides a theme to child views.
#[derive(Debug, Clone)]
pub struct ThemeProvider<V> {
    content: V,
    layer: ThemeLayer,
}

impl<V> ThemeProvider<V> {
    /// Wraps content with a theme layer.
    #[must_use]
    pub fn new(content: V, layer: impl Into<ThemeLayer>) -> Self {
        Self {
            content,
            layer: layer.into(),
        }
    }
}

impl<V: View> View for ThemeProvider<V> {
    fn body(self, env: &Environment) -> impl View {
        let base = theme(env);
        let applied = base.layer(self.layer);
        let mut themed_env = env.clone();
        themed_env.insert(applied.clone());
        install_color::<color::Background>(&mut themed_env, applied.colors(), env);
        install_color::<color::Surface>(&mut themed_env, applied.colors(), env);
        install_color::<color::SurfaceVariant>(&mut themed_env, applied.colors(), env);
        install_color::<color::Border>(&mut themed_env, applied.colors(), env);
        install_color::<color::Foreground>(&mut themed_env, applied.colors(), env);
        install_color::<color::MutedForeground>(&mut themed_env, applied.colors(), env);
        install_color::<color::Accent>(&mut themed_env, applied.colors(), env);
        install_color::<color::AccentForeground>(&mut themed_env, applied.colors(), env);
        install_typography::<Body>(&mut themed_env, applied.typography().body(), env);
        install_typography::<Title>(&mut themed_env, applied.typography().title(), env);
        install_typography::<Headline>(&mut themed_env, applied.typography().headline(), env);
        install_typography::<Subheadline>(&mut themed_env, applied.typography().subheadline(), env);
        install_typography::<Caption>(&mut themed_env, applied.typography().caption(), env);
        WithEnv::new(self.content, themed_env)
    }
}
