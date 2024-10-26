use waterui::color::{BackgroundColor, Color, ForegroundColor};

pub type waterui_color = Color;

impl_binding!(
    waterui_binding_color,
    Color,
    waterui_color,
    waterui_read_binding_color,
    waterui_set_binding_color,
    waterui_watch_binding_color,
    waterui_drop_binding_color
);

impl_computed!(
    waterui_computed_color,
    Color,
    waterui_color,
    waterui_read_computed_color,
    waterui_watch_computed_color,
    waterui_drop_computed_color
);

ffi_safe!(waterui_color);

#[repr(C)]
pub struct waterui_background_color {
    color: *mut waterui_computed_color,
}

into_ffi!(BackgroundColor, waterui_background_color, color);

ffi_metadata!(
    BackgroundColor,
    waterui_background_color,
    waterui_metadata_force_as_background_color,
    waterui_metadata_background_color_id
);

#[repr(C)]
pub struct waterui_foreground_color {
    color: *mut waterui_computed_color,
}

into_ffi!(ForegroundColor, waterui_foreground_color, color);

ffi_metadata!(
    ForegroundColor,
    waterui_foreground_color,
    waterui_metadata_force_as_foreground_color,
    waterui_metadata_foreground_color_id
);
