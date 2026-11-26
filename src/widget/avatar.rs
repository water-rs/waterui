//! A small portrait standing in for a person or an entity.
//!
//! An avatar is an image clipped to a shape, with something legible to show
//! when there is no image: the initials of a name, or a view the caller
//! supplies. Backends realize [`AvatarConfig`] directly with platform-native or
//! theme-driven styling.

use alloc::string::String;
use core::fmt;

#[cfg(feature = "image")]
use crate::Url;
use crate::metadata::secure::{DynamicRange, DynamicRangeMetadata};
use crate::prelude::*;
use nami::signal::{IntoComputed, IntoSignal};
use unicode_segmentation::UnicodeSegmentation as _;
use waterui_controls::{IntoLabel, Label, SemanticLabel};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::handler::ViewBuilder;
use waterui_core::metadata::Metadata;
use waterui_core::native::{Native, NativeView};
use waterui_shape::{ClipShape, Shape};

/// Decorative ring rendered around an [`Avatar`].
#[derive(Debug, Clone)]
pub struct AvatarRing {
    color: Color,
    width: f32,
}

impl AvatarRing {
    /// Creates a ring with `color` and stroke `width` in logical points.
    #[must_use]
    pub const fn new(color: Color, width: f32) -> Self {
        Self { color, width }
    }

    /// Returns the ring stroke color.
    #[must_use]
    pub const fn color(&self) -> &Color {
        &self.color
    }

    /// Returns the ring stroke width in logical points.
    #[must_use]
    pub const fn width(&self) -> f32 {
        self.width
    }
}

/// Configuration passed to a backend realizing an [`Avatar`].
#[derive(Debug)]
pub struct AvatarConfig<S> {
    spoken: Computed<Str>,
    monogram: Option<Computed<Str>>,
    fallback: Option<S>,
    #[cfg(feature = "image")]
    image: Option<Computed<Url>>,
    shape: Option<ClipShape>,
    side: Option<f32>,
    ring: Option<AvatarRing>,
}

impl<S> AvatarConfig<S> {
    /// Returns the resolved accessibility label for this avatar.
    #[must_use]
    pub const fn spoken(&self) -> &Computed<Str> {
        &self.spoken
    }

    /// Returns the reactive monogram text derived from the avatar's name, if no
    /// custom fallback view was supplied.
    #[must_use]
    pub const fn monogram(&self) -> Option<&Computed<Str>> {
        self.monogram.as_ref()
    }

    /// Returns the custom fallback child view slot, if supplied.
    #[must_use]
    pub const fn fallback(&self) -> Option<&S> {
        self.fallback.as_ref()
    }

    /// Takes ownership of the custom fallback child view slot, if supplied.
    pub const fn take_fallback(&mut self) -> Option<S> {
        self.fallback.take()
    }

    /// Returns the optional remote or local image URL signal.
    #[cfg(feature = "image")]
    #[must_use]
    pub const fn image(&self) -> Option<&Computed<Url>> {
        self.image.as_ref()
    }

    /// Returns the optional clip shape requested by `.shape(...)`.
    #[must_use]
    pub const fn shape(&self) -> Option<&ClipShape> {
        self.shape.as_ref()
    }

    /// Returns the optional side length in points requested by `.size(...)` / `.side(...)`.
    #[must_use]
    pub const fn side(&self) -> Option<f32> {
        self.side
    }

    /// Returns the optional ring outline requested by `.ring(...)`.
    #[must_use]
    pub const fn ring(&self) -> Option<&AvatarRing> {
        self.ring.as_ref()
    }
}

impl<S: 'static> MapSlots<S> for AvatarConfig<S> {
    type With<T: 'static> = AvatarConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> Self::With<T> {
        AvatarConfig {
            spoken: self.spoken,
            monogram: self.monogram,
            fallback: self.fallback.map(f),
            #[cfg(feature = "image")]
            image: self.image,
            shape: self.shape,
            side: self.side,
            ring: self.ring,
        }
    }
}

impl<S: 'static> NativeView for AvatarConfig<S> {}

impl<S> fmt::Display for AvatarConfig<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Avatar")
    }
}

/// Default monogram fallback built from the avatar's name.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct Monogram;

mod sealed {
    pub trait Sealed {}
}

/// Fallback view builder used by [`Avatar`].
#[doc(hidden)]
pub trait AvatarFallback: sealed::Sealed + 'static {
    /// Resolves this fallback into either a derived monogram signal or a child slot.
    fn build_fallback<S: ChildSlot>(
        self,
        spoken: &Computed<Str>,
    ) -> (Option<Computed<Str>>, Option<S>);
}

