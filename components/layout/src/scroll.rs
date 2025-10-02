//! Scroll containers that defer behaviour to the active renderer backend.

use waterui_core::{AnyView, View, raw_view};

/// A scrollable container that can display content larger than its bounds.
///
/// `ScrollView` is a special component that requires renderer support for actual scrolling behavior.
/// It cannot be implemented purely through the layout system and must be bridged through FFI
/// to the platform-specific scrolling implementations.
#[derive(Debug)]
pub struct ScrollView {
    axis: Axis,
    content: AnyView,
}

/// Defines the scrolling directions supported by `ScrollView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[non_exhaustive]
pub enum Axis {
    /// Allow horizontal scrolling only.
    Horizontal,
    /// Allow vertical scrolling only (default).
    #[default]
    Vertical,
    /// Allow scrolling in both directions.
    All,
}

impl ScrollView {
    /// Creates a new `ScrollView` with the specified scroll axis and content.
    #[must_use]
    pub const fn new(axis: Axis, content: AnyView) -> Self {
        Self { axis, content }
    }

    /// Decomposes the `ScrollView` into its axis and content.
    pub fn into_inner(self) -> (Axis, AnyView) {
        (self.axis, self.content)
    }

    /// Creates a `ScrollView` with horizontal scrolling.
    pub fn horizontal(content: impl View) -> Self {
        Self::new(Axis::Horizontal, AnyView::new(content))
    }

    /// Creates a `ScrollView` with vertical scrolling.
    pub fn vertical(content: impl View) -> Self {
        Self::new(Axis::Vertical, AnyView::new(content))
    }

    /// Creates a `ScrollView` with scrolling in both directions.
    pub fn both(content: impl View) -> Self {
        Self::new(Axis::All, AnyView::new(content))
    }
}

raw_view!(ScrollView);

/// Creates a vertical `ScrollView` with the given content.
///
/// This is the most common scroll direction for lists and long content.
/// The actual scrolling behavior is implemented by the renderer backend.
pub fn scroll(content: impl View) -> ScrollView {
    ScrollView::vertical(content)
}

/// Creates a horizontal `ScrollView` with the given content.
///
/// Useful for wide content that needs to scroll left-right.
/// The actual scrolling behavior is implemented by the renderer backend.
pub fn scroll_horizontal(content: impl View) -> ScrollView {
    ScrollView::horizontal(content)
}

/// Creates a `ScrollView` that can scroll in both directions.
///
/// Useful for large content like images or tables that may need both horizontal and vertical scrolling.
/// The actual scrolling behavior is implemented by the renderer backend.
pub fn scroll_both(content: impl View) -> ScrollView {
    ScrollView::both(content)
}
