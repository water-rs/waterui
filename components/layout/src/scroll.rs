use waterui_core::{AnyView, View, raw_view, view::TupleViews};

use super::stack::vstack;
#[derive(Debug, uniffi::Record)]
#[must_use]
pub struct ScrollView {
    pub content: AnyView,
    pub axis: Axis,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, uniffi::Enum)]
pub enum Axis {
    Horizontal,
    #[default]
    Vertical,
    All,
}

raw_view!(ScrollView);

impl ScrollView {
    pub fn new(content: impl View) -> Self {
        Self {
            content: AnyView::new(content),
            axis: Axis::All,
        }
    }

    pub fn horizontal(content: impl View) -> Self {
        Self {
            content: AnyView::new(content),
            axis: Axis::Horizontal,
        }
    }

    pub fn vertical(content: impl View) -> Self {
        Self {
            content: AnyView::new(content),
            axis: Axis::Vertical,
        }
    }
}

pub fn scroll(content: impl TupleViews) -> ScrollView {
    ScrollView::new(vstack(content))
}
