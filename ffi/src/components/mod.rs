use crate::{IntoFFI, WuiStr, WuiTypeId};

pub mod layout;

impl<T: IntoFFI + waterui_core::NativeView> IntoFFI for waterui_core::Native<T> {
    type FFI = T::FFI;
    fn into_ffi(self) -> Self::FFI {
        IntoFFI::into_ffi(self.into_inner())
    }
}

pub mod button;

ffi_view!(waterui::Str, WuiStr, plain);
pub mod lazy;

pub mod text;

/// Form component FFI bindings
pub mod form;

/// Navigation component FFI bindings
pub mod navigation;

/// Video component FFI bindings
pub mod video;

pub mod dynamic;

pub mod list;

pub mod table;

/// Returns the type ID for empty views as a 128-bit value.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_empty_id() -> WuiTypeId {
    WuiTypeId::of::<()>()
}

pub mod progress;

/// GPU surface FFI bindings for high-performance wgpu rendering
pub mod gpu_surface;

/// SystemIcon component FFI bindings for platform-native icons
pub mod icon;

/// WebView component FFI bindings
pub mod webview;

/// Map component FFI bindings
pub mod map;
pub use map::{WuiAnnotation, WuiCoordinate, WuiRegion};

/// ViewEffect component FFI bindings for GPU effect rendering
pub mod view_effect;

/// AppliedFilter metadata FFI bindings for GPU filter rendering
pub mod applied_filter;
/// FilteredView<Blur> hook FFI bindings for compositor-native blur paths
pub mod filtered_blur;
pub(crate) mod pixel_upload;

/// Android-only AHardwareBuffer import helpers (Vulkan)
#[cfg(target_os = "android")]
pub(crate) mod android_ahb;

/// ViewRenderer FFI bindings for capturing views to PNG
pub mod view_renderer;