impl sealed::Sealed for Monogram {}

impl AvatarFallback for Monogram {
    fn build_fallback<S: ChildSlot>(
        self,
        spoken: &Computed<Str>,
    ) -> (Option<Computed<Str>>, Option<S>) {
        let monogram = spoken.map(|name| initials(&name)).computed();
        (Some(monogram), None)
    }
}

impl<F: ViewBuilder> sealed::Sealed for F {}

impl<F: ViewBuilder> AvatarFallback for F {
    fn build_fallback<S: ChildSlot>(
        self,
        _spoken: &Computed<Str>,
    ) -> (Option<Computed<Str>>, Option<S>) {
        (None, Some(S::from_view(self.build())))
    }
}

/// A small portrait of a person or an entity.
///
/// The avatar always carries the name it stands for: assistive technology
/// announces it, and with no image and no supplied fallback the initials drawn
/// in the circle are derived from it. Everything else — shape, size, ring — is
/// an attribute, so there is one `Avatar` type rather than a `CircleAvatar`
/// and a `SquareAvatar`.
///
/// # Examples
///
/// ```rust
/// use waterui::prelude::*;
///
/// let author = avatar("Ada Lovelace");
/// ```
///
/// With a picture, a rounded-rectangle silhouette and a ring:
///
/// ```rust
/// use waterui::prelude::*;
/// use waterui::shape::{CornerRadius, RoundedRectangle};
///
/// # fn profile(photo: waterui::Url) -> impl View {
/// avatar("Grace Hopper")
///     .image(photo)
///     .shape(RoundedRectangle::new(CornerRadius::fraction(0.25)))
///     .side(64.0)
///     .ring(Color::accent(), 2.0)
/// # }
/// ```
#[derive(Debug)]
pub struct Avatar<F = Monogram> {
    label: Label<SemanticLabel>,
    fallback: F,
    #[cfg(feature = "image")]
    image: Option<Computed<Url>>,
    shape: Option<ClipShape>,
    side: Option<f32>,
    ring: Option<AvatarRing>,
    dynamic_range: Computed<DynamicRange>,
}

impl<F: ViewBuilder> Avatar<F> {
    /// An avatar named `name` that shows `fallback` when it has no image.
    ///
    /// This is the general constructor: `fallback` is any view — a monogram of
    /// your own, an icon, a generated identicon. For the ordinary case, where
    /// the fallback is the initials of the name, use [`fn@avatar`].
    ///
    /// `name` is required, and required at construction, for the same reason
    /// every `WaterUI` control demands a [`Label`]: a portrait with no name is
    /// an anonymous image to a screen reader. It is never drawn — only spoken,
    /// and read for the initials.
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::widget::avatar::Avatar;
    ///
    /// # fn team() -> impl View {
    /// Avatar::new("Katherine Johnson", || text("KJ").bold(true))
    /// # }
    /// ```
    #[must_use]
    pub fn new(name: impl IntoLabel, fallback: F) -> Self {
        Self::with_fallback(name, fallback)
    }
}

impl<F> Avatar<F> {
    /// The shared constructor: [`Monogram`] means "derive a monogram from the name".
    fn with_fallback(name: impl IntoLabel, fallback: F) -> Self {
        Self {
            label: name.into_label().into_semantic(),
            fallback,
            #[cfg(feature = "image")]
            image: None,
            shape: None,
            side: None,
            ring: None,
            // The issue this component answers, water-rs/waterui#93, is one
            // line: ban HDR by default.
            dynamic_range: DynamicRange::Standard.into_computed(),
        }
    }

    /// Shows `source` inside the avatar's silhouette.
    ///
    /// The source is a signal, so pointing an avatar at a different picture
    /// replaces the decoded frame without rebuilding anything around it. The
    /// fallback stays behind the picture and shows through until the first
    /// frame decodes, and again if the load fails.
    #[cfg(feature = "image")]
    #[must_use]
    pub fn image(mut self, source: impl IntoComputed<Url>) -> Self {
        self.image = Some(source.into_computed());
        self
    }

    /// Clips the avatar to `shape` instead of a circle.
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::shape::RoundedRectangle;
    ///
    /// # fn squircle() -> impl View {
    /// avatar("Radia Perlman").shape(RoundedRectangle::new(16.0))
    /// # }
    /// ```
    #[must_use]
    pub fn shape(mut self, shape: impl Shape) -> Self {
        self.shape = Some(ClipShape::new(shape));
        self
    }

