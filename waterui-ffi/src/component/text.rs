use waterui::component::Text;

use crate::{
    computed::{waterui_computed_bool, waterui_computed_str},
    ffi_view, IntoFFI,
};

#[repr(C)]
pub struct waterui_text {
    content: *mut waterui_computed_str,
    selection: *mut waterui_computed_bool,
}

impl IntoFFI for Text {
    type FFI = waterui_text;
    fn into_ffi(self) -> Self::FFI {
        waterui_text {
            content: self._content.into_ffi(),
            selection: self._selection.into_ffi(),
        }
    }
}

ffi_view!(
    Text,
    waterui_text,
    waterui_view_force_as_text,
    waterui_view_text_id
);
