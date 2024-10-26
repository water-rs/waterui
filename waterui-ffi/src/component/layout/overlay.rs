use waterui::component::layout::overlay::Overlay;

use crate::waterui_anyview;

#[repr(C)]
pub struct waterui_overlay {
    content: *mut waterui_anyview,
}

into_ffi!(Overlay, waterui_overlay, content);
