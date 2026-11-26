//! Backdrop material and Liquid Glass background contracts.

use alloc::sync::Arc;

use nami::cache::Cached;
use nami::constant::{Constant, Lazy};
use nami::debug::Debug as DebugSignal;
use nami::distinct::Distinct;
use nami::map::Map;
use nami::scan::Scan;
use nami::signal::{IntoSignal, WithMetadata};
use nami::{Binding, Signal, impl_constant};
use waterui_core::{Computed, Metadata, View, metadata::MetadataKey};
use waterui_graphics::color::Color;
use waterui_layout::BackgroundView;

use crate::{Capsule, ClipShape, Shape, ShapeKind};

/// A material background metadata: the [`Material`] a backend realizes
/// behind the wrapped content, following the contract [`Material`] states.
///
/// # Usage
///
/// Use via the `.background(Material::*)` API rather than directly:
///
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// let frosted = text!("Hello").background(Material::Regular);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct MaterialBackground(pub Material);

impl MetadataKey for MaterialBackground {}

/// A Liquid Glass background metadata.
///
/// Every backend realizes it, as it does [`MaterialBackground`]: a backend
/// that has the platform's glass primitive projects it, and any other backend
/// approximates it.
///
/// Use via the `.background(Glass::…)` API rather than directly:
///
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// let pill = text!("Now Playing").padding().background(Glass::regular());
/// ```
#[derive(Debug, Clone)]
pub struct GlassBackground(pub Glass);

impl MetadataKey for GlassBackground {}

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
/// Every backend handles it: a realization whose materials do not share a
/// backdrop renders the content unchanged.
///
/// Use via `.material_group()` rather than directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialGroup;

impl MetadataKey for MaterialGroup {}

/// Material types for background blur effects.
///
/// Materials create translucent blur effects that allow content behind the view
/// to show through with varying degrees of blur and vibrancy.
///
/// Every backend realizes `Material`, as a view's background through
/// metadata and as a window's background. The levels fall in two groups:
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
///   background: the window is translucent and tinted with the level's colour
///   treatment. The desktop behind it is blurred where the platform's
///   blur-behind is wired — an `NSVisualEffectView` blends it behind the
///   window on macOS, the DWM's acrylic system backdrop blurs it on Windows 11
///   22H2 and later, an X11 window manager honouring
///   `_KDE_NET_WM_BLUR_BEHIND_REGION` (`KWin`) blurs it, and so does a Wayland
///   compositor advertising `ext-background-effect-v1` and applying its blur
///   by its own policy — and shows through unblurred elsewhere: older Windows,
///   other X11 window managers and Wayland compositors not advertising the
///   global keep the window translucent and unblurred. As a view's background,
///   a behind-window level is unsupported on Hydrolysis and panics naming the
///   level (water-rs/waterui#1853).
///
/// # Examples
///
/// ```rust,ignore
/// use waterui::prelude::*;
/// use waterui::shape::RoundedRectangle;
///
/// // Frosted glass card
/// let card = text!("Hello")
///     .padding()
///     .background(Material::Regular)
///     .clip(RoundedRectangle::new(8.0));
///
/// // A heavier frost for an overlay over busy content
/// let overlay = text!("Overlay").background(Material::Thick);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each material variant by variant; a new material must fail their build instead of falling into a wildcard arm"
)]
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

impl_constant!(Material);

/// The two looks a Liquid Glass surface can have.
///
/// Glass is not a position on the [`Material`] thickness scale: it has its own
/// parameter set, so it is a separate type rather than a sixth material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each glass style variant by variant; a new glass style must fail their build instead of falling into a wildcard arm"
)]
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
/// ```rust,ignore
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
///             .shape(RoundedRectangle::new(12.0)),
///     );
/// ```
#[derive(Debug, Clone)]
pub struct Glass {
    style: GlassStyle,
    interactive: bool,
    tint: Option<Color>,
    shape: Arc<ClipShape>,
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
            shape: Arc::new(ClipShape::new(Capsule)),
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
    pub fn tint(mut self, color: impl IntoSignal<Color> + 'static) -> Self {
        self.tint = Some(Color::from_signal(color));
        self
    }

