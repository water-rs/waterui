use crate::{IntoFFI, WuiStr, WuiTypeId};

impl<T: IntoFFI + waterui_core::NativeView> IntoFFI for waterui_core::Native<T> {
    type FFI = T::FFI;
    fn into_ffi(self) -> Self::FFI {
        IntoFFI::into_ffi(self.into_inner())
    }
}

ffi_view!(waterui::Str, WuiStr, plain);

pub mod controls;
pub mod data;
mod layouting;
pub mod media;
mod nav;
pub mod platform;
pub mod typography;
mod visual;

pub use controls::{button, form, progress};
pub use data::map;
pub use data::map::{WuiAnnotation, WuiCoordinate, WuiRegion};
pub use layouting::{layout, lazy, list, table};
pub use media::video;
pub use nav::navigation;
pub use platform::{dynamic, icon, webview};
pub use typography::text;
#[cfg(target_os = "android")]
pub(crate) use visual::android_ahb;
pub(crate) use visual::pixel_upload;
pub use visual::{applied_filter, gpu_surface, view_effect, view_renderer};

/// Returns the type ID for empty views as a 128-bit value.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_empty_id() -> WuiTypeId {
    WuiTypeId::of::<()>()
}
