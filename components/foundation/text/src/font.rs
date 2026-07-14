//! Font configuration and the two-layer typography system.
//!
//! `WaterUI` text styling exposes three orthogonal entry points; each is
//! kept on purpose because they speak to different concerns. Pick the
//! highest-level one that fits the use case:
//!
//! 1. **Semantic categories** — `.title()`, `.headline()`, `.sub_headline()`,
//!    `.body()`, `.caption()`. These resolve through the active `Theme` and
//!    pick up accessibility text-size scaling automatically. **Prefer these
//!    for product UI.** They guarantee that a "Settings → Larger Text" user
//!    setting cascades into your screen without per-call ceremony.
//!
//! 2. **Font slots** — `.font(Title)`, `.font(font::Caption)`. Same
//!    semantic categories as (1), but expressed as a value you can hold,
//!    pass around, or compose. Prefer this when the choice is computed or
//!    needs to be parameterized; the `.title()` family is shorthand for
//!    these.
//!
//! 3. **Direct overrides** — `.size(f32)`, plus `.bold()`, `.italic()`,
//!    [`Font`](crate::font::Font) constructors with explicit size/weight/family. This is the
//!    escape hatch for fixed layouts (posters, splash screens, hero
//!    headlines) and for example/demo code that wants to demonstrate a
//!    specific visual. Direct overrides ignore Theme-driven scaling.
//!
//! These layers coexist by design: layers 1 and 2 are sugar over the
//! Theme-resolved font slots, layer 3 escapes that resolution entirely.
//! Do not collapse them — readability of UI code beats minimalism of API
//! surface.

use core::fmt::Debug;

use nami::{Computed, Signal, impl_constant};
use waterui_core::{
    Environment, Str,
    resolve::{self, AnyResolvable, Resolvable},
};

/// Font configuration for text rendering.
///
/// This struct defines all the visual properties that can be applied to text,
/// including size, styling, and decorations.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Font(AnyResolvable<ResolvedFont>);

impl Default for Font {
    fn default() -> Self {
        Self::new(Body)
    }
}

/// A resolved font with specific size, weight, and optional family.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResolvedFont {
    /// Font size in points.
    pub size: f32,
    /// Font weight.
    pub weight: FontWeight,
    /// Optional font family name (e.g., "MaterialIcons-Regular").
    /// None means use the system default font.
    pub family: Option<Str>,
}

impl ResolvedFont {
    /// Creates a new resolved font with the given size and weight.
    /// Uses the system default font family.
    #[must_use]
    pub const fn new(size: f32, weight: FontWeight) -> Self {
        Self {
            size,
            weight,
            family: None,
        }
    }

    /// Creates a new resolved font with a specific font family.
    #[must_use]
    pub fn with_family(size: f32, weight: FontWeight, family: impl Into<Str>) -> Self {
        Self {
            size,
            weight,
            family: Some(family.into()),
        }
    }

    /// Creates a new resolved font with a static font family (const-compatible).
    #[must_use]
    pub const fn with_static_family(size: f32, weight: FontWeight, family: &'static str) -> Self {
        Self {
            size,
            weight,
            family: Some(Str::from_static(family)),
        }
    }
}

/// Font weight enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontWeight {
    /// Thin weight (100).
    Thin,
    /// Ultra-light weight (200).
    UltraLight,
    /// Light weight (300).
    Light,
    /// Normal weight (400).
    #[default]
    Normal,
    /// Medium weight (500).
    Medium,
    /// Semi-bold weight (600).
    SemiBold,
    /// Bold weight (700).
    Bold,
    /// Ultra-bold weight (800).
    UltraBold,
    /// Black weight (900).
    Black,
}

impl_constant!(Font, ResolvedFont, FontWeight);

impl Font {
    /// Creates a new font from a resolvable value.
    pub fn new(font: impl Resolvable<Resolved = ResolvedFont> + 'static) -> Self {
        Self(AnyResolvable::new(font))
    }

    /// Sets the font weight.
    #[must_use]
    pub fn weight(self, weight: FontWeight) -> Self {
        Self::new(resolve::Map::new(self.0, move |font| ResolvedFont {
            size: font.size,
            weight,
            family: font.family,
        }))
    }

    /// Sets the font size in points.
    #[must_use]
    pub fn size(self, size: f32) -> Self {
        Self::new(resolve::Map::new(self.0, move |font| ResolvedFont {
            size,
            weight: font.weight,
            family: font.family,
        }))
    }

    /// Sets the font family.
    #[must_use]
    pub fn family(self, family: impl Into<Str> + Clone + 'static) -> Self {
        Self::new(resolve::Map::new(self.0, move |font| ResolvedFont {
            size: font.size,
            weight: font.weight,
            family: Some(family.clone().into()),
        }))
    }

    /// Sets the font to bold weight.
    /// Equal to calling `font.weight(FontWeight::Bold)`.
    #[must_use]
    pub fn bold(self) -> Self {
        self.weight(FontWeight::Bold)
    }

    /// Resolves the font in the given environment.
    #[must_use]
    pub fn resolve(&self, env: &Environment) -> Computed<ResolvedFont> {
        self.0.resolve(env)
    }
}

macro_rules! impl_font {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy)]
        pub struct $name;

        impl Resolvable for $name {
            type Resolved = ResolvedFont;
            fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
                env.query::<Self, Computed<Self::Resolved>>()
                    .cloned()
                    .expect(concat!(
                        stringify!($name),
                        " font token is not installed in the environment"
                    ))
            }
        }

        impl From<$name> for Font {
            fn from(value: $name) -> Self {
                Self::new(value)
            }
        }

        impl_constant!($name);
    };
}
impl_font!(Body, "Body font style.");
impl_font!(Title, "Title font style.");
impl_font!(Headline, "Headline font style.");
impl_font!(Subheadline, "Subheadline font style.");
impl_font!(Caption, "Caption font style.");
impl_font!(Footnote, "Footnote font style.");
