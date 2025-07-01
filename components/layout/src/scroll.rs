use waterui_core::{AnyView, View, raw_view, view::TupleViews};

use super::stack::vstack;
#[derive(Debug)]
#[must_use]
/// A view that allows scrolling through its content.
pub struct ScrollView {
    /// The content to be scrolled.
    pub content: AnyView,
    /// The axis along which the content can be scrolled.
    pub axis: Axis,
}

/// Represents the axis along which the content of a [`ScrollView`] can be scrolled.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Axis {
    /// Scrolls horizontally.
    Horizontal,
    /// Scrolls vertically.
    #[default]
    Vertical,
    /// Scrolls in both horizontal and vertical directions.
    All,
}

raw_view!(ScrollView);

impl ScrollView {
    /// Creates a new [`ScrollView`] scrolling in both horizontal and vertical directions.
    pub fn new(content: impl View) -> Self {
        Self {
            content: AnyView::new(content),
            axis: Axis::All,
        }
    }
    /// Creates a new [`ScrollView`] scrolling horizontally.
    pub fn horizontal(content: impl View) -> Self {
        Self {
            content: AnyView::new(content),
            axis: Axis::Horizontal,
        }
    }
    /// Creates a new [`ScrollView`] scrolling vertically.
    pub fn vertical(content: impl View) -> Self {
        Self {
            content: AnyView::new(content),
            axis: Axis::Vertical,
        }
    }
}

/// Creates a new [`ScrollView`] with the given content arranged vertically.
///
/// Equal to calling `ScrollView::new(vstack(content))`.
pub fn scroll(content: impl TupleViews) -> ScrollView {
    ScrollView::new(vstack(content))
}
