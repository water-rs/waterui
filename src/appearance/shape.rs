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
//! let card = text!("Card").clip(RoundedRectangle::new(8.0));
//!
//! // Fill a shape with color
//! let dot = Circle.fill(Color::red());
//! ```

pub use waterui_shape::{
    Angle, Capsule, Circle, ClipShape, ColorPaint, CornerRadii, CornerRadius, DirectPaint, Ellipse,
    FilledShape, GradientPaint, IntoPaint, Mask, MaskView, MorphShape, MorphableShape, Paint, Path,
    PathCommand, Rectangle, ResolvedShape, ResolvedStrokedShape, RoundedRectangle, Shape, ShapeExt,
    ShapeKind, StrokedShape,
};
