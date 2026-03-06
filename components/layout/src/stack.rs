//! Stack-based layout primitives.
//!
//! The submodules implement horizontal, vertical, and overlay stacks. These
//! views arrange child content according to alignments and spacing and are the
//! backbone of most declarative layouts in `WaterUI`.
//!
//! ![Stack](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/stack.svg)

mod vstack;
pub use vstack::*;
mod hstack;
pub use hstack::*;
mod zstack;
pub use zstack::*;

pub use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};

/// Defines the axis of a stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Axis {
    /// Horizontal axis is the x-axis (`HStack`)
    Horizontal,
    /// Vertical axis is the y-axis (`VStack`)
    Vertical,
}

impl Axis {
    /// Returns true if this axis is horizontal.
    #[must_use]
    pub const fn is_horizontal(&self) -> bool {
        matches!(self, Self::Horizontal)
    }

    /// Returns true if this axis is vertical.
    #[must_use]
    pub const fn is_vertical(&self) -> bool {
        matches!(self, Self::Vertical)
    }
}
