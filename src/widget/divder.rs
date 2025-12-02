//! Module that provides a simple divider component.
//!
//! This module contains the `Divider` component which is a visual separator
//! that can be used to create a clear distinction between different sections
//! or elements in a user interface.

use core::f32;

use waterui_color::Color;
use waterui_core::View;

use crate::ViewExt;

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
//
// ═══════════════════════════════════════════════════════════════════════════
// INTERNAL: Layout Contract for Backend Implementers
// ═══════════════════════════════════════════════════════════════════════════
//
// Stretch Axis: `Adaptive` - Expands along parent stack's cross axis.
// Thickness: Fixed 1pt on the non-expanding axis
// Main axis: Expands to fill parent's main axis dimension
//
// ═══════════════════════════════════════════════════════════════════════════
//
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct Divider;

impl View for Divider {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Color::srgb_f32(0.8, 0.8, 0.8).height(1.0) // Color is greedy, it will stretch to fill available width.
    }
}
