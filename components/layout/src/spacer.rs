//! Flexible layout gaps used by stacks and other containers.

use alloc::vec::Vec;
use waterui_core::raw_view;

use crate::{ChildMetadata, Layout, ProposalSize, Rect, Size};

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
    fn propose(&mut self, _parent: ProposalSize, _children: &[ChildMetadata]) -> Vec<ProposalSize> {
        // Spacer has no children
        Vec::new()
    }

    fn size(&mut self, parent: ProposalSize, _children: &[ChildMetadata]) -> Size {
        // Spacer takes all available space, but respects minimum length
        let width = parent.width.unwrap_or(self.min_length).max(self.min_length);
        let height = parent
            .height
            .unwrap_or(self.min_length)
            .max(self.min_length);

        Size::new(width, height)
    }

    fn place(
        &mut self,
        _bound: Rect,
        _proposal: ProposalSize,
        _children: &[ChildMetadata],
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