    /// Sets the avatar's side length in points. Avatars are square.
    #[must_use]
    pub const fn side(mut self, side: f32) -> Self {
        self.side = Some(side);
        self
    }

    /// Draws a ring of `width` points around the avatar in `color`.
    ///
    /// The ring is drawn inside the avatar's own side length, so a ring never
    /// changes how much room the avatar takes in a row.
    #[must_use]
    pub fn ring(mut self, color: impl IntoSignal<Color> + 'static, width: f32) -> Self {
        self.ring = Some(AvatarRing::new(Color::from_signal(color), width));
        self
    }

    /// Chooses the dynamic range the avatar's contents are shown in.
    ///
    /// An avatar defaults to [`DynamicRange::Standard`], against the framework-wide
    /// default of [`DynamicRange::High`]. A portrait is a thumbnail sitting in a row
    /// of chrome: an HDR source painted at 40 points has its highlights driven past
    /// the display's white point, so the small bright disc blooms and glares against
    /// the text beside it, and a row of avatars flickers as each one loads. Opt an
    /// individual avatar back in with `.dynamic_range(DynamicRange::High)` when its
    /// picture is the subject rather than a marker — a profile header at full width,
    /// say.
    #[must_use]
    pub fn dynamic_range(mut self, range: impl IntoComputed<DynamicRange>) -> Self {
        self.dynamic_range = range.into_computed();
        self
    }

    fn into_parts<S: ChildSlot>(
        self,
        env: &Environment,
    ) -> (AvatarConfig<S>, Computed<DynamicRange>)
    where
        F: AvatarFallback,
    {
        let Self {
            label,
            fallback,
            #[cfg(feature = "image")]
            image,
            shape,
            side,
            ring,
            dynamic_range,
        } = self;

        // Resolving first is what lets `accessibility_label` read a localized
        // name: an unresolved environment-dependent `Text` has no content
        // signal to take.
        let name = label.resolve(env).accessibility_label();
        let spoken = name.map(|name| name.to_plain()).computed();
        let (monogram, fallback) = fallback.build_fallback::<S>(&spoken);

        let config = AvatarConfig {
            spoken,
            monogram,
            fallback,
            #[cfg(feature = "image")]
            image,
            shape,
            side,
            ring,
        };
        (config, dynamic_range)
    }
}

impl<F: AvatarFallback> View for Avatar<F> {
    fn body(self, env: &Environment) -> impl View {
        let (config, dynamic_range) = self.into_parts::<AnyView>(env);
        Metadata::new(
            Native::new(config),
            DynamicRangeMetadata::new(dynamic_range),
        )
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let (config, dynamic_range) = self.into_parts::<B::Slot>(env);
        Metadata::new(
            Native::new(config),
            DynamicRangeMetadata::new(dynamic_range),
        )
        .dispatch(backend, env, cx)
    }
}

/// An avatar named `name`, showing the initials of that name when it has no
/// image.
///
/// The ergonomic counterpart to [`Avatar::new`], which takes an arbitrary
/// fallback view.
///
/// ```rust
/// use waterui::prelude::*;
///
/// # fn row() -> impl View {
/// hstack((avatar("Ada Lovelace"), text("Ada Lovelace")))
/// # }
/// ```
#[must_use]
pub fn avatar(name: impl IntoLabel) -> Avatar {
    Avatar::with_fallback(name, Monogram)
}

/// The monogram shown for `name` when an avatar has no picture.
///
/// One grapheme cluster from the first word, and one from the last when the
/// name has more than one word: `"Ada Lovelace"` reads `AL`, `"山田 太郎"`
/// reads `山太`. A single word contributes a single glyph — `"Ada"` is `A`,
/// not `AD`, because slicing a second letter out of one word produces an
/// abbreviation nobody wrote, and in an unspaced script such as Chinese it
/// would cut a name in half. Grapheme clusters rather than `char`s, so a
/// combining mark or an emoji sequence is not split down the middle.
/// Uppercasing is Unicode's own and leaves caseless scripts alone.
fn initials(name: &str) -> Str {
    let mut monogram = String::new();
    let mut words = name.split_whitespace();
    if let Some(first) = words.next() {
        monogram.push_str(&first_grapheme(first));
        if let Some(last) = words.last() {
            monogram.push_str(&first_grapheme(last));
        }
    }
    monogram.into()
}

fn first_grapheme(word: &str) -> String {
    word.graphemes(true)
        .next()
        .map_or_default(str::to_uppercase)
}

