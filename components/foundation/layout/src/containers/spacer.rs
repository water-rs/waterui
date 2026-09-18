//! Flexible layout gaps used by stacks and other containers.

use waterui_core::raw_view;

use crate::StretchAxis;

/// A flexible space that expands to push views apart.
///
/// Spacer adapts to its parent container: in `HStack` it expands horizontally,
/// in `VStack` it expands vertically. Use it to push views to opposite edges
/// or distribute space evenly.
///
/// # Layout Behavior
///
/// - **In `HStack`:** Expands horizontally only
/// - **In `VStack`:** Expands vertically only
/// - **In `ZStack`:** Claims nothing
///
/// # Examples
///
/// ```rust
/// use waterui::layout::spacer::spacer_min;
/// use waterui::prelude::*;
///
/// // Push button to trailing edge
/// hstack((
///     text("Title"),
///     spacer(),
///     button("Done").action(|| {}),
/// ));
///
/// // Center content with equal spacing
/// hstack((spacer(), text("Centered"), spacer()));
///
/// // Spacer with minimum length (never shrinks below 20pt)
/// spacer_min(20.0);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Spacer {
    min_length: f32,
}

impl Spacer {
    /// The default layout priority of a flexible gap, below ordinary content.
    pub const DEFAULT_LAYOUT_PRIORITY: i32 = i32::MIN;

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

raw_view!(Spacer, StretchAxis::MainAxis);

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
