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

use uniffi::custom_type;
use waterui_core::Color;

/// Represents a shadow effect that can be applied to UI elements.
///
/// A shadow is defined by its color, offset from the original element,
/// and blur radius.
#[derive(Debug, PartialEq, uniffi::Record)]
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

mod ffi {
    use uniffi::custom_type;

    use super::Vector;
    type VectorFloat = Vector<f32>;

    #[derive(Debug, uniffi::Record)]
    pub struct FFIVectorFloat {
        x: f32,
        y: f32,
    }

    impl From<FFIVectorFloat> for VectorFloat {
        fn from(value: FFIVectorFloat) -> Self {
            VectorFloat {
                x: value.x,
                y: value.y,
            }
        }
    }

    impl From<VectorFloat> for FFIVectorFloat {
        fn from(value: VectorFloat) -> Self {
            FFIVectorFloat {
                x: value.x,
                y: value.y,
            }
        }
    }

    custom_type!(VectorFloat, FFIVectorFloat);
}
