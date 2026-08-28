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
//! let avatar = Color::blue().clip(Circle);
//!
//! // Clip to rounded rectangle
//! let card = text!("Card").clip(RoundedRectangle::new(0.1));
//!
//! // Fill a shape with color
//! let dot = Circle.fill(Color::red());
//! ```

pub use waterui_shape::*;
