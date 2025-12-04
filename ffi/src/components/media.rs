use alloc::string::String;
use crate::WuiStr;
use crate::closure::WuiFn;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, IntoRust, WuiAnyView};
use waterui_media::{
    AspectRatio, Video,
    live::{LivePhotoConfig, LivePhotoSource},
    photo::{Event as PhotoEvent, PhotoConfig},
    video::{Event as VideoEvent, VideoPlayerConfig},
};

// Type alias for URL
type Url = WuiStr;
type Volume = f32;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum WuiAspectRatio {
    Fit = 0,
    Fill = 1,
    Stretch = 2,
}

impl IntoFFI for AspectRatio {
    type FFI = WuiAspectRatio;
    fn into_ffi(self) -> Self::FFI {
        match self {
            AspectRatio::Fit => WuiAspectRatio::Fit,
            AspectRatio::Fill => WuiAspectRatio::Fill,
            AspectRatio::Stretch => WuiAspectRatio::Stretch,
        }
    }
}

/// FFI representation of photo events.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum WuiPhotoEventType {
    Loaded = 0,
    Error = 1,
}

/// FFI representation of a photo event.
#[repr(C)]
pub struct WuiPhotoEvent {
    pub event_type: WuiPhotoEventType,
    pub error_message: WuiStr,
}

impl IntoFFI for PhotoEvent {
    type FFI = WuiPhotoEvent;
    fn into_ffi(self) -> Self::FFI {
        match self {
            PhotoEvent::Loaded => WuiPhotoEvent {
                event_type: WuiPhotoEventType::Loaded,
                error_message: "".into_ffi(),
            },
            PhotoEvent::Error(message) => WuiPhotoEvent {
                event_type: WuiPhotoEventType::Error,
                error_message: waterui::Str::from(message).into_ffi(),
            },
        }
    }
}

#[repr(C)]
pub struct WuiPhoto {
    pub source: Url,
    pub placeholder: *mut WuiAnyView,
    pub on_event: WuiFn<WuiPhotoEvent>,
}

impl IntoFFI for PhotoConfig {
    type FFI = WuiPhoto;
    fn into_ffi(self) -> Self::FFI {
        // Convert the Rust closure to a WuiFn
        // Swift will call this with WuiPhotoEvent, we convert to Rust Event and call the closure
        let on_event_fn = WuiFn::from(move |ffi_event: WuiPhotoEvent| {
            // Convert FFI event to Rust event
            let rust_event = match ffi_event.event_type {
                WuiPhotoEventType::Loaded => PhotoEvent::Loaded,
                WuiPhotoEventType::Error => {
                    let message_str = unsafe { ffi_event.error_message.into_rust() };
                    PhotoEvent::Error(String::from(message_str))
                }
            };

            // Call the user's closure
            (self.on_event)(rust_event);
        });

        WuiPhoto {
            source: self.source.into_ffi(),
            placeholder: self.placeholder.into_ffi(),
            on_event: on_event_fn,
        }
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

/// FFI representation of video player events.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum WuiVideoEventType {
    ReadyToPlay = 0,
    Ended = 1,
    Error = 2,
    Buffering = 3,
    BufferingEnded = 4,
}

/// FFI representation of a video player event.
#[repr(C)]
pub struct WuiVideoEvent {
    pub event_type: WuiVideoEventType,
    pub error_message: WuiStr,
}

impl IntoFFI for VideoEvent {
    type FFI = WuiVideoEvent;
    fn into_ffi(self) -> Self::FFI {
        match self {
            VideoEvent::ReadyToPlay => WuiVideoEvent {
                event_type: WuiVideoEventType::ReadyToPlay,
                error_message: "".into_ffi(),
            },
            VideoEvent::Ended => WuiVideoEvent {
                event_type: WuiVideoEventType::Ended,
                error_message: "".into_ffi(),
            },
            VideoEvent::Buffering => WuiVideoEvent {
                event_type: WuiVideoEventType::Buffering,
                error_message: "".into_ffi(),
            },
            VideoEvent::BufferingEnded => WuiVideoEvent {
                event_type: WuiVideoEventType::BufferingEnded,
                error_message: "".into_ffi(),
            },
            VideoEvent::Error { message } => WuiVideoEvent {
                event_type: WuiVideoEventType::Error,
                error_message: waterui::Str::from(message).into_ffi(),
            },
        }
    }
}

#[repr(C)]
pub struct WuiVideoPlayer {
    pub video: *mut WuiComputed<Video>,
    pub volume: *mut WuiBinding<Volume>,
    pub aspect_ratio: WuiAspectRatio,
    pub show_controls: bool,
    pub on_event: WuiFn<WuiVideoEvent>,
}

impl IntoFFI for VideoPlayerConfig {
    type FFI = WuiVideoPlayer;
    fn into_ffi(self) -> Self::FFI {
        // Convert the Rust closure to a WuiFn
        // Swift will call this with WuiVideoEvent, we convert to Rust Event and call the closure
        let on_event_fn = WuiFn::from(move |ffi_event: WuiVideoEvent| {
            // Convert FFI event to Rust event
            let rust_event = match ffi_event.event_type {
                WuiVideoEventType::ReadyToPlay => VideoEvent::ReadyToPlay,
                WuiVideoEventType::Ended => VideoEvent::Ended,
                WuiVideoEventType::Buffering => VideoEvent::Buffering,
                WuiVideoEventType::BufferingEnded => VideoEvent::BufferingEnded,
                WuiVideoEventType::Error => {
                    let message_str = unsafe { ffi_event.error_message.into_rust() };
                    VideoEvent::Error {
                        message: String::from(message_str),
                    }
                }
            };

            // Call the user's closure
            (self.on_event)(rust_event);
        });

        WuiVideoPlayer {
            video: self.video.into_ffi(),
            volume: self.volume.into_ffi(),
            aspect_ratio: self.aspect_ratio.into_ffi(),
            show_controls: self.show_controls,
            on_event: on_event_fn,
        }
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
ffi_view!(PhotoConfig, WuiPhoto, photo);

ffi_view!(VideoPlayerConfig, WuiVideoPlayer, video_player);

ffi_view!(LivePhotoConfig, WuiLivePhoto, live_photo);

// Note: Media enum has complex tuple variants that need special FFI handling
// - leaving for future implementation with manual IntoFFI implementation
