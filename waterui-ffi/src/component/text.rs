use waterui::{
    component::{text::TextConfig, Text},
    view::ConfigurableView,
};

use crate::{computed::waterui_computed_str, IntoFFI, IntoRust};

#[repr(C)]
pub struct waterui_text {
    content: *mut waterui_computed_str,
}

into_ffi!(TextConfig, waterui_text, content);

impl IntoFFI for Text {
    type FFI = waterui_text;
    fn into_ffi(self) -> Self::FFI {
        self.config().into_ffi()
    }
}

impl IntoRust for waterui_text {
    type Rust = Text;
    unsafe fn into_rust(self) -> Self::Rust {
        Text::new(self.content.into_rust())
    }
}

native_view!(
    TextConfig,
    waterui_text,
    waterui_view_force_as_text,
    waterui_view_text_id
);
