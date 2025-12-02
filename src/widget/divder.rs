//! Module that provides a simple divider component.
//!
//! This module contains the `Divider` component which is a visual separator
//! that can be used to create a clear distinction between different sections
//! or elements in a user interface.

use waterui_color::{Color, Grey};
use waterui_core::{View, layout::StretchAxis, raw_view};

/// A thin line that separates content.
///
/// Divider adapts to its parent container: in VStack it spans horizontally,
/// in HStack it spans vertically.
///
/// # Layout Behavior
///
/// - **In VStack:** Horizontal line spanning full width (1pt height)
/// - **In HStack:** Vertical line spanning full height (1pt width)
///
/// # Examples
///
/// ```ignore
/// // Horizontal divider in a vertical stack
/// vstack((
///     text("Section 1"),
///     Divider,
///     text("Section 2"),
/// ))
///
/// // Vertical divider in a horizontal stack
/// hstack((
///     text("Left"),
///     Divider,
///     text("Right"),
/// ))
/// ```
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct Divider;

impl View for Divider {
    fn body(self, env: &waterui_core::Environment) -> impl View {
        let mut env = env.clone();
        let axis = env.get::<StretchAxis>().unwrap_or_default();
        env.insert(axis);

        Grey
    }
}
