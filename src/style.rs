//! This module provides types for working with shadows and vectors in the UI system.
//!
//! # Examples
//!
//! ```
//! use waterui::prelude::*;
//! use waterui::style;
//! use waterui_color::Color;
//!
//! fn shadow_example() {
//!     let shadow = style::Shadow {
//!         color: Color::srgb(0, 0, 0),
//!         offset: style::Vector { x: 2.0, y: 2.0 },
//!         radius: 4.0,
//!     };
//! }
//! ```

use waterui_color::Color;
use waterui_core::metadata::MetadataKey;

/// Represents a shadow effect that can be applied to UI elements.
///
/// A shadow is defined by its color, offset from the original element,
/// and blur radius.
#[derive(Debug)]
pub struct Shadow {
    /// The color of the shadow, including alpha for opacity.
    pub color: Color,
    /// The offset of the shadow from the original element.
    pub offset: Vector<f32>,
    /// The blur radius of the shadow in pixels.
    pub radius: f32,
}

impl MetadataKey for Shadow {}

impl Shadow {
    /// Creates a new shadow with the specified color, offset, and radius.
    ///
    /// # Arguments
    ///
    /// * `color` - The color of the shadow
    /// * `offset` - The offset of the shadow from the original element
    /// * `radius` - The blur radius of the shadow in pixels
    #[must_use]
    pub const fn new(color: Color, offset: Vector<f32>, radius: f32) -> Self {
        Self {
            color,
            offset,
            radius,
        }
    }

    /// Creates a shadow with the same offset and radius for both x and y directions.
    ///
    /// The color is set to black by default.
    /// # Arguments
    /// * `value` - The value to set for both offset and radius
    #[must_use]
    pub fn splat(value: f32) -> Self {
        Self {
            color: Color::srgb(0, 0, 0),
            offset: Vector { x: value, y: value },
            radius: value,
        }
    }
}

impl Default for Shadow {
    fn default() -> Self {
        Self {
            color: Color::srgb(0, 0, 0),       // Default to black shadow
            offset: Vector { x: 0.0, y: 2.0 }, // Slightly below the element
            radius: 4.0,                       // Moderate blur
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
impl<T: Into<f64>> From<T> for Shadow {
    fn from(value: T) -> Self {
        let v = value.into() as f32;
        Self {
            color: Color::srgb(0, 0, 0),
            offset: Vector { x: 0.0, y: v },
            radius: v,
        }
    }
}

/// A 2D vector with x and y components.
///
/// This type is used to represent positions, sizes, and offsets
/// in the UI coordinate system.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Vector<T> {
    /// The x component of the vector.
    pub x: T,
    /// The y component of the vector.
    pub y: T,
}

impl<T> Vector<T> {
    /// Creates a new vector with the given x and y components.
    ///
    /// # Arguments
    ///
    /// * `x` - The x component
    /// * `y` - The y component
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }

    /// Creates a vector with both components set to the same value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to set for both x and y components
    pub const fn splat(value: T) -> Self
    where
        T: Copy,
    {
        Self { x: value, y: value }
    }
    /// Maps the components of the vector using the provided function.
    ///
    /// # Arguments
    /// * `f` - A function that takes a component and returns a new value
    ///
    /// # Returns
    /// A new vector with the mapped components
    pub fn map<U, F: FnMut(T) -> U>(self, mut f: F) -> Vector<U> {
        Vector {
            x: f(self.x),
            y: f(self.y),
        }
    }
}

// ============================================================================
// Transform
// ============================================================================

use nami::Computed;
use nami::signal::IntoComputed;

/// A 2D transform that can be applied to views as metadata.
///
/// Transforms are purely visual and do not affect layout calculations.
/// They are applied after layout, making them ideal for animations.
///
/// # Example
///
/// ```rust,ignore
/// use waterui::prelude::*;
/// use waterui::style::Transform;
///
/// // Scale a view to 150%
/// Color::red()
///     .width(100.0)
///     .height(100.0)
///     .transform(Transform::scale(1.5));
///
/// // Rotate a view 45 degrees
/// Color::blue()
///     .width(50.0)
///     .height(50.0)
///     .transform(Transform::rotation(45.0));
///
/// // Animate a transform
/// let scale = binding(1.0).animated();
/// Color::green()
///     .width(80.0)
///     .height(80.0)
///     .transform(Transform::scale(scale));
/// ```
#[derive(Debug)]
pub struct Transform {
    /// Scale factor along the X axis (1.0 = no scale)
    pub scale_x: Computed<f32>,
    /// Scale factor along the Y axis (1.0 = no scale)
    pub scale_y: Computed<f32>,
    /// Rotation angle in degrees (positive = clockwise)
    pub rotation: Computed<f32>,
    /// Translation offset along the X axis in points
    pub translate_x: Computed<f32>,
    /// Translation offset along the Y axis in points
    pub translate_y: Computed<f32>,
}

impl MetadataKey for Transform {}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

impl Transform {
    /// Creates an identity transform (no transformation).
    #[must_use]
    pub fn identity() -> Self {
        Self {
            scale_x: Computed::new(1.0_f32),
            scale_y: Computed::new(1.0_f32),
            rotation: Computed::new(0.0_f32),
            translate_x: Computed::new(0.0_f32),
            translate_y: Computed::new(0.0_f32),
        }
    }

