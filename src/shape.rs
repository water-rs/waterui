//! Shape system for view clipping.
//!
//! This module provides a trait-based system for defining shapes that can be used
//! to clip views. Shapes generate normalized path commands (0.0-1.0 coordinates)
//! that scale with view bounds.
//!
//! # Example
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//! use waterui::shape::*;
//!
//! // Clip to a circle
//! image("avatar.jpg").clip(Circle);
//!
//! // Clip to rounded rectangle
//! card.clip(RoundedRectangle::new(0.1));
//!
//! // Custom path
//! let triangle = Path::new()
//!     .move_to(0.5, 0.0)
//!     .line_to(1.0, 1.0)
//!     .line_to(0.0, 1.0)
//!     .close();
//! view.clip(triangle);
//! ```

use core::f32::consts::{FRAC_PI_2, PI, TAU};

use waterui_color::Color;
use waterui_core::{layout::StretchAxis, metadata::MetadataKey, raw_view};

// ============================================================================
// PathCommand - The primitive operations for drawing paths
// ============================================================================

/// A single path command for drawing shapes.
///
/// All coordinates are normalized (0.0-1.0) and scale with view bounds.
/// Native backends convert these to absolute coordinates based on view size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCommand {
    /// Move to a position without drawing.
    MoveTo {
        /// X coordinate (normalized 0.0-1.0)
        x: f32,
        /// Y coordinate (normalized 0.0-1.0)
        y: f32,
    },

    /// Draw a straight line to a position.
    LineTo {
        /// X coordinate (normalized 0.0-1.0)
        x: f32,
        /// Y coordinate (normalized 0.0-1.0)
        y: f32,
    },

    /// Draw a quadratic bezier curve.
    QuadTo {
        /// Control point x
        cx: f32,
        /// Control point y
        cy: f32,
        /// End point x
        x: f32,
        /// End point y
        y: f32,
    },

    /// Draw a cubic bezier curve.
    CubicTo {
        /// First control point x
        c1x: f32,
        /// First control point y
        c1y: f32,
        /// Second control point x
        c2x: f32,
        /// Second control point y
        c2y: f32,
        /// End point x
        x: f32,
        /// End point y
        y: f32,
    },

    /// Draw an arc.
    Arc {
        /// Center x (normalized)
        cx: f32,
        /// Center y (normalized)
        cy: f32,
        /// Radius x (normalized, relative to width)
        rx: f32,
        /// Radius y (normalized, relative to height)
        ry: f32,
        /// Start angle in radians
        start: f32,
        /// Sweep angle in radians (positive = clockwise)
        sweep: f32,
    },

    /// Close the current subpath by drawing a line to the start.
    Close,
}

// ============================================================================
// Shape Trait
// ============================================================================

/// A trait for types that can produce path commands for clipping.
///
/// All coordinates are normalized (0.0-1.0) and scale with view bounds.
/// Built-in shapes use stack-allocated arrays for zero heap allocation.
pub trait Shape {
    /// The iterator type returned by `path()`.
    type Iter: IntoIterator<Item = PathCommand>;

    /// Returns the path commands that define this shape.
    fn path(&self) -> Self::Iter;
}

// ============================================================================
// Common Shape Implementations
// ============================================================================

/// A circle inscribed in the view bounds.
///
/// Uses the minimum of width/height as the diameter, centered in the view.
#[derive(Debug, Clone, Copy, Default)]
pub struct Circle;

impl Shape for Circle {
    type Iter = [PathCommand; 1];

    fn path(&self) -> Self::Iter {
        // Full circle: arc sweeping 360 degrees around center (0.5, 0.5)
        [PathCommand::Arc {
            cx: 0.5,
            cy: 0.5,
            rx: 0.5,
            ry: 0.5,
            start: 0.0,
            sweep: TAU,
        }]
    }
}

/// An ellipse that fills the view bounds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ellipse;

impl Shape for Ellipse {
    type Iter = [PathCommand; 1];

    fn path(&self) -> Self::Iter {
        [PathCommand::Arc {
            cx: 0.5,
            cy: 0.5,
            rx: 0.5,
            ry: 0.5,
            start: 0.0,
            sweep: TAU,
        }]
    }
}

/// A capsule (pill) shape - rectangle with fully rounded ends.
///
/// The rounded ends use semicircles based on the shorter dimension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Capsule;

impl Shape for Capsule {
    type Iter = [PathCommand; 4];

