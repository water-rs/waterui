use waterui::component::picker::color::ColorPickerConfig;

use crate::{component::color::waterui_binding_color, waterui_anyview};

#[repr(C)]
pub struct waterui_color_picker {
    label: *mut waterui_anyview,
    value: *mut waterui_binding_color,
}

into_ffi!(ColorPickerConfig, waterui_color_picker, label, value);

native_view!(
    ColorPickerConfig,
    waterui_color_picker,
    waterui_view_force_as_color_picker,
    waterui_view_color_picker_id
);
