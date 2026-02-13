use crate::IntoFFI;
use crate::WuiStr;
use crate::reactive::WuiComputed;
use waterui_media::live::{LivePhotoConfig, LivePhotoSource};

// Photo is now a composite View (uses Dynamic + Image internally)
// so it doesn't need FFI bindings - it resolves to GpuSurface which has FFI

// =============================================================================
// LivePhoto
// =============================================================================

into_ffi! { LivePhotoConfig,
    pub struct WuiLivePhoto {
        source: *mut WuiComputed<LivePhotoSource>,
    }
}

into_ffi! {
    LivePhotoSource,
    pub struct WuiLivePhotoSource {
         image: WuiStr,
         video: WuiStr,
    }
}

impl IntoFFI for waterui_media::Url {
    type FFI = WuiStr;
    fn into_ffi(self) -> Self::FFI {
        self.inner().into_ffi()
    }
}

// =============================================================================
// FFI view bindings
// =============================================================================

ffi_view!(LivePhotoConfig, WuiLivePhoto, live_photo);

// MediaPicker is now a pure Rust view backed by waterkit-dialog.
// No media picker manager/service FFI is needed.
