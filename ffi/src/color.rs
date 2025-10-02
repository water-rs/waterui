use crate::{IntoFFI, IntoNullableFFI, IntoRust, impl_binding};

use waterui::{Color, core::color::Colorspace};

ffi_type!(WuiColor, Color, waterui_drop_color);

pub enum WuiColorspace {
    Srgb,
    DisplayP3,
}

// colorspace: extended srgb
#[repr(C)]
pub struct WuiResolvedColor {
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
}

impl_binding!(
    Color,
    *mut WuiColor,
    waterui_binding_read_color,
    waterui_binding_set_color,
    waterui_binding_watch_color,
    waterui_drop_color_watcher_guard
);
