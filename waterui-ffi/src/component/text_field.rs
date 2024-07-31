use waterui::component::{text_field::TextFieldStyle, TextField};

use crate::{
    binding::waterui_binding_str, computed::waterui_computed_str, waterui_anyview, IntoFFI,
};

#[repr(C)]
pub struct waterui_text_field {
    label: *mut waterui_anyview,
    value: *mut waterui_binding_str,
    prompt: *mut waterui_computed_str,
    style: waterui_text_field_style,
}

#[repr(C)]
pub enum waterui_text_field_style {
    DEFAULT,
    PLAIN,
    OUTLINED,
    UNDERLINED,
}

impl IntoFFI for TextFieldStyle {
    type FFI = waterui_text_field_style;
    fn into_ffi(self) -> Self::FFI {
        match self {
            TextFieldStyle::Plain => waterui_text_field_style::PLAIN,
            TextFieldStyle::Outlined => waterui_text_field_style::OUTLINED,
            TextFieldStyle::Underlined => waterui_text_field_style::UNDERLINED,
            _ => waterui_text_field_style::DEFAULT,
        }
    }
}

impl IntoFFI for TextField {
    type FFI = waterui_text_field;
    fn into_ffi(self) -> Self::FFI {
        Self::FFI {
            label: self._label.into_ffi(),
            value: self._value.into_ffi(),
            prompt: self._prompt.into_ffi(),
            style: self._style.into_ffi(),
        }
    }
}

ffi_view!(
    TextField,
    waterui_text_field,
    waterui_view_force_as_text_field,
    waterui_view_text_field_id
);
