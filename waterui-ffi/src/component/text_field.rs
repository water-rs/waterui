use waterui::component::form::text_field::TextFieldConfig;

use crate::{binding::waterui_binding_str, waterui_anyview};

use super::text::waterui_text;

#[repr(C)]
pub struct waterui_text_field {
    label: *mut waterui_anyview,
    value: *mut waterui_binding_str,
    prompt: waterui_text,
}

into_ffi!(TextFieldConfig, waterui_text_field, label, value, prompt);

native_view!(
    TextFieldConfig,
    waterui_text_field,
    waterui_view_force_as_text_field,
    waterui_view_text_field_id
);
