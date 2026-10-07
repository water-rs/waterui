//! Background styling for views.
//!
//! Backgrounds fill the bounds behind a view: any [`View`] — a solid
//! [`Color`], a [`gradient`](crate::gradient) view, an image — and the
//! platform's blur [`Material`]s and Liquid [`Glass`].
//!
//! # Rendering Model
//!
//! A [`View`] passed to [`background`](crate::ViewExt::background) is composed
//! by the framework in Rust via `BackgroundView`.
//!
//! `Material` and `Glass` are different: they become `MaterialBackground` and
//! `GlassBackground` metadata that each backend realizes, because they treat
//! the content behind the view rather than drawing a view of their own. Every
//! backend realizes `Material` (see its contract); `Glass` is an asymmetric
//! primitive.
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::color::WorkingColor;
//!
//! text!("Hello").background(Color::srgb(20, 40, 80));
//! text!("Hello").background(Gradient::linear(
//!     vec![(0.0, WorkingColor::BLACK), (1.0, WorkingColor::WHITE)],
//!     [0.5, 0.0],
//!     [0.5, 1.0],
//! ));
//! ```

use nami::signal::IntoComputed;
use suiteki::Str;
use waterui_core::{AnyView, Computed, IgnorableMetadata, View, metadata::MetadataKey};
use waterui_graphics::color::{Color, Srgb};
use waterui_graphics::gradient::Gradient;
use waterui_layout::BackgroundView;
use waterui_shape::{Capsule, Shape, ShapeKind};

/// A material background metadata: the [`Material`] a backend realizes
/// behind the wrapped content, following the contract [`Material`] states.
///
/// # Usage
///
/// Use via the `.background(Material::*)` API rather than directly:
///
/// ```rust
/// use waterui::prelude::*;
///
/// let frosted = text!("Hello").background(Material::Regular);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct MaterialBackground(pub Material);

impl MetadataKey for MaterialBackground {}

/// A trait for types that can be applied as backgrounds.
///
/// This enables a unified `.background()` API that accepts:
/// - `Material` - Creates a native blur effect (`MaterialBackground` metadata)
/// - Any [`View`] - Creates a standard background view
pub trait IntoBackground {
    /// The output view type after applying the background.
    type Output<Content: View>: View;

    /// Apply this background to the given content view.
    fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content>;
}

impl IntoBackground for Material {
    type Output<Content: View> = IgnorableMetadata<MaterialBackground>;

    fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content> {
        IgnorableMetadata::new(AnyView::new(content), MaterialBackground(self))
    }
}

/// A Liquid Glass background metadata.
///
/// This is an ignorable metadata delegated to native backends, the same way
/// [`MaterialBackground`] is: a backend that has the platform's glass primitive
/// projects it, and any other backend approximates it or renders the content
/// without a background.
///
/// Use via the `.background(Glass::…)` API rather than directly:
///
/// ```rust
/// use waterui::prelude::*;
///
/// let pill = text!("Now Playing").padding().background(Glass::regular());
/// ```
#[derive(Debug, Clone)]
pub struct GlassBackground(pub Glass);

impl MetadataKey for GlassBackground {}

impl IntoBackground for Glass {
    type Output<Content: View> = IgnorableMetadata<GlassBackground>;

    fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content> {
        IgnorableMetadata::new(AnyView::new(content), GlassBackground(self))
    }
}

/// A material group metadata: the backdrop materials in the wrapped subtree
/// form one group.
///
/// The members of one group share one capture of what lies behind the
/// group, so a member does not see another member of its group. Where the
/// realization's style supports it, members may also merge into one shape
/// where they come close. Whether and how they merge, and the distance
/// over which they do, belong to the style and its theme tokens, never to
/// the view.
///
/// The nearest enclosing group wins: a group nested in another starts a
/// group of its own, and its materials do not join the outer one. A
/// material outside every group is a group of its own.
///
/// This is ignorable metadata, like [`MaterialBackground`]: a realization
/// whose style does not group materials renders the content unchanged.
///
/// Use via [`material_group`](crate::ViewExt::material_group) rather than
/// directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialGroup;

