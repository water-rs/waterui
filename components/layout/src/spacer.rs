//! Flexible layout gaps used by stacks and other containers.

use alloc::vec::Vec;
use waterui_core::raw_view;

use crate::{Layout, ProposalSize, Rect, Size, SubView};

/// A flexible space component that expands to fill available space.
///
/// Spacers are commonly used in layouts to push other components apart or to
/// create flexible spacing that adapts to the container size.
#[derive(Debug, Clone, PartialEq)]
pub struct Spacer {
    min_length: f32,
}

impl Spacer {
    /// Creates a new spacer with the specified minimum length.
    #[must_use]
    pub const fn new(min_length: f32) -> Self {
        Self { min_length }
    }

    /// Creates a spacer with zero minimum length.
    #[must_use]
    pub const fn flexible() -> Self {
        Self { min_length: 0.0 }
    }
}

/// Layout implementation for a single spacer.
///
/// Spacers are greedy and will expand to fill all available space
/// in the direction they are placed, respecting their minimum length.
#[derive(Debug, Clone)]
pub struct SpacerLayout {
    min_length: f32,
}

impl Layout for SpacerLayout {
    fn size_that_fits(
        &self,
        _proposal: ProposalSize,
        _children: &mut [&mut dyn SubView],
    ) -> Size {
        // Spacer reports its minimum length as intrinsic size (like SwiftUI)
        // The parent stack will expand it to fill remaining space during place()
        Size::new(self.min_length, self.min_length)
    }

    fn place(
        &self,
        _bounds: Rect,
        _children: &mut [&mut dyn SubView],
    ) -> Vec<Rect> {
        // Spacer has no children to place
        Vec::new()
    }
}

impl From<Spacer> for SpacerLayout {
    fn from(spacer: Spacer) -> Self {
        Self {
            min_length: spacer.min_length,
        }
    }
}

raw_view!(Spacer); // Spacer has a special behavior in layouting

/// Creates a flexible spacer with zero minimum length.
///
/// This spacer will expand to fill all available space in layouts.
#[must_use]
pub const fn spacer() -> Spacer {
    Spacer::flexible()
}

/// Creates a spacer with a specific minimum length.
///
/// This spacer will expand to fill available space but never shrink below the minimum.
#[must_use]
pub const fn spacer_min(min_length: f32) -> Spacer {
    Spacer::new(min_length)
}
