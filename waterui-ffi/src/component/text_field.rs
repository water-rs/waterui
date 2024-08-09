use waterui::{component::text_field::TextFieldConfig, view::ConfigurableView};

use crate::{binding::waterui_binding_str, waterui_anyview, IntoFFI};

use super::text::waterui_text;

#[repr(C)]
pub struct waterui_text_field {
    label: *mut waterui_anyview,
    value: *mut waterui_binding_str,
    prompt: waterui_text,
}

impl IntoFFI for TextFieldConfig {
    type FFI = waterui_text_field;
    fn into_ffi(self) -> Self::FFI {
        Self::FFI {
            label: self.label.into_ffi(),
            value: self.value.into_ffi(),
            prompt: self.prompt.config().into_ffi(),
        }
    }
}

native_view!(
    TextFieldConfig,
    waterui_text_field,
    waterui_view_force_as_text_field,
    waterui_view_text_field_id
);
