//! A small portrait standing in for a person or an entity.
//!
//! An avatar is an image clipped to a shape, with something legible to show
//! when there is no image: the initials of a name, or a view the caller
//! supplies. It is a Rust-side composition of primitives that already exist —
//! a stack, a frame, a clip, theme tokens — and ships no FFI type of its own.

use alloc::string::String;
use alloc::vec::Vec;

use crate::metadata::secure::ColorSpace;
use crate::prelude::*;
use crate::theme::color::{MutedForeground, SurfaceVariant};
#[cfg(feature = "media")]
use nami::signal::IntoComputed;
use unicode_segmentation::UnicodeSegmentation as _;
use waterui_controls::{IntoLabel, Label};
use waterui_core::accessibility::{AccessibilityChildren, AccessibilityRole};
use waterui_core::handler::{AnyViewBuilder, ViewBuilder};
use waterui_layout::stack::zstack;
use waterui_shape::{Circle, PathCommand, Shape, ShapeExt, ShapeKind};
use waterui_text::Text;

/// The side length an avatar takes when the caller names none, in points.
///
/// A list row's leading avatar, which is what most of them are.
const DEFAULT_SIZE: f32 = 40.0;

/// Initial glyphs are drawn at this fraction of the avatar's inner side.
///
/// Two capitals at 40% of a circle's diameter sit inside it with the optical
/// margin a monogram wants; larger and the letters touch the clip.
const INITIALS_SCALE: f32 = 0.4;

/// A shape captured by value so an avatar can both clip to it and fill behind
/// it.
///
/// [`Shape`] is a producer of path commands, not a value: `ClipShape::new`
/// consumes one and `ShapeExt::fill` consumes another, while an avatar needs
/// the same silhouette for its clip, its container fill, and its ring. Holding
/// the resolved [`ShapeKind`] and unit-space commands lets one authored shape
/// answer all three. The kind is what backends act on — a normalized radius
/// resolved per axis turns a circular corner elliptical — so it is carried
/// alongside the commands rather than derived from them.
#[derive(Debug, Clone)]
struct CapturedShape {
    kind: ShapeKind,
    commands: Vec<PathCommand>,
}

impl CapturedShape {
    fn new(shape: &impl Shape) -> Self {
        Self {
            kind: shape.shape_kind(),
            commands: shape.path().into_iter().collect(),
        }
    }

    /// The corner radius, in points, that a [`Border`] must use to trace this
    /// shape around a square of `side` points.
    ///
    /// An avatar's box is square, so a circle, an ellipse inscribed in it and
    /// a capsule are the same outline: half the side.
    fn corner_radius(&self, side: f32) -> f32 {
        match self.kind {
            ShapeKind::Rect | ShapeKind::CustomPath => 0.0,
            ShapeKind::Circle | ShapeKind::Ellipse | ShapeKind::Capsule => side / 2.0,
            ShapeKind::RoundedRect { corner_radius } => corner_radius * side,
            ShapeKind::UnevenRoundedRect {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => top_left.max(top_right).max(bottom_left).max(bottom_right) * side,
        }
    }
}

impl Shape for CapturedShape {
    type Iter = Vec<PathCommand>;

    fn path(&self) -> Self::Iter {
        self.commands.clone()
    }

    fn shape_kind(&self) -> ShapeKind {
        self.kind
    }
}

impl ShapeExt for CapturedShape {}

/// The outline drawn around an avatar.
#[derive(Debug)]
struct Ring {
    color: Color,
    width: f32,
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
/// use waterui::shape::RoundedRectangle;
///
/// # fn profile(photo: waterui::Url) -> impl View {
/// avatar("Grace Hopper")
///     .image(photo)
///     .shape(RoundedRectangle::new(0.25))
///     .size(64.0)
///     .ring(Color::new(theme_color::Accent), 2.0)
/// # }
/// ```
#[derive(Debug)]
pub struct Avatar {
    label: Label,
    fallback: Option<AnyViewBuilder<AnyView>>,
    #[cfg(feature = "media")]
    image: Option<Computed<Url>>,
    shape: CapturedShape,
    size: f32,
    ring: Option<Ring>,
    color_space: ColorSpace,
}

impl Avatar {
    /// An avatar named `name` that shows `fallback` when it has no image.
    ///
    /// This is the general constructor: `fallback` is any view — a monogram of
    /// your own, an icon, a generated identicon. For the ordinary case, where
    /// the fallback is the initials of the name, use [`avatar`].
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
    /// Avatar::new("Katherine Johnson", || text("KJ").bold())
    /// # }
    /// ```
    #[must_use]
    pub fn new(name: impl IntoLabel, fallback: impl ViewBuilder) -> Self {
        Self::with_fallback(
            name,
            Some(AnyViewBuilder::new(move || AnyView::new(fallback.build()))),
        )
    }

    /// The shared constructor: `None` means "derive a monogram from the name".
    fn with_fallback(name: impl IntoLabel, fallback: Option<AnyViewBuilder<AnyView>>) -> Self {
        Self {
            label: name.into_label(),
            fallback,
            #[cfg(feature = "media")]
            image: None,
            shape: CapturedShape::new(&Circle),
            size: DEFAULT_SIZE,
            ring: None,
            // The issue this component answers, water-rs/waterui#93, is one
            // line: ban HDR by default.
            color_space: ColorSpace::Sdr,
        }
    }

