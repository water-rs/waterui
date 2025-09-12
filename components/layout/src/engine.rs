// Layout engine

// for layout containers, such as Stack, Grid, Overlay, etc.

use waterui_core::{AnyView, raw_view};

use crate::Size;

/// Layout constraints that define minimum and maximum size limits.
///
/// Used by the layout engine to determine how children should be sized
/// within their container's available space.
#[derive(Debug)]
pub struct Constraint {
    /// The minimum allowed size for the layout.
    pub min: Size,
    /// The maximum allowed size for the layout.
    pub max: Size,
}

/// Represents a child element with its measured dimensions.
///
/// Contains the sizing information needed by the layout engine
/// to position and size the child appropriately.
#[derive(Debug)]
pub struct MeasuredChild {
    /// The ideal size that the child wants to be.
    pub ideal_size: Size,
    /// The minimum size that the child can be constrained to.
    pub min_size: Size,
    /// The maximum size that the child can be expanded to.
    pub max_size: Size,
}

/// The result of a layout calculation.
///
/// Contains the final size of the container and the positions
/// where each child should be placed.
#[derive(Debug)]
pub struct LayoutResult {
    /// The final size of the container after layout.
    pub size: Size,
    /// The positions where each child should be placed.
    /// Uses Size struct to store x,y coordinates as width,height.
    pub child_positions: Vec<Size>,
    /// The container view that performed the layout.
    pub child: AnyView,
}

/// Trait for implementing custom layout algorithms.
///
/// Types that implement this trait can define how their children
/// should be arranged within the available space.
pub trait Layout: core::fmt::Debug {
    /// Performs the layout calculation.
    ///
    /// Given the constraints and measured children, this method should
    /// return the final container size and child positions.
    fn layout(self, constraint: Constraint, measured_children: &[MeasuredChild]) -> LayoutResult;
}

/// A wrapper that holds a layout implementation.
///
/// This is used to type-erase layout implementations so they
/// can be used uniformly throughout the view system.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Container(Box<dyn Layout>);

impl Container {
    /// Creates a new container with the given layout implementation.
    pub fn new(layout: impl Layout + 'static) -> Self {
        Self(Box::new(layout))
    }
}

raw_view!(Container);
