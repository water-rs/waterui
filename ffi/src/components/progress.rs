use nami::Computed;
use waterui::component::{
    Native,
    progress::{ProgressConfig, ProgressStyle},
};

use crate::{IntoFFI, WuiAnyView};

#[repr(C)]
pub struct WuiProgress {
    pub label: *mut WuiAnyView,
    pub value_label: *mut WuiAnyView,
    pub value: *mut Computed<f64>,
    pub style: WuiProgressStyle,
}

#[repr(C)]
pub enum WuiProgressStyle {
    Linear,
    Circular,
}

impl IntoFFI for ProgressStyle {
    type FFI = WuiProgressStyle;
    fn into_ffi(self) -> WuiProgressStyle {
        match self {
            ProgressStyle::Linear => WuiProgressStyle::Linear,
            _ => WuiProgressStyle::Circular,
        }
    }
}

ffi_struct!(
    ProgressConfig,
    WuiProgress,
    label,
    value_label,
    value,
    style
);

ffi_view!(
    Native<ProgressConfig>,
    WuiProgress,
    waterui_progress_id,
    waterui_force_as_progress
);