#[cfg(test)]
mod tests {
    use super::{Avatar, AvatarRing, avatar, initials};
    use crate::metadata::secure::{DynamicRange, DynamicRangeMetadata};
    use nami::{Signal, binding};
    use waterui_core::{AnyView, Environment, backend::peel_metadata};
    use waterui_shape::{ClipShape, CornerRadius, RoundedRectangle, ShapeKind};
    use waterui_text::text;

    #[test]
    fn an_avatar_is_standard_dynamic_range_unless_asked_otherwise() {
        let metadata =
            peel_metadata::<DynamicRangeMetadata>(avatar("Ada Lovelace"), &Environment::new())
                .expect("water-rs/waterui#93: an avatar opts its contents out of HDR");
        assert_eq!(metadata.value.range().snapshot(), DynamicRange::Standard);
    }

    #[test]
    fn an_avatar_can_be_opted_back_into_high_dynamic_range() {
        let metadata = peel_metadata::<DynamicRangeMetadata>(
            avatar("Ada Lovelace").dynamic_range(DynamicRange::High),
            &Environment::new(),
        )
        .expect("an avatar whose picture is the subject can ask for the framework default back");
        assert_eq!(metadata.value.range().snapshot(), DynamicRange::High);
    }

    #[test]
    fn an_avatar_dynamic_range_tracks_a_signal() {
        let range = binding(DynamicRange::Standard);
        let metadata = peel_metadata::<DynamicRangeMetadata>(
            avatar("Ada Lovelace").dynamic_range(range.clone()),
            &Environment::new(),
        )
        .expect("avatar attaches dynamic-range metadata");
        assert_eq!(metadata.value.range().snapshot(), DynamicRange::Standard);
        range.set(DynamicRange::High);
        assert_eq!(metadata.value.range().snapshot(), DynamicRange::High);
    }

    #[test]
    fn two_word_names_take_the_first_and_last_initial() {
        assert_eq!(&*initials("Ada Lovelace"), "AL");
        assert_eq!(&*initials("grace brewster murray hopper"), "GH");
    }

    #[test]
    fn one_word_names_take_a_single_glyph() {
        assert_eq!(&*initials("Ada"), "A");
        assert_eq!(&*initials("madonna"), "M");
    }

    #[test]
    fn caseless_scripts_are_left_alone() {
        assert_eq!(&*initials("山田 太郎"), "山太");
        assert_eq!(&*initials("山田太郎"), "山");
        assert_eq!(&*initials("Ольга Ладыженская"), "ОЛ");
    }

    #[test]
    fn a_grapheme_cluster_is_never_split() {
        // A base letter plus a combining acute; taking one `char` would show
        // the accent as a lone mark on the next glyph.
        assert_eq!(&*initials("e\u{301}douard Lucas"), "E\u{301}L");
        assert_eq!(&*initials("👩‍🚀 Crew"), "👩‍🚀C");
    }

    #[test]
    fn an_empty_name_has_no_monogram() {
        assert_eq!(&*initials("   "), "");
        assert_eq!(&*initials(""), "");
    }

    #[test]
    fn avatar_config_carries_semantic_attributes() {
        let env = Environment::new();
        let (config, _) = avatar("Ada Lovelace").into_parts::<AnyView>(&env);
        assert_eq!(&*config.spoken().snapshot(), "Ada Lovelace");
        assert_eq!(config.monogram().map(Signal::snapshot), Some("AL".into()));
        assert!(config.fallback().is_none());
        assert!(config.shape().is_none());
        assert!(config.side().is_none());
        assert!(config.ring().is_none());

        let (custom, _) = Avatar::new("Katherine Johnson", || text("KJ"))
            .shape(RoundedRectangle::new(CornerRadius::fraction(0.25)))
            .side(48.0)
            .ring(crate::prelude::Color::srgb(255, 0, 0), 2.0)
            .into_parts::<AnyView>(&env);
        assert!(custom.monogram().is_none());
        assert!(custom.fallback().is_some());
        assert!(matches!(
            custom.shape().map(ClipShape::kind),
            Some(ShapeKind::RoundedRect { .. })
        ));
        assert_eq!(custom.side(), Some(48.0));
        assert_eq!(custom.ring().map(AvatarRing::width), Some(2.0));
    }

    #[test]
    fn avatar_side_does_not_shadow_view_ext_size() {
        use crate::view::ViewExt as _;
        let _framed = avatar("Ada Lovelace").side(48.0).size(64.0, 64.0);
    }
}