    /// Creates a uniform scale transform.
    ///
    /// # Arguments
    /// * `scale` - The scale factor (1.0 = no scale, 2.0 = double size, 0.5 = half size)
    #[must_use]
    pub fn scale(scale: impl IntoComputed<f32>) -> Self {
        let scale = scale.into_computed();
        Self {
            scale_x: scale.clone(),
            scale_y: scale,
            rotation: Computed::new(0.0_f32),
            translate_x: Computed::new(0.0_f32),
            translate_y: Computed::new(0.0_f32),
        }
    }

    /// Creates a non-uniform scale transform.
    ///
    /// # Arguments
    /// * `x` - Scale factor along the X axis
    /// * `y` - Scale factor along the Y axis
    #[must_use]
    pub fn scale_xy(x: impl IntoComputed<f32>, y: impl IntoComputed<f32>) -> Self {
        Self {
            scale_x: x.into_computed(),
            scale_y: y.into_computed(),
            rotation: Computed::new(0.0_f32),
            translate_x: Computed::new(0.0_f32),
            translate_y: Computed::new(0.0_f32),
        }
    }

    /// Creates a rotation transform.
    ///
    /// # Arguments
    /// * `degrees` - Rotation angle in degrees (positive = clockwise)
    #[must_use]
    pub fn rotation(degrees: impl IntoComputed<f32>) -> Self {
        Self {
            scale_x: Computed::new(1.0_f32),
            scale_y: Computed::new(1.0_f32),
            rotation: degrees.into_computed(),
            translate_x: Computed::new(0.0_f32),
            translate_y: Computed::new(0.0_f32),
        }
    }

    /// Creates a translation transform.
    ///
    /// # Arguments
    /// * `x` - Offset in points along the X axis
    /// * `y` - Offset in points along the Y axis
    #[must_use]
    pub fn translation(x: impl IntoComputed<f32>, y: impl IntoComputed<f32>) -> Self {
        Self {
            scale_x: Computed::new(1.0_f32),
            scale_y: Computed::new(1.0_f32),
            rotation: Computed::new(0.0_f32),
            translate_x: x.into_computed(),
            translate_y: y.into_computed(),
        }
    }

    /// Sets the scale factors.
    #[must_use]
    pub fn with_scale(mut self, x: impl IntoComputed<f32>, y: impl IntoComputed<f32>) -> Self {
        self.scale_x = x.into_computed();
        self.scale_y = y.into_computed();
        self
    }

    /// Sets the rotation angle.
    #[must_use]
    pub fn with_rotation(mut self, degrees: impl IntoComputed<f32>) -> Self {
        self.rotation = degrees.into_computed();
        self
    }

    /// Sets the translation offset.
    #[must_use]
    pub fn with_translation(mut self, x: impl IntoComputed<f32>, y: impl IntoComputed<f32>) -> Self {
        self.translate_x = x.into_computed();
        self.translate_y = y.into_computed();
        self
    }
}
