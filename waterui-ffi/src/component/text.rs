use waterui::component::Text;

use crate::{
    computed::{waterui_computed_bool, waterui_computed_str},
    ffi_view, IntoFFI, IntoRust,
};

#[repr(C)]
pub struct waterui_text {
    content: *mut waterui_computed_str,
    selectable: *mut waterui_computed_bool,
}

impl IntoFFI for Text {
    type FFI = waterui_text;
    fn into_ffi(self) -> Self::FFI {
        waterui_text {
            content: self._content.into_ffi(),
            selectable: self._selectable.into_ffi(),
        }
    }
}

impl IntoRust for waterui_text {
    type Rust = Text;
    unsafe fn into_rust(self) -> Self::Rust {
        Text::new(self.content.into_rust()).selectable(self.selectable.into_rust())
    }
}

ffi_view!(
    Text,
    waterui_text,
    waterui_view_force_as_text,
    waterui_view_text_id
);
