//! Layout components and utilities for the `WaterUI` framework.
//!
//! This crate provides comprehensive layout capabilities including stacks,
//! grids, overlays, scrolling, spacing, and flexible frame-based sizing.

extern crate alloc;

/// Stack layout components for arranging views linearly or in layers.
pub mod stack;

/// Layout engine for container-based layouts.
pub mod engine;

/// Grid layout components for two-dimensional arrangements.
pub mod grid;
pub use grid::row;
/// Overlay components for layered content.
pub mod overlay;
pub use overlay::overlay;
/// Scroll view components for scrollable content.
pub mod scroll;
pub use scroll::scroll;
/// Spacer components for adding flexible space.
pub mod spacer;
pub use spacer::spacer;

/// Alignment options for layout positioning.
///
/// Defines how content should be aligned within its container.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Alignment {
    /// Default alignment (platform-specific).
    #[default]
    Default,
    /// Align to the leading edge (left in LTR, right in RTL).
    Leading,
    /// Center alignment.
    Center,
    /// Align to the trailing edge (right in LTR, left in RTL).
    Trailing,
}

/// Frame configuration for view sizing and positioning.
///
/// This struct contains all the properties that define how a view
/// should be sized and positioned within its container.
#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Frame {
    /// The minimum size constraints for the frame.
    pub min_size: Size,
    /// The preferred size of the frame.
    pub size: Size,

    /// The maximum size constraints for the frame.
    pub max_size: Size,

    /// The alignment of the frame within its container.
    pub alignment: Alignment,
}

/// Represents a 2D size with width and height dimensions.
///
/// This is used throughout the layout system to represent dimensions,
/// constraints, and measurements.
#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Size {
    /// The width of the size.
    pub width: f64,
    /// The height of the size.
    pub height: f64,
}

/// Represents a 2D point with x and y coordinates.
///
/// Used for positioning elements within the layout system.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Point {
    /// The x-coordinate of the point.
    pub x: f64,
    /// The y-coordinate of the point.
    pub y: f64,
}

/// Represents a rectangle defined by an origin point and size.
///
/// Used for defining bounds and areas within the layout system.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Rectangle {
    /// The origin point (top-left corner) of the rectangle.
    pub origin: Point,
    /// The size (width and height) of the rectangle.
    pub size: Size,
}

impl Frame {
    /// Creates a new frame with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Edge spacing configuration for margins, padding, etc.
///
/// This struct defines spacing values for all four edges of a rectangle,
/// commonly used for margins, padding, and border spacing.
#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Edge {
    /// Spacing for the top edge.
    pub top: f64,
    /// Spacing for the right edge.
    pub right: f64,
    /// Spacing for the bottom edge.
    pub bottom: f64,
    /// Spacing for the left edge.
    pub left: f64,
}

impl Edge {
    /// Creates a new `Edge` with all sides set to zero.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        }
    }

    /// Creates an `Edge` with spacing for vertical sides (top and bottom).
    #[must_use]
    pub fn vertical(size: impl Into<f64>) -> Self {
        let size = size.into();
        Self::zero().left(size).right(size)
    }
    /// Creates an `Edge` with spacing for horizontal sides (left and right).
    #[must_use]
    pub fn horizontal(size: impl Into<f64>) -> Self {
        let size = size.into();

        Self::zero().top(size).bottom(size)
    }

    /// Creates an `Edge` with equal spacing on all sides.
    #[must_use]
    pub fn round(size: impl Into<f64>) -> Self {
        let size = size.into();

        Self::vertical(size).left(size).right(size)
    }
}

macro_rules! modify_field {
    ($ident:ident,$ty:ty) => {
        /// Sets the `$ident` field of the frame to the specified size.
        #[must_use]
        pub fn $ident(mut self, size: impl Into<$ty>) -> Self {
            self.$ident = size.into();
            self
        }
    };
}

impl Edge {
    modify_field!(top, f64);
    modify_field!(left, f64);
    modify_field!(right, f64);
    modify_field!(bottom, f64);
}

impl Frame {
    modify_field!(alignment, Alignment);
}

impl Frame {
    /// Sets the width of the frame.
    #[must_use]
    pub fn width(mut self, size: impl Into<f64>) -> Self {
        self.size.width = size.into();
        self
    }

    /// Sets the minimum width constraint of the frame.
    #[must_use]
    pub fn min_width(mut self, size: impl Into<f64>) -> Self {
        self.min_size.width = size.into();
        self
    }

    /// Sets the maximum width constraint of the frame.
    #[must_use]
    pub fn max_width(mut self, size: impl Into<f64>) -> Self {
        self.max_size.width = size.into();
        self
    }

    /// Sets the height of the frame.
    #[must_use]
    pub fn height(mut self, size: impl Into<f64>) -> Self {
        self.size.height = size.into();
        self
    }

    /// Sets the minimum height constraint of the frame.
    #[must_use]
    pub fn min_height(mut self, size: impl Into<f64>) -> Self {
        self.min_size.height = size.into();
        self
    }

    /// Sets the maximum height constraint of the frame.
    #[must_use]
    pub fn max_height(mut self, size: impl Into<f64>) -> Self {
        self.max_size.height = size.into();
        self
    }

    /// Sets both width and height of the frame.
    #[must_use]
    pub fn size(mut self, width: impl Into<f64>, height: impl Into<f64>) -> Self {
        self.size.width = width.into();
        self.size.height = height.into();
        self
    }

    /// Sets both minimum width and height constraints of the frame.
    #[must_use]
    pub fn min_size(mut self, width: impl Into<f64>, height: impl Into<f64>) -> Self {
        self.min_size.width = width.into();
        self.min_size.height = height.into();
        self
    }

    /// Sets both maximum width and height constraints of the frame.
    #[must_use]
    pub fn max_size(mut self, width: impl Into<f64>, height: impl Into<f64>) -> Self {
        self.max_size.width = width.into();
        self.max_size.height = height.into();
        self
    }
}
