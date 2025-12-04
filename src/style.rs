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
