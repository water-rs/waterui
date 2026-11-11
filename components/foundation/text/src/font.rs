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
//! 3. **Direct overrides** — `.size(f32)`, `.weight(..)`, `.italic()`,
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
//!
//! Bold is orthogonal in the same way: `.bold()` asks for the platform's bold
//! face of the font and each backend resolves it, while `.weight(..)` is an
//! exact weight. See [`Font::bold`].
//!
//! Text nobody styled is its own state, not the body slot: [`Font::default`]
//! sizes and spaces it as body text, but a backend sees
//! [`ResolvedFont::unstyled`] and gives it the platform's plain bold where
//! that differs from the body style's bold.

use core::fmt::Debug;

use alloc::string::String;

use nami::{Computed, Signal, SignalExt, impl_constant, signal::IntoSignal};
use waterui_core::{
    Environment, IntoSignalF32, Str,
    resolve::{AnyResolvable, Resolvable},
};

/// Font configuration for text rendering.
///
/// This struct defines all the visual properties that can be applied to text,
/// including size, styling, and decorations.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Font(AnyResolvable<ResolvedFont>);

/// The font of text nobody styled.
///
/// It resolves to the [`Body`] slot's face, size and metrics, marked
/// [`ResolvedFont::unstyled`] so a backend can tell it from text set in the
/// body style.
impl Default for Font {
    fn default() -> Self {
        Self::new(Unstyled)
    }
}

/// The resolvable behind [`Font::default`]: the body slot, marked unstyled.
#[derive(Debug, Clone, Copy)]
struct Unstyled;

impl Resolvable for Unstyled {
    type Resolved = ResolvedFont;

    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Body.resolve(env).map(|mut font| {
            font.unstyled = true;
            font
        })
    }
}

/// A resolved font with specific size, weight, and optional family.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResolvedFont {
    /// Font size in points.
    size: f32,
    /// Font weight.
    weight: FontWeight,
    /// Absolute line height in logical points.
    ///
    /// `None` uses the selected font's preferred metrics.
    line_height: Option<f32>,
    /// Additional spacing between adjacent glyphs in logical points.
    letter_spacing: f32,
    /// Optional font family name (e.g., "MaterialIcons-Regular").
    /// None means use the system default font.
    family: Option<Str>,
    /// The design the face is chosen from when no family is named.
    ///
    /// A named family is exact and wins; the design says which of the
    /// platform's own faces to use in its absence.
    design: FontDesign,
    /// Whether the text asks for the platform's bold face of this font.
    bold: bool,
    /// The weight the platform's bold trait gives this face, when the
    /// theme that published the face declares it.
    bold_weight: Option<FontWeight>,
    /// Whether this is the face of text nobody styled ([`Font::default`]).
    unstyled: bool,
}

impl ResolvedFont {
    /// Font size in points.
    #[must_use]
    pub const fn size(&self) -> f32 {
        self.size
    }

    /// Sets the `size`.
    pub const fn set_size(&mut self, size: f32) {
        self.size = size;
    }

    /// Font weight.
    #[must_use]
    pub const fn weight(&self) -> FontWeight {
        self.weight
    }

    /// Absolute line height in logical points.
    ///
    /// `None` uses the selected font's preferred metrics.
    #[must_use]
    pub const fn line_height(&self) -> Option<f32> {
        self.line_height
    }

    /// Additional spacing between adjacent glyphs in logical points.
    #[must_use]
    pub const fn letter_spacing(&self) -> f32 {
        self.letter_spacing
    }

    /// Optional font family name (e.g., "MaterialIcons-Regular").
    /// None means use the system default font.
    #[must_use]
    pub const fn family(&self) -> Option<&Str> {
        self.family.as_ref()
    }

    /// The design the face is chosen from when no family is named.
    ///
    /// A named family is exact and wins; the design says which of the
    /// platform's own faces to use in its absence.
    #[must_use]
    pub const fn design(&self) -> FontDesign {
        self.design
    }

