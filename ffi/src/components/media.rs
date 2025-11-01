use crate::WuiStr;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, WuiAnyView, ffi_view};
use waterui_media::{LivePhoto, Photo, VideoPlayer};
use waterui_media::{
    Video,
    live::{LivePhotoConfig, LivePhotoSource},
    photo::PhotoConfig,
    video::VideoPlayerConfig,
};

// Type alias for URL
type Url = WuiStr;
type Volume = f32;

into_ffi! {PhotoConfig,
    pub struct WuiPhoto {
        source: Url,
        placeholder: *mut WuiAnyView,
    }
}

// It is not a native view, the actual view is VideoPlayer
#[repr(C)]
pub struct WuiVideo {
    url: Url,
}

impl IntoFFI for Video {
    type FFI = WuiVideo;
    fn into_ffi(self) -> Self::FFI {
        WuiVideo {
            url: self.url().inner().into_ffi(),
        }
    }
}

into_ffi! {VideoPlayerConfig,
    pub struct WuiVideoPlayer {
        video: *mut WuiComputed<Video>,
        volume: *mut WuiBinding<Volume>,
    }
}

into_ffi! { LivePhotoConfig,
    pub struct WuiLivePhoto {
        source: *mut WuiComputed<LivePhotoSource>,
    }
}

into_ffi! {
    LivePhotoSource,
    pub struct WuiLivePhotoSource {
         image: Url,
         video: Url,
    }
}

impl IntoFFI for waterui_media::Url {
    type FFI = WuiStr;
    fn into_ffi(self) -> Self::FFI {
        self.inner().into_ffi()
    }
}

// FFI view bindings for media components
native_view!(Photo, WuiPhoto);

native_view!(VideoPlayer, WuiVideoPlayer);

native_view!(LivePhoto, WuiLivePhoto);

ffi_view!(LivePhotoSource, WuiLivePhotoSource);

// Note: Media enum has complex tuple variants that need special FFI handling
// - leaving for future implementation with manual IntoFFI implementation
