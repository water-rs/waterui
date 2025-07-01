//! Layout components and utilities for the `WaterUI` framework.
//!
//! This crate provides comprehensive layout capabilities including stacks,
//! grids, overlays, scrolling, spacing, and flexible frame-based sizing.

extern crate alloc;

/// Stack layout components for arranging views linearly or in layers.
pub mod stack;

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
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Frame {
    /// The preferred width of the frame.
    pub width: f64,
    /// The minimum allowed width.
    pub min_width: f64,
    /// The maximum allowed width.
    pub max_width: f64,
    /// The preferred height of the frame.
    pub height: f64,
    /// The minimum allowed height.
    pub min_height: f64,
    /// The maximum allowed height.
    pub max_height: f64,
    /// The margin spacing around the frame.
    pub margin: Edge,
    /// The alignment of the frame within its container.
    pub alignment: Alignment,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            width: f64::NAN,
            min_width: f64::NAN,
            max_width: f64::NAN,
            height: f64::NAN,
            min_height: f64::NAN,
            max_height: f64::NAN,
            margin: Edge::default(),
            alignment: Alignment::default(),
        }
    }
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
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[repr(C)]
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

impl Default for Edge {
    fn default() -> Self {
        Self {
            top: f64::NAN,
            right: f64::NAN,
            bottom: f64::NAN,
            left: f64::NAN,
        }
    }
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
    modify_field!(width, f64);
    modify_field!(min_width, f64);
    modify_field!(max_width, f64);
    modify_field!(height, f64);
    modify_field!(min_height, f64);
    modify_field!(max_height, f64);
    modify_field!(margin, Edge);
    modify_field!(alignment, Alignment);
}