    fn path(&self) -> Self::Iter {
        // Capsule in normalized coords (aspect ratio handled by native)
        [
            PathCommand::MoveTo { x: 0.5, y: 0.0 },
            PathCommand::Arc {
                cx: 0.5,
                cy: 0.5,
                rx: 0.5,
                ry: 0.5,
                start: -FRAC_PI_2,
                sweep: PI,
            },
            PathCommand::Arc {
                cx: 0.5,
                cy: 0.5,
                rx: 0.5,
                ry: 0.5,
                start: FRAC_PI_2,
                sweep: PI,
            },
            PathCommand::Close,
        ]
    }
}

/// A rectangle with uniform corner radius.
#[derive(Debug, Clone, Copy)]
pub struct RoundedRectangle {
    /// Corner radius (normalized, 0.0-0.5 range).
    pub corner_radius: f32,
}

impl RoundedRectangle {
    /// Creates a new rounded rectangle with the given corner radius.
    ///
    /// # Arguments
    /// * `corner_radius` - Normalized corner radius (0.0-0.5 range)
    #[must_use]
    pub const fn new(corner_radius: f32) -> Self {
        Self { corner_radius }
    }
}

impl Shape for RoundedRectangle {
    type Iter = [PathCommand; 10];

    fn path(&self) -> Self::Iter {
        let r = self.corner_radius;
        [
            PathCommand::MoveTo { x: r, y: 0.0 },
            PathCommand::LineTo { x: 1.0 - r, y: 0.0 },
            PathCommand::Arc {
                cx: 1.0 - r,
                cy: r,
                rx: r,
                ry: r,
                start: -FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: 1.0, y: 1.0 - r },
            PathCommand::Arc {
                cx: 1.0 - r,
                cy: 1.0 - r,
                rx: r,
                ry: r,
                start: 0.0,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: r, y: 1.0 },
            PathCommand::Arc {
                cx: r,
                cy: 1.0 - r,
                rx: r,
                ry: r,
                start: FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: 0.0, y: r },
            PathCommand::Arc {
                cx: r,
                cy: r,
                rx: r,
                ry: r,
                start: PI,
                sweep: FRAC_PI_2,
            },
            PathCommand::Close,
        ]
    }
}

/// A rectangle with independent corner radii.
#[derive(Debug, Clone, Copy)]
pub struct UnevenRoundedRectangle {
    /// Top-leading corner radius (normalized).
    pub top_leading: f32,
    /// Top-trailing corner radius (normalized).
    pub top_trailing: f32,
    /// Bottom-leading corner radius (normalized).
    pub bottom_leading: f32,
    /// Bottom-trailing corner radius (normalized).
    pub bottom_trailing: f32,
}

impl UnevenRoundedRectangle {
    /// Creates a new uneven rounded rectangle with independent corner radii.
    #[must_use]
    pub const fn new(
        top_leading: f32,
        top_trailing: f32,
        bottom_leading: f32,
        bottom_trailing: f32,
    ) -> Self {
        Self {
            top_leading,
            top_trailing,
            bottom_leading,
            bottom_trailing,
        }
    }
}

impl Shape for UnevenRoundedRectangle {
    type Iter = [PathCommand; 10];

    fn path(&self) -> Self::Iter {
        let tl = self.top_leading;
        let tr = self.top_trailing;
        let bl = self.bottom_leading;
        let br = self.bottom_trailing;
        [
            PathCommand::MoveTo { x: tl, y: 0.0 },
            PathCommand::LineTo { x: 1.0 - tr, y: 0.0 },
            PathCommand::Arc {
                cx: 1.0 - tr,
                cy: tr,
                rx: tr,
                ry: tr,
                start: -FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: 1.0, y: 1.0 - br },
            PathCommand::Arc {
                cx: 1.0 - br,
                cy: 1.0 - br,
                rx: br,
                ry: br,
                start: 0.0,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: bl, y: 1.0 },
            PathCommand::Arc {
                cx: bl,
                cy: 1.0 - bl,
                rx: bl,
                ry: bl,
                start: FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: 0.0, y: tl },
            PathCommand::Arc {
                cx: tl,
                cy: tl,
                rx: tl,
                ry: tl,
                start: PI,
                sweep: FRAC_PI_2,
            },
            PathCommand::Close,
        ]
    }
}

/// A simple rectangle with sharp corners.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rectangle;

impl Shape for Rectangle {
    type Iter = [PathCommand; 5];

    fn path(&self) -> Self::Iter {
        [
            PathCommand::MoveTo { x: 0.0, y: 0.0 },
            PathCommand::LineTo { x: 1.0, y: 0.0 },
            PathCommand::LineTo { x: 1.0, y: 1.0 },
            PathCommand::LineTo { x: 0.0, y: 1.0 },
            PathCommand::Close,
        ]
    }
}