    /// Whether the text asks for the platform's bold face of this font
    /// ([`Font::bold`]).
    ///
    /// This is a trait the backend resolves, not a weight: it asks for the
    /// face the platform calls bold for this font's own weight, which is not
    /// necessarily [`FontWeight::Bold`]. An explicit
    /// [`FontWeight::Bold`] set through [`Font::weight`] is always weight 700.
    #[must_use]
    pub const fn bold(&self) -> bool {
        self.bold
    }

    /// The weight the platform's bold trait gives this face, when the theme
    /// that published the face declares it.
    ///
    /// A backend theme declares it for its slot faces, whose bold face the
    /// platform defines per text style. `None` leaves the trait to the
    /// backend's rule for a face it realizes directly. [`Font::size`],
    /// [`Font::weight`] and a named [`Font::family`] select another face, so
    /// they drop the declaration.
    #[must_use]
    pub const fn bold_weight(&self) -> Option<FontWeight> {
        self.bold_weight
    }

    /// Whether this is the face of text nobody styled ([`Font::default`]).
    ///
    /// An unstyled font resolves to the [`Body`] slot's face, size, metrics
    /// and declared [`Self::bold_weight`], so it lays out as body text. The
    /// mark lets a backend whose platform bolds unstyled text differently
    /// from its body text style apply its plain bold under the
    /// [`Self::bold`] trait; a backend without that distinction treats it as
    /// body. Choosing a slot with [`Font::new`], or another face with
    /// [`Font::size`], [`Font::weight`] or a named [`Font::family`], styles
    /// the text and clears the mark.
    #[must_use]
    pub const fn unstyled(&self) -> bool {
        self.unstyled
    }

    /// Drops what the previous face declared about its bold counterpart, for
    /// a modifier that selects another face.
    const fn select_face(&mut self) {
        self.bold_weight = None;
        self.unstyled = false;
    }
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
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each font design variant by variant; a new font design must fail their build instead of falling into a wildcard arm"
)]
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
            bold: false,
            bold_weight: None,
            unstyled: false,
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
            bold: false,
            bold_weight: None,
            unstyled: false,
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
            bold: false,
            bold_weight: None,
            unstyled: false,
        }
    }

    /// Sets the design the face is chosen from.
    #[must_use]
    pub const fn with_design(mut self, design: FontDesign) -> Self {
        self.design = design;
        self
    }

    /// Declares the weight the platform's bold trait gives this face.
    ///
    /// For a theme publishing a slot face whose bold counterpart the
    /// platform defines; see [`Self::bold_weight`].
    #[must_use]
    pub const fn with_bold_weight(mut self, weight: FontWeight) -> Self {
        self.bold_weight = Some(weight);
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
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: the nine CSS weights from 100 to 900"
)]
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

/// A family override applied by [`Font::family`].
///
/// A named family is exact and replaces whatever family the font resolved
/// to. [`FontFamily::Inherited`] names no family, so the font keeps the one
/// its slot resolves to — the value for a setting the user left unset.
///
/// A family signal converts from the value types a setting holds: a string
/// literal, a [`Str`], a `String`, or an `Option<Str>` whose `None` is
/// [`FontFamily::Inherited`]. `Binding<Option<Str>>` therefore passes
/// straight to [`Font::family`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum FontFamily {
    /// Keep the family the font already resolves to.
    #[default]
    Inherited,
    /// Use this family.
    Named(Str),
}

impl_constant!(FontFamily);

impl From<Str> for FontFamily {
    fn from(family: Str) -> Self {
        Self::Named(family)
    }
}

impl From<&'static str> for FontFamily {
    fn from(family: &'static str) -> Self {
        Self::Named(Str::from_static(family))
    }
}

impl From<String> for FontFamily {
    fn from(family: String) -> Self {
        Self::Named(family.into())
    }
}

