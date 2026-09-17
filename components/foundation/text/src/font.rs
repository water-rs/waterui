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
//!
//! Orthogonal to all three is the [`FontDesign`]: `.monospaced()` keeps a
//! font's slot, size and weight and only asks for the platform's monospaced
//! face, the way a design does in `SwiftUI`. It is not a family name — the
//! backend picks the face — and not a slot, so `Font::from(Caption)
//! .monospaced()` is as expressible as the body-sized code block.

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
    /// Absolute line height in logical points.
    ///
    /// `None` uses the selected font's preferred metrics.
    pub line_height: Option<f32>,
    /// Additional spacing between adjacent glyphs in logical points.
    pub letter_spacing: f32,
    /// Optional font family name (e.g., "MaterialIcons-Regular").
    /// None means use the system default font.
    pub family: Option<Str>,
    /// The design the face is chosen from when no family is named.
    ///
    /// A named family is exact and wins; the design says which of the
    /// platform's own faces to use in its absence.
    pub design: FontDesign,
}

/// A semantic choice of typeface, resolved by each backend to a platform face.
///
/// This is the axis `SwiftUI` calls a font design and Material a generic font
/// family: it names what the text is for, not a font. Every backend maps it
/// to its own face — the system monospaced font on Apple platforms, the
/// monospace typeface on Android, the fontconfig or parley generic elsewhere —
/// so a code block reads in a fixed-pitch face on every platform without an
/// application naming one.
///
/// Exhaustive on purpose: every backend maps each design to a face, so a new
/// design is a change every backend has to answer, and the compiler says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontDesign {
    /// The platform's default proportional face.
    #[default]
    Default,
    /// The platform's fixed-pitch face, for code and aligned columns.
    Monospaced,
}

impl ResolvedFont {
    /// Creates a new resolved font with the given size and weight.
    /// Uses the system default font family.
    #[must_use]
    pub const fn new(size: f32, weight: FontWeight) -> Self {
        Self {
            size,
            weight,
            line_height: None,
            letter_spacing: 0.0,
            family: None,
            design: FontDesign::Default,
        }
    }

    /// Creates a new resolved font with a specific font family.
    #[must_use]
    pub fn with_family(size: f32, weight: FontWeight, family: impl Into<Str>) -> Self {
        Self {
            size,
            weight,
            line_height: None,
            letter_spacing: 0.0,
            family: Some(family.into()),
            design: FontDesign::Default,
        }
    }

    /// Creates a new resolved font with a static font family (const-compatible).
    #[must_use]
    pub const fn with_static_family(size: f32, weight: FontWeight, family: &'static str) -> Self {
        Self {
            size,
            weight,
            line_height: None,
            letter_spacing: 0.0,
            family: Some(Str::from_static(family)),
            design: FontDesign::Default,
        }
    }

    /// Sets the design the face is chosen from.
    #[must_use]
    pub const fn with_design(mut self, design: FontDesign) -> Self {
        self.design = design;
        self
    }