// ============================================================================
// Custom Path Builder
// ============================================================================

/// A custom path defined by explicit commands.
///
/// Use this for arbitrary shapes that aren't covered by the built-in shapes.
/// Note: This uses heap allocation internally since the number of commands
/// is not known at compile time.
#[derive(Debug, Clone, Default)]
pub struct Path {
    commands: Vec<PathCommand>,
}

impl Path {
    /// Creates a new empty path.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves to a position without drawing.
    #[must_use]
    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::MoveTo { x, y });
        self
    }

    /// Draws a straight line to a position.
    #[must_use]
    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::LineTo { x, y });
        self
    }

    /// Draws a quadratic bezier curve.
    #[must_use]
    pub fn quad_to(mut self, cx: f32, cy: f32, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::QuadTo { cx, cy, x, y });
        self
    }

    /// Draws a cubic bezier curve.
    #[must_use]
    pub fn cubic_to(mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> Self {
        self.commands
            .push(PathCommand::CubicTo { c1x, c1y, c2x, c2y, x, y });
        self
    }

    /// Draws an arc.
    #[must_use]
    pub fn arc(mut self, cx: f32, cy: f32, rx: f32, ry: f32, start: f32, sweep: f32) -> Self {
        self.commands.push(PathCommand::Arc {
            cx,
            cy,
            rx,
            ry,
            start,
            sweep,
        });
        self
    }

    /// Closes the current subpath.
    #[must_use]
    pub fn close(mut self) -> Self {
        self.commands.push(PathCommand::Close);
        self
    }
}

impl Shape for Path {
    type Iter = alloc::vec::IntoIter<PathCommand>;

    fn path(&self) -> Self::Iter {
        self.commands.clone().into_iter()
    }
}

// ============================================================================
// ClipShape Metadata
// ============================================================================

/// Metadata for clipping a view to a shape.
///
/// This collects the path commands from a Shape and stores them for FFI.
#[derive(Debug)]
pub struct ClipShape {
    commands: Vec<PathCommand>,
}

impl ClipShape {
    /// Creates a new clip shape from any type implementing Shape.
    pub fn new(shape: impl Shape) -> Self {
        Self {
            commands: shape.path().into_iter().collect(),
        }
    }

    /// Returns the path commands.
    #[must_use]
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }
}

impl MetadataKey for ClipShape {}

// ============================================================================
// FilledShape - Shape as a View with fill color
// ============================================================================

/// A shape filled with a color, rendered as a native view.
///
/// `FilledShape` combines a shape definition with a fill color, creating a view
/// that fills available space (like SwiftUI's Shape views).
///
/// # Layout Behavior
///
/// FilledShape is a **greedy view** that expands to fill all available space in both
/// directions, just like `Color`. Use `.frame()` to constrain its size.
///
/// # Example
///
/// ```rust,ignore
/// use waterui::prelude::*;
/// use waterui::shape::*;
///
/// // A red circle that fills available space
/// Circle.fill(Color::red())
///
/// // A blue rounded rectangle with fixed size
/// RoundedRectangle::new(0.1)
///     .fill(Color::blue())
///     .frame()
///     .size(100.0, 50.0)
/// ```
#[derive(Debug)]
pub struct FilledShape {
    /// Path commands defining the shape.
    commands: Vec<PathCommand>,
    /// Fill color for the shape.
    fill: Color,
}

impl FilledShape {
    /// Creates a new filled shape from a shape and color.
    pub fn new(shape: impl Shape, fill: impl Into<Color>) -> Self {
        Self {
            commands: shape.path().into_iter().collect(),
            fill: fill.into(),
        }
    }

    /// Returns the path commands.
    #[must_use]
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }

    /// Returns the fill color.
    #[must_use]
    pub fn fill(&self) -> &Color {
        &self.fill
    }
}

// FilledShape is a native view that fills available space
raw_view!(FilledShape, StretchAxis::Both);

// ============================================================================
// ShapeExt - Extension trait for adding fill to shapes
// ============================================================================

/// Extension trait for filling shapes with color.
///
/// This trait provides the `.fill()` method for any type that implements `Shape`.
pub trait ShapeExt: Shape + Sized {
    /// Fills the shape with the specified color.
    ///
    /// Returns a `FilledShape` view that can be used in the view hierarchy.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::shape::*;
    ///
    /// Circle.fill(Color::red())
    /// ```
    fn fill(self, color: impl Into<Color>) -> FilledShape {
        FilledShape::new(self, color)
    }
}

// Implement ShapeExt for all Shape types
impl<S: Shape + Sized> ShapeExt for S {}
