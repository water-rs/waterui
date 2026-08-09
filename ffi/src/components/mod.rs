use crate::{IntoFFI, WuiStr, WuiTypeId};

impl<T: IntoFFI + waterui_core::NativeView> IntoFFI for waterui_core::Native<T> {
    type FFI = T::FFI;
    fn into_ffi(self) -> Self::FFI {
        IntoFFI::into_ffi(self.into_inner())
    }
}

ffi_view!(waterui::Str, WuiStr, plain);

/// FFI bindings for interactive controls (buttons, forms, progress indicators).
pub mod controls;
/// FFI bindings for data-driven components such as maps.
pub mod data;
mod layouting;
/// FFI bindings for media playback components such as video.
pub mod media;
mod nav;
/// FFI bindings for platform-integration components (icons, web views, dynamic content).
pub mod platform;
/// FFI bindings for text and typography components.
pub mod typography;
mod visual;

pub use controls::{button, form, progress};
pub use data::map;
pub use data::map::{WuiAnnotation, WuiCoordinate, WuiRegion};
#[cfg(feature = "c-api")]
pub use layouting::table;
pub use layouting::{layout, lazy, list};
pub use media::video;
pub use nav::navigation;
pub use platform::{dynamic, icon, webview};
pub use typography::text;
#[cfg(feature = "android-jni")]
pub(crate) use visual::gpu_runtime;
#[cfg(feature = "c-api")]
pub use visual::{applied_filter, view_effect};
pub use visual::{gpu_surface, view_renderer};

/// Returns the type ID for empty views as a 128-bit value.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_empty_id() -> WuiTypeId {
    WuiTypeId::of::<()>()
}
