use core::fmt::Debug;

use alloc::boxed::Box;
use nami::{Computed, Signal, impl_constant};
use waterui_core::{
    Environment,
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

/// A resolved font with specific size and weight.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResolvedFont {
    /// Font size in points.
    pub size: f32,
    /// Font weight.
    pub weight: FontWeight,
}

impl ResolvedFont {
    /// Creates a new resolved font with the given size and weight.
    #[must_use]
    pub const fn new(size: f32, weight: FontWeight) -> Self {
        Self { size, weight }
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

/// A trait for custom font types that can be resolved.
pub trait CustomFont: Debug + Clone {
    /// Resolves the font in the given environment.
    fn resolve(&self, env: &Environment) -> ResolvedFont;
}


trait CustomFontImpl {
    fn resolve(&self, env: &Environment) -> ResolvedFont;
    fn box_clone(&self) -> Box<dyn CustomFontImpl>;
}

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
        }))
    }

    /// Sets the font size in points.
    #[must_use]
    pub fn size(self, size: f32) -> Self {
        Self::new(resolve::Map::new(self.0, move |font| ResolvedFont {
            size,
            weight: font.weight,
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
    ($name:ident,$default_size:expr,$default_weight:expr, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy)]
        pub struct $name;

        impl Resolvable for $name {
            type Resolved = ResolvedFont;
            fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
                env.query::<Self, Computed<Self::Resolved>>()
                    .cloned()
                    .unwrap_or_else(|| {
                        Computed::constant(ResolvedFont::new($default_size, $default_weight))
                    })
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
impl_font!(Body, 16.0, FontWeight::Normal, "Body font style.");
impl_font!(Title, 24.0, FontWeight::SemiBold, "Title font style.");
impl_font!(Headline, 32.0, FontWeight::Bold, "Headline font style.");
impl_font!(
    Subheadline,
    20.0,
    FontWeight::SemiBold,
    "Subheadline font style."
);
impl_font!(Caption, 12.0, FontWeight::Normal, "Caption font style.");
impl_font!(Footnote, 10.0, FontWeight::Light, "Footnote font style.");
