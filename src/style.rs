//! This module provides types for working with shadows and vectors in the UI system.
//!
//! # Examples
//!
//! ```
//! use waterui::prelude::*;
//! use waterui::style;
//! use waterui_graphics::color::Color;
//!
//! fn shadow_example() {
//!     let shadow = style::Shadow {
//!         color: Color::srgb(0, 0, 0),
//!         offset: style::Vector { x: 2.0, y: 2.0 },
//!         radius: 4.0,
//!     };
//! }
//! ```

use nami::{Computed, SignalExt};
use waterui_core::{IntoSignalF32, metadata::MetadataKey};
use waterui_graphics::color::Color;

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
// Transform Components (Scale, Rotation, Offset)
// ============================================================================

/// Anchor point for transforms, specified as normalized coordinates.
/// (0.0, 0.0) = top-left, (0.5, 0.5) = center, (1.0, 1.0) = bottom-right
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    /// X coordinate (0.0 = left, 0.5 = center, 1.0 = right)
    pub x: f32,
    /// Y coordinate (0.0 = top, 0.5 = center, 1.0 = bottom)
    pub y: f32,
}

impl Anchor {
    /// Center anchor point (0.5, 0.5)
    pub const CENTER: Self = Self { x: 0.5, y: 0.5 };
    /// Top-left anchor point (0.0, 0.0)
    pub const TOP_LEFT: Self = Self { x: 0.0, y: 0.0 };
    /// Top-right anchor point (1.0, 0.0)
    pub const TOP_RIGHT: Self = Self { x: 1.0, y: 0.0 };
    /// Bottom-left anchor point (0.0, 1.0)
    pub const BOTTOM_LEFT: Self = Self { x: 0.0, y: 1.0 };
    /// Bottom-right anchor point (1.0, 1.0)
    pub const BOTTOM_RIGHT: Self = Self { x: 1.0, y: 1.0 };

    /// Creates a custom anchor point.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl Default for Anchor {
    fn default() -> Self {
        Self::CENTER
    }
}

// ============================================================================
// Scale Metadata
// ============================================================================

/// Scale transform that can be applied to views.
///
/// Scales are purely visual and do not affect layout calculations.
/// The scale is applied around the specified anchor point.
///
/// # Example
///
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// // Scale from center (default)
/// view.scale(1.5)
///
/// // Scale from top-left corner
/// view.scale_from(1.5, Anchor::TOP_LEFT)
///
/// // Animate scale
/// let factor = Binding::container(1.0).with(Animation::spring(300.0, 15.0));
/// view.scale(factor)
/// ```
#[derive(Debug)]
pub struct Scale {
    /// Scale factor along X axis (1.0 = no scale)
    pub x: Computed<f32>,
    /// Scale factor along Y axis (1.0 = no scale)
    pub y: Computed<f32>,
    /// Anchor point for the scale transform
    pub anchor: Anchor,
}

impl MetadataKey for Scale {}

impl Scale {
    /// Creates a uniform scale transform from center.
    #[must_use]
    pub fn uniform(factor: impl IntoSignalF32) -> Self {
        let factor = factor.into_signal_f32();
        Self {
            x: factor.clone().computed(),
            y: factor.computed(),
            anchor: Anchor::CENTER,
        }
    }

    /// Creates a uniform scale transform from a specific anchor point.
    #[must_use]
    pub fn uniform_from(factor: impl IntoSignalF32, anchor: Anchor) -> Self {
        let factor = factor.into_signal_f32();
        Self {
            x: factor.clone().computed(),
            y: factor.computed(),
            anchor,
        }
    }

    /// Creates a non-uniform scale transform from center.
    #[must_use]
    pub fn xy(x: impl IntoSignalF32, y: impl IntoSignalF32) -> Self {
        Self {
            x: x.into_signal_f32().computed(),
            y: y.into_signal_f32().computed(),
            anchor: Anchor::CENTER,
        }
    }

    /// Creates a non-uniform scale transform from a specific anchor point.
    #[must_use]
    pub fn xy_from(x: impl IntoSignalF32, y: impl IntoSignalF32, anchor: Anchor) -> Self {
        Self {
            x: x.into_signal_f32().computed(),
            y: y.into_signal_f32().computed(),
            anchor,
        }
    }
}

// ============================================================================
// Rotation Metadata
// ============================================================================

/// Rotation transform that can be applied to views.
///
/// Rotations are purely visual and do not affect layout calculations.
/// The rotation is applied around the specified anchor point.
///
/// # Example
///
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// // Rotate around center (default)
/// view.rotation(45.0)
///
/// // Rotate around top-left corner
/// view.rotation_from(45.0, Anchor::TOP_LEFT)
///
/// // Animate rotation
/// let angle = Binding::container(0.0).with(Animation::spring(300.0, 15.0));
/// view.rotation(angle)
/// ```
#[derive(Debug)]
pub struct Rotation {
    /// Rotation angle in degrees (positive = clockwise)
    pub angle: Computed<f32>,
    /// Anchor point for the rotation transform
    pub anchor: Anchor,
}

impl MetadataKey for Rotation {}

impl Rotation {
    /// Creates a rotation transform around center.
    #[must_use]
    pub fn degrees(angle: impl IntoSignalF32) -> Self {
        Self {
            angle: angle.into_signal_f32().computed(),
            anchor: Anchor::CENTER,
        }
    }

    /// Creates a rotation transform around a specific anchor point.
    #[must_use]
    pub fn degrees_from(angle: impl IntoSignalF32, anchor: Anchor) -> Self {
        Self {
            angle: angle.into_signal_f32().computed(),
            anchor,
        }
    }
}

// ============================================================================
// Offset Metadata
// ============================================================================

/// Offset (translation) transform that can be applied to views.
///
/// Offsets are purely visual and do not affect layout calculations.
/// This is a simple translation with no anchor point needed.
///
/// # Example
///
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// // Move view by (10, 20) points
/// view.offset(10.0, 20.0)
///
/// // Animate offset
/// let x = Binding::container(0.0).with(Animation::spring(300.0, 15.0));
/// view.offset(x, 0.0)
/// ```
#[derive(Debug)]
pub struct Offset {
    /// Offset along X axis in points
    pub x: Computed<f32>,
    /// Offset along Y axis in points
    pub y: Computed<f32>,
}

impl MetadataKey for Offset {}

impl Offset {
    /// Creates an offset transform.
    #[must_use]
    pub fn new(x: impl IntoSignalF32, y: impl IntoSignalF32) -> Self {
        Self {
            x: x.into_signal_f32().computed(),
            y: y.into_signal_f32().computed(),
        }
    }
}