    /// Sets absolute line height and letter spacing.
    #[must_use]
    pub const fn with_typography_metrics(mut self, line_height: f32, letter_spacing: f32) -> Self {
        self.line_height = Some(line_height);
        self.letter_spacing = letter_spacing;
        self
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

impl_constant!(Font, ResolvedFont, FontWeight, FontDesign);

impl Font {
    /// Creates a new font from a resolvable value.
    pub fn new(font: impl Resolvable<Resolved = ResolvedFont> + 'static) -> Self {
        Self(AnyResolvable::new(font))
    }

    /// Sets the font weight.
    #[must_use]
    pub fn weight(self, weight: FontWeight) -> Self {
        Self::new(resolve::Map::new(self.0, move |mut font| {
            font.weight = weight;
            font
        }))
    }

    /// Sets the font size in points.
    ///
    /// A size selects a new face, so the typography metrics declared for
    /// the previous size are dropped: the resized font uses the new face's
    /// preferred line height and no extra letter spacing, the way a platform
    /// text style resized by hand does. A theme slot publishes its own face's
    /// pitch as an absolute line height; carrying that pitch onto a smaller
    /// face would space its lines as the larger one. Call [`Self::line_height`]
    /// or [`Self::letter_spacing`] after `size` to declare metrics for the
    /// new face.
    #[must_use]
    pub fn size(self, size: f32) -> Self {
        Self::new(resolve::Map::new(self.0, move |mut font| {
            font.size = size;
            font.line_height = None;
            font.letter_spacing = 0.0;
            font
        }))
    }

    /// Sets the font family.
    #[must_use]
    pub fn family(self, family: impl Into<Str> + Clone + 'static) -> Self {
        Self::new(resolve::Map::new(self.0, move |mut font| {
            font.family = Some(family.clone().into());
            font
        }))
    }

    /// Sets the design the face is chosen from, keeping the slot, size and
    /// weight.
    #[must_use]
    pub fn design(self, design: FontDesign) -> Self {
        Self::new(resolve::Map::new(self.0, move |mut font| {
            font.design = design;
            font
        }))
    }

    /// Asks for the platform's fixed-pitch face.
    /// Equal to calling `font.design(FontDesign::Monospaced)`.
    #[must_use]
    pub fn monospaced(self) -> Self {
        self.design(FontDesign::Monospaced)
    }

    /// Sets an absolute line height in logical points.
    #[must_use]
    pub fn line_height(self, line_height: f32) -> Self {
        Self::new(resolve::Map::new(self.0, move |mut font| {
            font.line_height = Some(line_height);
            font
        }))
    }

    /// Sets additional spacing between adjacent glyphs in logical points.
    #[must_use]
    pub fn letter_spacing(self, letter_spacing: f32) -> Self {
        Self::new(resolve::Map::new(self.0, move |mut font| {
            font.letter_spacing = letter_spacing;
            font
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

/// A semantic font slot in the theme's type scale.
///
/// Implemented by each font slot type (`Body`, `Title`, `Headline`,
/// `Subheadline`, `Caption`, `Footnote`). The slot's [`FontSlot::DEFAULT`]
/// constants are the framework-wide default type scale every backend installs
/// when the application theme leaves the slot unset.
pub trait FontSlot {
    /// The default font for this slot.
    ///
    /// Together these constants form the framework-wide default type scale
    /// every backend installs when the application theme leaves the slot
    /// unset.
    const DEFAULT: ResolvedFont;
}

macro_rules! impl_font {
    ($name:ident, $doc:expr, $size:expr, $weight:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy)]
        pub struct $name;

        impl FontSlot for $name {
            const DEFAULT: ResolvedFont = ResolvedFont::new($size, $weight);
        }

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
impl_font!(Body, "Body font style.", 16.0, FontWeight::Normal);
impl_font!(Title, "Title font style.", 22.0, FontWeight::Normal);
impl_font!(Headline, "Headline font style.", 24.0, FontWeight::Normal);
impl_font!(
    Subheadline,
    "Subheadline font style.",
    16.0,
    FontWeight::Medium
);
impl_font!(Caption, "Caption font style.", 12.0, FontWeight::Normal);
impl_font!(Footnote, "Footnote font style.", 11.0, FontWeight::Medium);

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with_body(font: ResolvedFont) -> Environment {
        Environment::new().store::<Body, Computed<ResolvedFont>>(Computed::constant(font))
    }

    #[test]
    fn size_drops_the_previous_face_typography_metrics() {
        let slot = ResolvedFont::new(17.0, FontWeight::Normal).with_typography_metrics(22.0, 0.4);
        let env = env_with_body(slot);

        let resized = Font::new(Body).size(14.0).resolve(&env).get();
        assert!((resized.size - 14.0).abs() < f32::EPSILON);
        assert_eq!(resized.line_height, None);
        assert!(resized.letter_spacing.abs() < f32::EPSILON);

        let declared = Font::new(Body)
            .size(14.0)
            .line_height(18.0)
            .resolve(&env)
            .get();
        assert_eq!(declared.line_height, Some(18.0));
    }
}
