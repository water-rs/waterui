use waterui_core::{AnyView, View, raw_view, view::TupleViews};

use super::stack::vstack;
#[derive(Debug)]
#[must_use]
pub struct ScrollView {
    pub content: AnyView,
    pub axis: Axis,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

pub(crate) mod ffi {
    use waterui_core::{AnyView, ffi_view};
    use waterui_ffi::{ffi_enum, ffi_struct};

    use super::{Axis, ScrollView};

    #[repr(C)]
    pub struct WuiScrollView {
        pub content: *mut AnyView,
        pub axis: WuiAxis,
    }

    ffi_enum!(Axis, WuiAxis, Horizontal, Vertical, All);

    ffi_struct!(ScrollView, WuiScrollView, content, axis);
    ffi_view!(
        ScrollView,
        WuiScrollView,
        waterui_scroll_id,
        waterui_force_as_scroll
    );
}
