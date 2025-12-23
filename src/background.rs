//! Background and foreground styling for views.
//!
//! Backgrounds fill the bounds behind a view. They support solid colors, gradients,
//! blur effects (materials), and images.
//!
//! # Gradient Backgrounds
//!
//! Gradients are rendered using GPU shaders for consistent cross-platform appearance:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//! use waterui::gradient::{LinearGradient, ColorStop, UnitPoint};
//!
//! text!("Hello")
//!     .background(LinearGradient::new(
//!         vec![
//!             ColorStop::new(Color::red(), 0.0),
//!             ColorStop::new(Color::blue(), 1.0),
//!         ],
//!         UnitPoint::TOP,
//!         UnitPoint::BOTTOM,
//!     ));
//! ```

use nami::signal::IntoComputed;
use waterui_color::{Color, Srgb};
use waterui_core::{Computed, metadata::MetadataKey};
use waterui_str::Str;

use crate::gradient::{
    AngularGradient, ColorStop, Gradient, LinearGradient, MeshGradient, MeshVertex,
    RadialGradient, UnitPoint,
};

/// Represents different kinds of backgrounds that can be applied to UI elements.
#[derive(Debug)]
pub enum Background {
    /// A solid color background.
    Color(Computed<Color>),
    /// An image background.
    Image(Computed<Str>),
    /// A material background (blur effects).
    Material(Material),
    /// WebGPU shader background.
    Shader(Shader),
    /// A gradient background (linear, radial, angular, or mesh).
    Gradient(Gradient),
}

/// A WebGPU shader background.
///
/// Not implemented yet.
#[derive(Debug)]
pub struct Shader {}

/// Material types for background blur effects.
///
/// Materials create translucent blur effects that allow content behind the view
/// to show through with varying degrees of blur and vibrancy.
///
/// On iOS/macOS, these map to SwiftUI's `Material` types using `UIVisualEffectView`.
/// On Android API 31+, these use `RenderEffect.createBlurEffect()` with varying radii.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// // Frosted glass card
/// content
///     .padding()
///     .background(Material::Regular)
///     .clip_shape(RoundedRectangle::new(16.0));
///
/// // Subtle blur for overlays
/// overlay.background(Material::UltraThin);
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
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Frosted glass effect
    /// content.background(Material::Regular);
    ///
    /// // Subtle overlay blur
    /// overlay.background(Material::UltraThin);
    /// ```
    pub fn material(material: Material) -> Self {
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
    /// ```rust,ignore
    /// Background::linear_gradient(
    ///     vec![
    ///         ColorStop::new(Color::red(), 0.0),
    ///         ColorStop::new(Color::blue(), 1.0),
    ///     ],
    ///     UnitPoint::TOP,
    ///     UnitPoint::BOTTOM,
    /// )
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

/// Represents the color of text or other foreground elements in a UI.
#[derive(Debug)]
pub struct ForegroundColor {
    /// The computed color value.
    pub color: Computed<Color>,
}

impl MetadataKey for ForegroundColor {}

impl ForegroundColor {
    /// Creates a new foreground color.
    ///
    /// # Arguments
    ///
    /// * `color` - A value that can be converted into a computed color.
    ///
    /// # Returns
    ///
    /// A new `ForegroundColor` instance with the specified color.
    pub fn new(color: impl IntoComputed<Color>) -> Self {
        Self {
            color: color.into_computed(),
        }
    }
}
