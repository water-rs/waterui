//! Background and foreground styling for views.
//!
//! Backgrounds fill the bounds behind a view, distinct from `Shape::fill()` which fills
//! the shape itself. Backgrounds support solid colors and blur effects (materials).
//!
//! # Ordering with Clip Shapes
//!
//! When creating rounded cards or clipped views with backgrounds, the order matters:
//! - Apply `.background()` first to set the background
//! - Then apply `.clip_shape()` to clip both the view and its background
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//! use waterui::shape::RoundedRectangle;
//!
//! // Correct: background first, then clip
//! text!("Hello")
//!     .padding()
//!     .background(Color::blue())
//!     .clip_shape(RoundedRectangle::new(12.0));
//!
//! // For a frosted glass effect
//! content
//!     .background(Material::Regular)
//!     .clip_shape(RoundedRectangle::new(16.0));
//! ```

use nami::signal::IntoComputed;
use waterui_color::{Color, Srgb};
use waterui_core::{Computed, metadata::MetadataKey};
use waterui_str::Str;

/// A background that fills the bounds behind a view.
///
/// Unlike `Shape::fill()` which fills a shape's area, backgrounds fill the entire
/// rectangular bounds of the view they're applied to. Use backgrounds for:
/// - Solid color fills behind content
/// - Blur/frosted glass effects (materials)
/// - Image backgrounds
///
/// # Examples
///
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// // Solid color background
/// text!("Hello").background(Color::red());
///
/// // Blur material (frosted glass effect)
/// content.background(Material::Regular);
/// ```
#[derive(Debug)]
pub enum Background {
    /// A solid color background.
    Color(Computed<Color>),
    /// An image background.
    Image(Computed<Str>),
    /// A material background (blur effects, vibrancy).
    Material(Material),
    /// WebGPU shader background (not yet implemented).
    Shader(Shader),
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