impl MetadataKey for MaterialGroup {}

impl<V: View> IntoBackground for V {
    type Output<Content: View> = BackgroundView<Content, V>;

    fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content> {
        BackgroundView::new(content, self)
    }
}

/// Represents different kinds of backgrounds that can be applied to UI elements.
///
/// A metadata key that nothing in this repository attaches or reads: a
/// background is any [`View`] passed to
/// [`background`](crate::ViewExt::background). It remains because renderers
/// built against this crate still name it.
#[derive(Debug)]
pub enum Background {
    /// A solid color background.
    Color(Computed<Color>),
    /// An image background.
    Image(Computed<Str>),
    /// A material background (blur effects).
    Material(Material),
    /// A gradient background.
    Gradient(Gradient),
}

/// Material types for background blur effects.
///
/// Materials create translucent blur effects that allow content behind the view
/// to show through with varying degrees of blur and vibrancy.
///
/// The mainline backends, Apple and Hydrolysis, realize `Material`. As a
/// view's background it travels as ignorable metadata, so a backend that does
/// not realize it, such as an experimental one, draws the content without it;
/// as a window's background see [`WindowBackground::Material`]. The levels
/// fall in two groups:
///
/// - **Within-window levels** — [`Regular`](Self::Regular),
///   [`Thick`](Self::Thick) and [`UltraThick`](Self::UltraThick) — are a
///   backdrop treatment of the window's own content behind the view, clipped
///   to the view's shape: that content passes through the level's colour stage
///   (a luminance curve, a chroma gain and a brightness offset) and is then
///   blurred, with no tint layer on top. Apple platforms project them onto the
///   native visual-effect views; self-drawn backends draw the treatment
///   themselves, Hydrolysis through Cherenkov on every platform, Android
///   included. Hydrolysis's HWUI render target (water-rs/waterui#1899) must
///   realize the same treatment when it lands.
/// - **Behind-window levels** — [`UltraThin`](Self::UltraThin) and
///   [`Thin`](Self::Thin) — blur what lies behind the window, which is the
///   compositor's work and is therefore defined per platform. Apple platforms
///   realize them natively: macOS blends them behind the window, and iOS,
///   with nothing behind its windows, over the app's own content.
///
///   On Hydrolysis a behind-window level is realized as a window's
///   background ([`WindowBackground::Material`]): the window is translucent
///   and tinted with the level's colour treatment. The desktop behind it is
///   blurred where the platform's blur-behind is wired and shows through
///   unblurred elsewhere; X11 and Wayland on Linux and the system backdrop on
///   Windows are water-rs/waterui#1856, water-rs/waterui#1857 and
///   water-rs/waterui#1858. As a view's background, a behind-window level is
///   unsupported on Hydrolysis and panics naming the level
///   (water-rs/waterui#1853).
///
/// [`WindowBackground::Material`]: crate::window::WindowBackground::Material
///
/// # Examples
///
/// ```rust
/// use waterui::prelude::*;
/// use waterui::shape::RoundedRectangle;
///
/// // Frosted glass card
/// let card = text!("Hello")
///     .padding()
///     .background(Material::Regular)
///     .clip(RoundedRectangle::new(0.1));
///
/// // A heavier frost for an overlay over busy content
/// let overlay = text!("Overlay").background(Material::Thick);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Material {
    /// Ultra-thin blur, most transparent. Subtle frosted effect.
    UltraThin,
    /// Thin blur, slightly more opaque than ultra-thin.
    Thin,
    /// Regular blur, balanced transparency and blur.
    #[default]
    Regular,
    /// Thick blur, more opaque with stronger blur.
    Thick,
    /// Ultra-thick blur, most opaque. Heavy frosted effect.
    UltraThick,
}

nami::impl_constant!(Material);

/// The two looks a Liquid Glass surface can have.
///
/// Glass is not a position on the [`Material`] thickness scale: it has its own
/// parameter set, so it is a separate type rather than a sixth material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlassStyle {
    /// The standard glass: lensing and highlights over a light diffusion of
    /// the content behind it, legible over anything.
    #[default]
    Regular,
    /// Glass with less diffusion, for surfaces over media or colorful content
    /// that should stay visible through it.
    Clear,
}

