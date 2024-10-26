use waterui::component::layout::scroll::{Axis, ScrollView};

use crate::{waterui_anyview, IntoFFI};

#[repr(C)]
pub enum waterui_axis {
    HORIZONTAL,
    VERTICAL,
    ALL,
}

impl IntoFFI for Axis {
    type FFI = waterui_axis;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Axis::Horizontal => waterui_axis::HORIZONTAL,
            Axis::Vertical => waterui_axis::VERTICAL,
            Axis::All => waterui_axis::ALL,
        }
    }
}

#[repr(C)]
pub struct waterui_scroll {
    content: *mut waterui_anyview,
    axis: waterui_axis,
}

into_ffi!(ScrollView, waterui_scroll, content, axis);

ffi_view!(
    ScrollView,
    waterui_scroll,
    waterui_view_force_as_scroll,
    waterui_view_scroll_id
);