    /// The outline of the glass surface: the rectangle family, a capsule, a
    /// circle, an ellipse or a custom path.
    ///
    /// Apple's system glass takes its outline from a corner configuration, so
    /// an ellipse and a custom path are unsupported there, and on macOS so
    /// are per-corner radii.
    #[must_use]
    pub fn shape(mut self, shape: impl Shape) -> Self {
        self.shape = Arc::new(ClipShape::new(shape));
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

    /// The kind of the glass surface's outline.
    #[must_use]
    pub fn shape_kind(&self) -> ShapeKind {
        self.shape.kind()
    }

    /// The glass surface's outline: its kind and, for a custom path, the
    /// unit-space path commands.
    #[must_use]
    pub fn outline(&self) -> &ClipShape {
        &self.shape
    }
}

/// Marker for backgrounds implemented directly as views, [`Material`], or [`Glass`].
#[derive(Debug, Clone, Copy)]
pub struct ViewBackground;

/// Marker for reactive colour signals applied directly to `.background(..)`.
#[derive(Debug, Clone, Copy)]
pub struct ColorSignalBackground;

/// A trait for types that can be applied as backgrounds.
///
/// This enables a unified `.background()` API that accepts:
/// - `Material` - Creates a native blur effect (`MaterialBackground` metadata)
/// - `Glass` - Creates a Liquid Glass background (`GlassBackground` metadata)
/// - Any [`View`] - Creates a standard background view
/// - Any colour [`Signal`] (such as `selected.select(Color::from(Accent), Color::from(SurfaceVariant))`)
pub trait IntoBackground<Marker = ViewBackground> {
    /// The output view type after applying the background.
    type Output<Content: View>: View;

    /// Apply this background to the given content view.
    fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content>;
}

impl IntoBackground<ViewBackground> for Material {
    type Output<Content: View> = Metadata<MaterialBackground, Content>;

    fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content> {
        Metadata::new(content, MaterialBackground(self))
    }
}

impl IntoBackground<ViewBackground> for Glass {
    type Output<Content: View> = Metadata<GlassBackground, Content>;

    fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content> {
        Metadata::new(content, GlassBackground(self))
    }
}

impl<V: View> IntoBackground<ViewBackground> for V {
    type Output<Content: View> = BackgroundView<Content, V>;

    fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content> {
        BackgroundView::new(content, self)
    }
}

macro_rules! impl_color_signal_background {
    ($($ty:ty => ($($generics:ident),*));+ $(;)?) => {
        $(
            impl<$($generics),*> IntoBackground<ColorSignalBackground> for $ty
            where
                Self: IntoSignal<Color> + 'static,
            {
                type Output<Content: View> = BackgroundView<Content, Color>;

                fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content> {
                    BackgroundView::new(content, Color::from_signal(self))
                }
            }
        )+
    };
}

impl_color_signal_background!(
    Binding<Out> => (Out);
    Computed<Out> => (Out);
    Map<C, F, Out> => (C, F, Out);
    Constant<Out> => (Out);
    Lazy<F, Out> => (F, Out);
    Scan<Out> => (Out);
    WithMetadata<C, T> => (C, T);
    DebugSignal<C> => (C);
);

impl<C> IntoBackground<ColorSignalBackground> for Cached<C>
where
    C: Signal,
    C::Output: Clone,
    Self: IntoSignal<Color> + 'static,
{
    type Output<Content: View> = BackgroundView<Content, Color>;

    fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content> {
        BackgroundView::new(content, Color::from_signal(self))
    }
}

impl<S> IntoBackground<ColorSignalBackground> for Distinct<S>
where
    S: Signal,
    S::Output: PartialEq,
    Self: IntoSignal<Color> + 'static,
{
    type Output<Content: View> = BackgroundView<Content, Color>;

    fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content> {
        BackgroundView::new(content, Color::from_signal(self))
    }
}
