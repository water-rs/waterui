//! Shape system for view clipping and filled shapes.
//!
//! This module re-exports the shape system from `waterui-shape`.
//!
//! # Example
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::shape::*;
//!
//! // Clip to a circle
//! image("avatar.jpg").clip(Circle);
//!
//! // Clip to rounded rectangle
//! card.clip(RoundedRectangle::new(0.1));
//!
//! // Fill a shape with color
//! Circle.fill(Color::red());
//! ```

pub use waterui_shape::*;