impl From<Option<Str>> for FontFamily {
    fn from(family: Option<Str>) -> Self {
        family.map_or(Self::Inherited, Self::Named)
    }
}

/// A font whose resolved value is rewritten by the current value of a signal.
///
/// Every [`Font`] modifier is one of these: the modifier's value signal is
/// zipped with the inner font's resolved signal, so a change to either
/// re-resolves the font in place and no view is rebuilt.
#[derive(Clone)]
struct Modified<T: 'static> {
    font: Font,
    value: Computed<T>,
    apply: fn(&mut ResolvedFont, T),
}

impl<T: 'static> Debug for Modified<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Modified")
            .field("font", &self.font)
            .finish_non_exhaustive()
    }
}

impl<T: Clone + 'static> Resolvable for Modified<T> {
    type Resolved = ResolvedFont;

    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
        let apply = self.apply;
        self.font
            .resolve(env)
            .zip(&self.value)
            .map(move |(mut font, value)| {
                apply(&mut font, value);
                font
            })
    }
}

impl Font {
    /// Creates a new font from a resolvable value.
    pub fn new(font: impl Resolvable<Resolved = ResolvedFont> + 'static) -> Self {
        Self(AnyResolvable::new(font))
    }

    fn modified<T: Clone + 'static>(
        self,
        value: Computed<T>,
        apply: fn(&mut ResolvedFont, T),
    ) -> Self {
        Self::new(Modified {
            font: self,
            value,
            apply,
        })
    }

    /// Sets the font weight.
    ///
    /// Accepts a static [`FontWeight`] or any signal of one.
    ///
    /// The weight is exact: `weight(FontWeight::Bold)` is weight 700 on
    /// every backend, unlike [`Self::bold`]. A weight selects another face,
    /// so the slot's declared [`ResolvedFont::bold_weight`] is dropped and
    /// the font is no longer [`ResolvedFont::unstyled`]; a [`Self::bold`]
    /// trait stays and applies to the new weight.
    #[must_use]
    pub fn weight(self, weight: impl IntoSignal<FontWeight> + 'static) -> Self {
        self.modified(weight.into_signal().computed(), |font, weight| {
            font.weight = weight;
            font.select_face();
        })
    }

    /// Sets the font size in points.
    ///
    /// Accepts a number or any numeric signal.
    ///
    /// A size selects a new face, so the typography metrics declared for
    /// the previous size are dropped: the resized font uses the new face's
    /// preferred line height and no extra letter spacing, the way a platform
    /// text style resized by hand does. A theme slot publishes its own face's
    /// pitch as an absolute line height; carrying that pitch onto a smaller
    /// face would space its lines as the larger one. Call [`Self::line_height`]
    /// or [`Self::letter_spacing`] after `size` to declare metrics for the
    /// new face. The slot's declared [`ResolvedFont::bold_weight`] and the
    /// [`ResolvedFont::unstyled`] mark are dropped for the same reason: a
    /// [`Self::bold`] trait on the resized font resolves as the platform
    /// bolds a fixed-size face.
    #[must_use]
    pub fn size(self, size: impl IntoSignalF32) -> Self {
        self.modified(size.into_signal_f32().computed(), |font, size| {
            font.size = size;
            font.line_height = None;
            font.letter_spacing = 0.0;
            font.select_face();
        })
    }

    /// Sets the font family.
    ///
    /// Accepts a family name or any signal of a [`FontFamily`], including
    /// `Binding<Option<Str>>`: while the signal holds
    /// [`FontFamily::Inherited`] (`None`), the font keeps the family its slot
    /// resolves to. A change of the signal re-resolves the font in place.
    /// A named family replaces the slot's face, so it drops the slot's
    /// declared [`ResolvedFont::bold_weight`] and the
    /// [`ResolvedFont::unstyled`] mark.
    #[must_use]
    pub fn family(self, family: impl IntoSignal<FontFamily> + 'static) -> Self {
        self.modified(family.into_signal().computed(), |font, family| {
            if let FontFamily::Named(family) = family {
                font.family = Some(family);
                font.select_face();
            }
        })
    }

    /// Sets the design the face is chosen from, keeping the slot, size and
    /// weight.
    ///
    /// Accepts a static [`FontDesign`] or any signal of one.
    #[must_use]
    pub fn design(self, design: impl IntoSignal<FontDesign> + 'static) -> Self {
        self.modified(design.into_signal().computed(), |font, design| {
            font.design = design;
        })
    }

    /// Asks for the platform's fixed-pitch face.
    /// Equal to calling `font.design(FontDesign::Monospaced)`.
    #[must_use]
    pub fn monospaced(self) -> Self {
        self.design(FontDesign::Monospaced)
    }

    /// Sets an absolute line height in logical points.
    ///
    /// Accepts a number or any numeric signal.
    #[must_use]
    pub fn line_height(self, line_height: impl IntoSignalF32) -> Self {
        self.modified(
            line_height.into_signal_f32().computed(),
            |font, line_height| font.line_height = Some(line_height),
        )
    }

    /// Sets additional spacing between adjacent glyphs in logical points.
    ///
    /// Accepts a number or any numeric signal.
    #[must_use]
    pub fn letter_spacing(self, letter_spacing: impl IntoSignalF32) -> Self {
        self.modified(
            letter_spacing.into_signal_f32().computed(),
            |font, letter_spacing| font.letter_spacing = letter_spacing,
        )
    }

    /// Asks for the platform's bold face of this font.
    ///
    /// Bold is a trait each backend resolves from its platform's own
    /// definition, not a weight: on Apple a text-style font gets its style's
    /// bold counterpart (Semibold for body text) and a fixed-size font the
    /// bold symbolic trait; on Android the bold typeface style adds 300 to
    /// the font's weight. Unstyled text ([`Font::default`]) takes the
    /// platform's plain bold, which is not necessarily its body style's: on
    /// macOS it is Bold, while `.body()` text bolds to Semibold. Use
    /// [`Self::weight`] with [`FontWeight::Bold`] for an exact weight of 700.
    /// The trait is independent of the weight, so it applies to whatever
    /// weight the font has, in either order.
    #[must_use]
    pub fn bold(self) -> Self {
        self.modified(Computed::constant(()), |font, ()| font.bold = true)
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
    use alloc::{rc::Rc, vec::Vec};
    use core::cell::RefCell;

    use nami::Binding;

    use super::*;

    fn env_with_body(font: ResolvedFont) -> Environment {
        Environment::new().store::<Body, Computed<ResolvedFont>>(Computed::constant(font))
    }

    #[test]
    fn size_drops_the_previous_face_typography_metrics() {
        let slot = ResolvedFont::new(17.0, FontWeight::Normal).with_typography_metrics(22.0, 0.4);
        let env = env_with_body(slot);

        let resized = Font::new(Body).size(14.0).resolve(&env).snapshot();
        assert!((resized.size - 14.0).abs() < f32::EPSILON);
        assert_eq!(resized.line_height, None);
        assert!(resized.letter_spacing.abs() < f32::EPSILON);

        let declared = Font::new(Body)
            .size(14.0)
            .line_height(18.0)
            .resolve(&env)
            .snapshot();
        assert_eq!(declared.line_height, Some(18.0));
    }

    #[test]
    fn bold_is_a_trait_distinct_from_weight_700() {
        let slot =
            ResolvedFont::new(17.0, FontWeight::Normal).with_bold_weight(FontWeight::SemiBold);
        let env = env_with_body(slot);

        let bold = Font::new(Body).bold().resolve(&env).snapshot();
        assert!(bold.bold());
        assert_eq!(bold.weight(), FontWeight::Normal);
        assert_eq!(bold.bold_weight(), Some(FontWeight::SemiBold));

        let weighted = Font::new(Body)
            .weight(FontWeight::Bold)
            .resolve(&env)
            .snapshot();
        assert!(!weighted.bold());
        assert_eq!(weighted.weight(), FontWeight::Bold);
    }

    /// Unstyled text lays out exactly as the body slot but stays
    /// distinguishable from text set in the body style, until a modifier
    /// picks another face.
    #[test]
    fn unstyled_resolves_as_body_and_is_distinguishable_from_it() {
        let slot = ResolvedFont::new(17.0, FontWeight::Normal)
            .with_typography_metrics(22.0, 0.4)
            .with_bold_weight(FontWeight::SemiBold);
        let env = env_with_body(slot);

        let unstyled = Font::default().bold().resolve(&env).snapshot();
        let body = Font::new(Body).bold().resolve(&env).snapshot();
        assert!(unstyled.unstyled());
        assert!(!body.unstyled());
        for font in [&unstyled, &body] {
            assert!((font.size() - 17.0).abs() < f32::EPSILON);
            assert_eq!(font.weight(), FontWeight::Normal);
            assert_eq!(font.line_height(), Some(22.0));
            assert!((font.letter_spacing() - 0.4).abs() < f32::EPSILON);
            assert_eq!(font.family(), None);
            assert_eq!(font.design(), FontDesign::Default);
            assert!(font.bold());
            assert_eq!(font.bold_weight(), Some(FontWeight::SemiBold));
        }

        let kept = Font::default()
            .monospaced()
            .line_height(20.0)
            .letter_spacing(0.0)
            .family(FontFamily::Inherited)
            .resolve(&env)
            .snapshot();
        assert!(kept.unstyled());

        for font in [
            Font::default().size(20.0),
            Font::default().weight(FontWeight::Light),
            Font::default().family("Menlo"),
        ] {
            assert!(!font.resolve(&env).snapshot().unstyled());
        }
    }

    #[test]
    fn a_new_face_drops_the_declared_bold_weight_and_keeps_the_trait() {
        let slot =
            ResolvedFont::new(17.0, FontWeight::Normal).with_bold_weight(FontWeight::SemiBold);
        let env = env_with_body(slot);

        for font in [
            Font::new(Body).bold().size(20.0),
            Font::new(Body).size(20.0).bold(),
            Font::new(Body).bold().weight(FontWeight::Light),
            Font::new(Body).weight(FontWeight::Light).bold(),
            Font::new(Body).bold().family("Menlo"),
        ] {
            let resolved = font.resolve(&env).snapshot();
            assert!(resolved.bold());
            assert_eq!(resolved.bold_weight(), None);
        }

        let inherited = Font::new(Body)
            .bold()
            .family(FontFamily::Inherited)
            .monospaced()
            .resolve(&env)
            .snapshot();
        assert_eq!(inherited.bold_weight(), Some(FontWeight::SemiBold));
    }

    #[test]
    fn family_follows_its_signal_and_inherits_the_slot_family_while_unset() {
        let env = env_with_body(ResolvedFont::with_static_family(
            16.0,
            FontWeight::Normal,
            "Inter",
        ));
        let setting: Binding<Option<Str>> = Binding::container(None);
        let resolved = Font::new(Body).family(setting.clone()).resolve(&env);

        let observed = Rc::new(RefCell::new(Vec::new()));
        let _guard = resolved.watch({
            let observed = observed.clone();
            move |context| observed.borrow_mut().push(context.into_value().family)
        });

        assert_eq!(resolved.snapshot().family.as_deref(), Some("Inter"));

        setting.set(Some(Str::from_static("Menlo")));
        assert_eq!(resolved.snapshot().family.as_deref(), Some("Menlo"));

        setting.set(None);
        assert_eq!(resolved.snapshot().family.as_deref(), Some("Inter"));

        assert_eq!(
            *observed.borrow(),
            [
                Some(Str::from_static("Menlo")),
                Some(Str::from_static("Inter"))
            ]
        );
    }
}