/// A Liquid Glass background.
///
/// Glass is the chrome-layer surface of iOS 26 and macOS 26: a lens over the
/// content behind it rather than a frosted pane. It is distinct from
/// [`Material`] on purpose — Apple keeps `.regularMaterial` and `glassEffect`
/// apart as well — and it stays an asymmetric primitive: Apple backends
/// project it onto the platform's glass effect, other backends approximate or
/// ignore it.
///
/// Glass takes its outline from the effect itself rather than from an outer
/// clip, because a mask over a glass surface destroys its refraction and
/// highlights. The shape is therefore part of the glass; it is a capsule
/// unless [`Glass::shape`] says otherwise.
///
/// # Examples
///
/// ```rust
/// use waterui::prelude::*;
/// use waterui::shape::RoundedRectangle;
///
/// // A floating pill
/// let pill = text!("Now Playing").padding().background(Glass::regular());
///
/// // A tinted, touch-reactive card over a photo
/// let card = text!("Save")
///     .padding()
///     .background(
///         Glass::clear()
///             .interactive(true)
///             .tint(Color::srgb(20, 120, 255))
///             .shape(RoundedRectangle::new(0.2)),
///     );
/// ```
#[derive(Debug, Clone)]
pub struct Glass {
    style: GlassStyle,
    interactive: bool,
    tint: Option<Color>,
    shape: ShapeKind,
}

impl Default for Glass {
    fn default() -> Self {
        Self::new(GlassStyle::default())
    }
}

impl Glass {
    /// Creates glass of the given style with the default parameters: not
    /// interactive, untinted, capsule-shaped.
    #[must_use]
    pub fn new(style: GlassStyle) -> Self {
        Self {
            style,
            interactive: false,
            tint: None,
            shape: Capsule.shape_kind(),
        }
    }

    /// Regular glass, the default look.
    #[must_use]
    pub fn regular() -> Self {
        Self::new(GlassStyle::Regular)
    }

    /// Clear glass, for surfaces over media.
    #[must_use]
    pub fn clear() -> Self {
        Self::new(GlassStyle::Clear)
    }

    /// Whether the glass reacts to touch and pointer interaction with the
    /// platform's own press and hover effects.
    #[must_use]
    pub const fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    /// Washes the glass with a color.
    #[must_use]
    pub fn tint(mut self, color: impl Into<Color>) -> Self {
        self.tint = Some(color.into());
        self
    }

    /// The outline of the glass surface.
    #[must_use]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "shapes are unit values passed by value everywhere else (`.clip(Capsule)`); a reference here would read as a different API"
    )]
    pub fn shape(mut self, shape: impl Shape) -> Self {
        self.shape = shape.shape_kind();
        self
    }

    /// The glass style.
    #[must_use]
    pub const fn style(&self) -> GlassStyle {
        self.style
    }

    /// Whether the glass reacts to interaction.
    #[must_use]
    pub const fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// The tint, if any.
    #[must_use]
    pub const fn tint_color(&self) -> Option<&Color> {
        self.tint.as_ref()
    }

    /// The outline of the glass surface.
    #[must_use]
    pub const fn shape_kind(&self) -> ShapeKind {
        self.shape
    }
}

impl MetadataKey for Background {}

impl From<Color> for Background {
    fn from(color: Color) -> Self {
        Self::Color(Computed::new(color))
    }
}

impl From<Srgb> for Background {
    fn from(color: Srgb) -> Self {
        Self::from(Color::from(color))
    }
}

impl From<Material> for Background {
    fn from(material: Material) -> Self {
        Self::Material(material)
    }
}

impl From<Gradient> for Background {
    fn from(gradient: Gradient) -> Self {
        Self::Gradient(gradient)
    }
}

impl Background {
    /// Creates a new background with a solid color.
    pub fn color(color: impl IntoComputed<Color>) -> Self {
        Self::Color(color.into_computed())
    }

    /// Creates a new background with a blur material effect.
    #[must_use]
    pub const fn material(material: Material) -> Self {
        Self::Material(material)
    }
}
