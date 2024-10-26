use crate::{computed::waterui_computed_double, waterui_anyview, IntoFFI};

use waterui::component::progress::{ProgressConfig, ProgressStyle};

#[repr(C)]
pub enum waterui_style_progress {
    DEFAULT,
    CIRCULAR,
    LINEAR,
}

impl IntoFFI for ProgressStyle {
    type FFI = waterui_style_progress;
    fn into_ffi(self) -> Self::FFI {
        match self {
            ProgressStyle::Circular => waterui_style_progress::CIRCULAR,
            ProgressStyle::Linear => waterui_style_progress::LINEAR,
            _ => waterui_style_progress::DEFAULT,
        }
    }
}

#[repr(C)]
pub struct waterui_progress {
    label: *mut waterui_anyview,
    value: *mut waterui_computed_double,
    style: waterui_style_progress,
}

into_ffi!(ProgressConfig, waterui_progress, label, value, style);

native_view!(
    ProgressConfig,
    waterui_progress,
    waterui_view_force_as_progress,
    waterui_view_progress_id
);
