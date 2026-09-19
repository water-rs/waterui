//! Background and foreground styling for views.
//!
//! Backgrounds fill the bounds behind a view. They support solid colors, gradients,
//! blur effects (materials), and images.
//!
//! # Rendering Model
//!
//! Most backgrounds (`Color`, gradients, and any `View` passed to `.background()`)
//! are composed by the framework in Rust via `BackgroundView`.
//!
//! `Material` is different: it becomes `MaterialBackground` metadata and is delegated
//! to platform backends on a best-effort basis, because true backdrop blur requires
//! native compositor APIs that are not uniformly available from the Rust layer.
//!
//! # Gradient Backgrounds
//!
//! [`Background`] describes gradients ([`Background::linear_gradient`],
//! [`Background::radial_gradient`], …) and backends render them with GPU shaders
//! for consistent cross-platform appearance.
//!
//! Note that [`background`](crate::ViewExt::background) itself takes an
//! [`IntoBackground`], which is implemented for [`Material`] and for any
//! [`View`] — a bare [`LinearGradient`] is neither, so it cannot be passed
//! directly. Layer a view that draws the gradient instead:
//!
//! ```rust
//! use waterui::prelude::*;
//!
//! text!("Hello").background(Color::srgb(20, 40, 80));
//! ```

use nami::signal::IntoComputed;
use suiteki::Str;
use waterui_core::{AnyView, Computed, IgnorableMetadata, View, metadata::MetadataKey};
use waterui_graphics::color::{Color, Srgb};
use waterui_layout::BackgroundView;
use waterui_shape::{Capsule, Shape, ShapeKind};

use crate::gradient::{
    AngularGradient, ColorStop, Gradient, LinearGradient, MeshGradient, MeshVertex, RadialGradient,
    UnitPoint,
};

/// A material background metadata for native blur effects.
///
/// This is an ignorable metadata delegated to native backends. Backends should
/// provide the best material effect they can (real blur, approximation, or no-op).
///
/// This metadata remains ignorable because full material/backdrop effects depend
/// on platform compositor capabilities and cannot be implemented uniformly in Rust.
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

impl<V: View> IntoBackground for V {
    type Output<Content: View> = BackgroundView<Content, V>;

    fn apply_background<Content: View>(self, content: Content) -> Self::Output<Content> {
        BackgroundView::new(content, self)
    }
}

/// Represents different kinds of backgrounds that can be applied to UI elements.
#[derive(Debug)]
pub enum Background {
    /// A solid color background.
    Color(Computed<Color>),
    /// An image background.
    Image(Computed<Str>),
    /// A material background (blur effects).
    Material(Material),
    /// A gradient background (linear, radial, angular, or mesh).
    Gradient(Gradient),
}

/// Material types for background blur effects.
///
/// Materials create translucent blur effects that allow content behind the view
/// to show through with varying degrees of blur and vibrancy.
///
/// Material rendering is backend-defined and best-effort.
/// - On Apple platforms, this typically maps to native material/visual effect APIs.
/// - Other platforms may provide approximations or ignore the metadata.
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
/// // Subtle blur for overlays
/// let overlay = text!("Overlay").background(Material::UltraThin);
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

impl Background {
    /// Creates a new background with a solid color.
    ///
    /// # Arguments
    ///
    /// * `color` - A value that can be converted into a computed color.
    pub fn color(color: impl IntoComputed<Color>) -> Self {
        Self::Color(color.into_computed())
    }

    /// Creates a new background with a blur material effect.
    ///
    /// # Arguments
    ///
    /// * `material` - The material type (e.g., `Material::Regular`, `Material::Thin`)
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Frosted glass effect
    /// let frosted = text!("Hello").background(Material::Regular);
    ///
    /// // Subtle overlay blur
    /// let subtle = text!("Overlay").background(Material::UltraThin);
    /// ```
    #[must_use]
    pub const fn material(material: Material) -> Self {
        Self::Material(material)
    }

    /// Creates a linear gradient background.
    ///
    /// # Arguments
    ///
    /// * `stops` - Color stops defining the gradient colors
    /// * `start` - Starting point of the gradient
    /// * `end` - Ending point of the gradient
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::background::Background;
    /// use waterui::gradient::ColorStop;
    ///
    /// let gradient = Background::linear_gradient(
    ///     vec![
    ///         ColorStop::new(Color::red(), 0.0),
    ///         ColorStop::new(Color::blue(), 1.0),
    ///     ],
    ///     UnitPoint::TOP,
    ///     UnitPoint::BOTTOM,
    /// );
    /// ```
    pub fn linear_gradient(
        stops: Vec<ColorStop>,
        start: impl Into<UnitPoint>,
        end: impl Into<UnitPoint>,
    ) -> Self {
        Self::Gradient(Gradient::Linear(LinearGradient::new(stops, start, end)))
    }

    /// Creates a radial gradient background.
    ///
    /// # Arguments
    ///
    /// * `stops` - Color stops defining the gradient colors
    /// * `center` - Center point of the gradient
    /// * `start_radius` - Inner radius (0.0 = point at center)
    /// * `end_radius` - Outer radius (fraction of view size)
    pub fn radial_gradient(
        stops: Vec<ColorStop>,
        center: impl Into<UnitPoint>,
        start_radius: f32,
        end_radius: f32,
    ) -> Self {
        Self::Gradient(Gradient::Radial(RadialGradient::new(
            stops,
            center,
            start_radius,
            end_radius,
        )))
    }

    /// Creates an angular (conic) gradient background.
    ///
    /// # Arguments
    ///
    /// * `stops` - Color stops defining the gradient colors
    /// * `center` - Center point of the gradient
    /// * `start_angle` - Starting angle in radians
    /// * `end_angle` - Ending angle in radians
    pub fn angular_gradient(
        stops: Vec<ColorStop>,
        center: impl Into<UnitPoint>,
        start_angle: f32,
        end_angle: f32,
    ) -> Self {
        Self::Gradient(Gradient::Angular(AngularGradient::new(
            stops,
            center,
            start_angle,
            end_angle,
        )))
    }

    /// Creates a mesh gradient background.
    ///
    /// # Arguments
    ///
    /// * `width` - Number of columns in the vertex grid
    /// * `height` - Number of rows in the vertex grid
    /// * `vertices` - Vertices arranged row by row (width × height total)
    #[must_use]
    pub fn mesh_gradient(width: u32, height: u32, vertices: Vec<MeshVertex>) -> Self {
        Self::Gradient(Gradient::Mesh(MeshGradient::new(width, height, vertices)))
    }
}

// Gradient From implementations
impl From<LinearGradient> for Background {
    fn from(gradient: LinearGradient) -> Self {
        Self::Gradient(Gradient::Linear(gradient))
    }
}

impl From<RadialGradient> for Background {
    fn from(gradient: RadialGradient) -> Self {
        Self::Gradient(Gradient::Radial(gradient))
    }
}

impl From<AngularGradient> for Background {
    fn from(gradient: AngularGradient) -> Self {
        Self::Gradient(Gradient::Angular(gradient))
    }
}

impl From<MeshGradient> for Background {
    fn from(gradient: MeshGradient) -> Self {
        Self::Gradient(Gradient::Mesh(gradient))
    }
}

impl From<Gradient> for Background {
    fn from(gradient: Gradient) -> Self {
        Self::Gradient(gradient)
    }
}
