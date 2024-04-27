use crate::{computed::waterui_computed_int, ffi_view, waterui_anyview, IntoFFI};

use waterui::component::progress::{ProgressStyle, RawProgress};

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
pub struct Progress {
    label: *mut waterui_anyview,
    progress: *mut waterui_computed_int,
    style: waterui_style_progress,
}

impl IntoFFI for RawProgress {
    type FFI = Progress;
    fn into_ffi(self) -> Self::FFI {
        Progress {
            label: self._label.into_ffi(),
            progress: self._progress.into_ffi(),
            style: self._style.into_ffi(),
        }
    }
}

ffi_view!(
    RawProgress,
    Progress,
    waterui_view_force_as_progress,
    waterui_view_progress_id
);
