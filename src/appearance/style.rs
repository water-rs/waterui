//! This module provides types for working with shadows and vectors in the UI system.
//!
//! # Examples
//!
//! ```
//! use waterui::prelude::*;
//! use waterui::shape::RoundedRectangle;
//! use waterui::style;
//! use waterui_graphics::color::Color;
//!
//! fn shadow_example() {
//!     let shadow = style::Shadow::new(4.0)
//!         .color(Color::srgb(0, 0, 0))
//!         .offset(2.0, 2.0)
//!         .silhouette(RoundedRectangle::new(8.0));
//! }
//! ```

pub use waterui_shape::style::{Anchor, Offset, Rotation, Scale, Shadow, Vector};
