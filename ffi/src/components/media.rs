use crate::WuiStr;
use crate::{IntoFFI, WuiAnyView, ffi_struct, ffi_view};
use waterui::component::Native;
use waterui::{Binding, Computed};
use waterui_media::{
    Video,
    live::{LivePhotoConfig, LivePhotoSource},
    photo::PhotoConfig,
    video::VideoPlayerConfig,
};

// Type alias for URL
type Url = WuiStr;
type Volume = f64;

/// C representation of Photo configuration
#[repr(C)]
pub struct WuiPhoto {
    /// The image source URL
    pub source: Url,
    /// Pointer to the placeholder view
    pub placeholder: *mut WuiAnyView,
}
#[repr(C)]
pub struct WuiVideo {
    pub url: Url,
}

/// C representation of VideoPlayer configuration
#[repr(C)]
pub struct WuiVideoPlayer {
    /// Pointer to the video computed value
    pub video: *mut Computed<Video>,
    /// Pointer to the volume binding
    pub volume: *mut Binding<Volume>,
}

/// C representation of LivePhoto configuration
#[repr(C)]
pub struct WuiLivePhoto {
    /// Pointer to the live photo source computed value
    pub source: *mut Computed<LivePhotoSource>,
}

/// C representation of LivePhotoSource
#[repr(C)]
pub struct WuiLivePhotoSource {
    /// The image URL
    pub image: Url,
    /// The video URL
    pub video: Url,
}

ffi_struct!(LivePhotoSource, WuiLivePhotoSource, image, video);

// Implement struct conversions
ffi_struct!(PhotoConfig, WuiPhoto, source, placeholder);
ffi_struct!(VideoPlayerConfig, WuiVideoPlayer, video, volume);
ffi_struct!(LivePhotoConfig, WuiLivePhoto, source);

impl IntoFFI for Video {
    type FFI = WuiVideo;
    fn into_ffi(self) -> Self::FFI {
        WuiVideo {
            url: self.url().inner().into_ffi(),
        }
    }
}

impl IntoFFI for waterui_media::Url {
    type FFI = WuiStr;
    fn into_ffi(self) -> Self::FFI {
        self.inner().into_ffi()
    }
}

// FFI view bindings for media components
ffi_view!(
    Native<PhotoConfig>,
    WuiPhoto,
    waterui_photo_id,
    waterui_force_as_photo
);

ffi_view!(
    Native<VideoPlayerConfig>,
    WuiVideoPlayer,
    waterui_video_player_id,
    waterui_force_as_video_player
);

ffi_view!(
    Native<LivePhotoConfig>,
    WuiLivePhoto,
    waterui_live_photo_id,
    waterui_force_as_live_photo
);

ffi_view!(LivePhotoSource, waterui_live_photo_source_id);

// Note: Media enum has complex tuple variants that need special FFI handling
// - leaving for future implementation with manual IntoFFI implementation