    /// Shows `source` inside the avatar's silhouette.
    ///
    /// The source is a signal, so pointing an avatar at a different picture
    /// replaces the decoded frame without rebuilding anything around it. The
    /// fallback stays behind the picture and shows through until the first
    /// frame decodes, and again if the load fails.
    #[cfg(feature = "media")]
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
    /// avatar("Radia Perlman").shape(RoundedRectangle::new(0.25))
    /// # }
    /// ```
    #[must_use]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "a shape is authored inline (`.shape(RoundedRectangle::new(0.25))`); taking a reference would force the caller to bind it first, and `ClipShape::new` takes it by value for the same reason"
    )]
    pub fn shape(mut self, shape: impl Shape) -> Self {
        self.shape = CapturedShape::new(&shape);
        self
    }

    /// Sets the avatar's side length in points. Avatars are square.
    #[must_use]
    pub const fn size(mut self, side: f32) -> Self {
        self.size = side;
        self
    }

    /// Draws a ring of `width` points around the avatar in `color`.
    ///
    /// The ring is drawn inside the avatar's own side length, so a ring never
    /// changes how much room the avatar takes in a row.
    #[must_use]
    pub fn ring(mut self, color: impl Into<Color>, width: f32) -> Self {
        self.ring = Some(Ring {
            color: color.into(),
            width,
        });
        self
    }

    /// Chooses the dynamic range the avatar's contents are shown in.
    ///
    /// An avatar defaults to [`ColorSpace::Sdr`], against the framework-wide
    /// default of HDR. A portrait is a thumbnail sitting in a row of chrome: an
    /// HDR source painted at 40 points has its highlights driven past the
    /// display's white point, so the small bright disc blooms and glares
    /// against the text beside it, and a row of avatars flickers as each one
    /// loads. Opt an individual avatar back in with
    /// `.dynamic_range(ColorSpace::Hdr)` when its picture is the subject rather
    /// than a marker — a profile header at full width, say.
    #[must_use]
    pub const fn dynamic_range(mut self, space: ColorSpace) -> Self {
        self.color_space = space;
        self
    }
}

impl View for Avatar {
    fn body(self, env: &Environment) -> impl View {
        let Self {
            label,
            fallback,
            #[cfg(feature = "media")]
            image,
            shape,
            size,
            ring,
            color_space,
        } = self;

        // Resolving first is what lets `accessibility_label` read a localized
        // name: an unresolved environment-dependent `Text` has no content
        // signal to take.
        let name = label.resolve(env).accessibility_label();
        let spoken = name.map(|name| name.to_plain()).computed();

        let ring_width = ring.as_ref().map_or(0.0, |ring| ring.width);
        let inner = (size - ring_width * 2.0).max(0.0);

        let fallback = fallback.map_or_else(
            || {
                AnyView::new(
                    Text::new(spoken.map(|name| initials(&name)).computed())
                        .size(f64::from(inner * INITIALS_SCALE))
                        .color(Color::new(MutedForeground)),
                )
            },
            |fallback| fallback.build(),
        );

        #[cfg(feature = "media")]
        let picture = image.map(|source| AnyView::new(Photo::new(source).resizable()));
        #[cfg(not(feature = "media"))]
        let picture: Option<AnyView> = None;

        let content = zstack((fallback, picture))
            .size(inner, inner)
            .background(shape.clone().fill(Color::new(SurfaceVariant)))
            .clip(shape.clone());

        let framed = match ring {
            Some(ring) => AnyView::new(content.padding_with(ring_width).border_with(
                Border::new(ring.color, ring.width).corner_radius(shape.corner_radius(size)),
            )),
            None => AnyView::new(content),
        };

        framed
            .a11y_label(spoken)
            .a11y_role(AccessibilityRole::Image)
            // The initials and the picture are the avatar's own chrome. Without
            // this the monogram would announce itself a second time, under the
            // node that already carries the person's full name.
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
            .color_space(color_space)
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
    Avatar::with_fallback(name, None)
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
        .map(str::to_uppercase)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{Avatar, CapturedShape, avatar, initials};
    use crate::metadata::secure::{ColorSpace, HighDynamicRange, StandardDynamicRange};
    use waterui_core::{Environment, Metadata, View};
    use waterui_shape::{Circle, RoundedRectangle, ShapeKind};

    /// `AnyView::new` unwraps an `AnyView` rather than nesting one, so this
    /// hands back exactly the view `Avatar::body` produced.
    fn realize(avatar: Avatar) -> waterui_core::AnyView {
        waterui_core::AnyView::new(avatar.body(&Environment::new()))
    }

    #[test]
    fn an_avatar_is_standard_dynamic_range_unless_asked_otherwise() {
        assert!(
            realize(avatar("Ada Lovelace"))
                .downcast::<Metadata<StandardDynamicRange>>()
                .is_ok(),
            "water-rs/waterui#93: an avatar opts its contents out of HDR"
        );
    }

    #[test]
    fn an_avatar_can_be_opted_back_into_high_dynamic_range() {
        assert!(
            realize(avatar("Ada Lovelace").dynamic_range(ColorSpace::Hdr))
                .downcast::<Metadata<HighDynamicRange>>()
                .is_ok(),
            "an avatar whose picture is the subject can ask for the framework default back"
        );
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
    fn a_ring_traces_the_shape_it_surrounds() {
        assert!((CapturedShape::new(&Circle).corner_radius(40.0) - 20.0).abs() < f32::EPSILON);
        assert!(
            (CapturedShape::new(&RoundedRectangle::new(0.25)).corner_radius(40.0) - 10.0).abs()
                < f32::EPSILON
        );
        assert!(matches!(
            CapturedShape::new(&Circle).kind,
            ShapeKind::Circle
        ));
    }
}
