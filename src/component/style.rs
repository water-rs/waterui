//! This module provides types for working with shadows and vectors in the UI system.
//!
//! # Examples
//!
//! ```
//! use crate::Shadow;
//! use crate::Vector;
//! use waterui_core::Color;
//!
//! let shadow = Shadow {
//!     color: Color::rgba(0, 0, 0, 0.5),
//!     offset: Vector { x: 2.0, y: 2.0 },
//!     radius: 4.0,
//! };
//! ```

use waterui_core::Color;

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
