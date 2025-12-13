use crate::WuiStr;
use crate::closure::WuiFn;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, IntoRust};
use alloc::string::String;
use nami::SignalExt;
use nami::signal::IntoComputed;
use waterui_media::{
    AspectRatio, Url,
    live::{LivePhotoConfig, LivePhotoSource},
    photo::{Event as PhotoEvent, PhotoConfig},
    video::{Event as VideoEvent, VideoConfig, VideoPlayerConfig},
};

// Type alias for URL
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
    pub source: WuiStr,
    pub on_event: WuiFn<WuiPhotoEvent>,
}

impl IntoFFI for PhotoConfig {
    type FFI = WuiPhoto;
    fn into_ffi(self) -> Self::FFI {
        // Convert the Rust closure to a WuiFn
        // Native code will call this with WuiPhotoEvent, we convert to Rust Event and call the closure
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
            on_event: on_event_fn,
        }
    }
}

/// FFI representation of video events.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum WuiVideoEventType {
    ReadyToPlay = 0,
    Ended = 1,
    Error = 2,
    Buffering = 3,
    BufferingEnded = 4,
}

/// FFI representation of a video event.
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

// =============================================================================
// Video - Raw video view without controls
// =============================================================================

/// FFI representation of the raw Video component (no native controls).
#[repr(C)]
pub struct WuiVideo {
    /// The video source URL as a string (reactive).
    /// Swift expects WuiStr, so we convert Url -> Str.
    pub source: *mut WuiComputed<waterui::Str>,
    /// The volume of the video.
    pub volume: *mut WuiBinding<Volume>,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: WuiAspectRatio,
    /// Whether the video should loop when it ends.
    pub loops: bool,
    /// The event handler for video events.
    pub on_event: WuiFn<WuiVideoEvent>,
}

impl IntoFFI for VideoConfig {
    type FFI = WuiVideo;
    fn into_ffi(self) -> Self::FFI {
        // Convert the Rust closure to a WuiFn
        let on_event_fn = WuiFn::from(move |ffi_event: WuiVideoEvent| {
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

            (self.on_event)(rust_event);
        });

        // Convert Computed<Url> to Computed<Str> for FFI boundary
        let source_str = self.source.map(|url: Url| url.inner()).into_computed();

        WuiVideo {
            source: source_str.into_ffi(),
            volume: self.volume.into_ffi(),
            aspect_ratio: self.aspect_ratio.into_ffi(),
            loops: self.loops,
            on_event: on_event_fn,
        }
    }
}

// =============================================================================
// VideoPlayer - Full-featured player with native controls
// =============================================================================

/// FFI representation of the VideoPlayer component (with native controls).
#[repr(C)]
pub struct WuiVideoPlayer {
    /// The video source URL as a string (reactive).
    /// Swift expects WuiStr, so we convert Url -> Str.
    pub source: *mut WuiComputed<waterui::Str>,
    /// The volume of the video player.
    pub volume: *mut WuiBinding<Volume>,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: WuiAspectRatio,
    /// Whether to show native playback controls.
    pub show_controls: bool,
    /// The event handler for the video player.
    pub on_event: WuiFn<WuiVideoEvent>,
}

impl IntoFFI for VideoPlayerConfig {
    type FFI = WuiVideoPlayer;
    fn into_ffi(self) -> Self::FFI {
        // Convert the Rust closure to a WuiFn
        let on_event_fn = WuiFn::from(move |ffi_event: WuiVideoEvent| {
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

            (self.on_event)(rust_event);
        });

        // Convert Computed<Url> to Computed<Str> for FFI boundary
        let source_str = self.source.map(|url: Url| url.inner()).into_computed();

        WuiVideoPlayer {
            source: source_str.into_ffi(),
            volume: self.volume.into_ffi(),
            aspect_ratio: self.aspect_ratio.into_ffi(),
            show_controls: self.show_controls,
            on_event: on_event_fn,
        }
    }
}

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

ffi_view!(PhotoConfig, WuiPhoto, photo);

// Video - raw video view without controls
ffi_view!(VideoConfig, WuiVideo, video);

// VideoPlayer - full-featured player with native controls
ffi_view!(VideoPlayerConfig, WuiVideoPlayer, video_player);

ffi_view!(LivePhotoConfig, WuiLivePhoto, live_photo);

// Note: Media enum has complex tuple variants that need special FFI handling
// - leaving for future implementation with manual IntoFFI implementation

// =============================================================================
// Video - Computed signal wrapper type for reactive video sources
// =============================================================================

/// A wrapper type representing a video source for reactive Computed signals.
/// This is a newtype wrapper around `Url` that allows separate FFI handling
/// for video sources in computed signals (used by Android's reactive video player).
#[derive(Debug, Clone)]
pub struct Video(pub Url);

impl Video {
    /// Creates a new Video from a URL.
    pub fn new(url: Url) -> Self {
        Self(url)
    }

    /// Returns the inner URL.
    pub fn url(&self) -> &Url {
        &self.0
    }

    /// Consumes self and returns the inner URL.
    pub fn into_url(self) -> Url {
        self.0
    }
}

impl From<Url> for Video {
    fn from(url: Url) -> Self {
        Self(url)
    }
}

/// FFI representation of a Video source for Computed signals.
/// This is used by Android to observe video source changes reactively.
#[repr(C)]
pub struct WuiComputedVideo {
    /// The URL of the video source.
    pub url: WuiStr,
}

impl IntoFFI for Video {
    type FFI = WuiComputedVideo;
    fn into_ffi(self) -> Self::FFI {
        WuiComputedVideo {
            url: self.0.inner().into_ffi(),
        }
    }
}

impl IntoRust for WuiComputedVideo {
    type Rust = Video;
    unsafe fn into_rust(self) -> Self::Rust {
        unsafe {
            let url_str: waterui::Str = self.url.into_rust();
            Video::new(url_str.parse().unwrap())
        }
    }
}

// Generate computed FFI functions for Video type
crate::ffi_computed!(Video, WuiComputedVideo, video);

// =============================================================================
// MediaPickerManager - Unified Manager for Present and Load
// =============================================================================

use waterui_media::media_picker::{
    CustomMediaPickerManager, MediaFilter, MediaPickerManager, SelectedId,
};

/// FFI representation of a simple media filter type.
/// Complex nested filters (All, Not, Any) are not supported via FFI.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiMediaFilterType {
    /// Filter for live photos only.
    LivePhoto = 0,
    /// Filter for videos only.
    Video = 1,
    /// Filter for images only.
    Image = 2,
    /// Filter for all media types.
    All = 3,
}

impl IntoFFI for MediaFilter {
    type FFI = WuiMediaFilterType;
    fn into_ffi(self) -> Self::FFI {
        match self {
            MediaFilter::LivePhoto => WuiMediaFilterType::LivePhoto,
            MediaFilter::Video => WuiMediaFilterType::Video,
            MediaFilter::Image => WuiMediaFilterType::Image,
            // Complex filters default to All for simplicity
            MediaFilter::All(_) | MediaFilter::Not(_) | MediaFilter::Any(_) => {
                WuiMediaFilterType::All
            }
        }
    }
}

// MediaPicker is now a Button wrapper in Rust, no longer a native view
// (No FFI struct or ffi_view! registration needed)

// =============================================================================
// MediaPickerManager FFI - Environment Service Installation
// =============================================================================

use crate::WuiEnv;
use alloc::boxed::Box;
use waterui_media::Media;

/// A callback for receiving selected media ID when user picks media.
///
/// This is a C-compatible closure that native code calls when picker completes.
#[repr(C)]
pub struct MediaPickerPresentCallback {
    /// Opaque pointer to the callback data.
    pub data: *mut (),
    /// Function to call with the selected media. This consumes the callback.
    pub call: unsafe extern "C" fn(*mut (), SelectedId),
}

/// Type alias for the native media picker present function.
pub type MediaPickerPresentFn =
    unsafe extern "C" fn(WuiMediaFilterType, MediaPickerPresentCallback);

/// FFI representation of the result from loading media.
///
/// For Live Photos / Motion Photos, both `url_ptr` (image) and `video_url_ptr` (video)
/// are populated. For regular images/videos, only `url_ptr` is used.
#[repr(C)]
pub struct MediaLoadResult {
    /// Pointer to UTF-8 encoded URL string (image URL for Live Photos).
    pub url_ptr: *const u8,
    /// Length of the URL string in bytes.
    pub url_len: usize,
    /// Pointer to UTF-8 encoded video URL (only for Live Photos).
    pub video_url_ptr: *const u8,
    /// Length of the video URL string in bytes.
    pub video_url_len: usize,
    /// Media type: 0 = Image, 1 = Video, 2 = LivePhoto.
    pub media_type: u8,
}

/// Media type constants matching native implementations.
pub mod media_type {
    /// Image media type.
    pub const IMAGE: u8 = 0;
    /// Video media type.
    pub const VIDEO: u8 = 1;
    /// Live Photo / Motion Photo media type.
    pub const LIVE_PHOTO: u8 = 2;
}

/// A callback for receiving loaded media from native code.
///
/// This is a C-compatible closure that native code calls with the result.
#[repr(C)]
pub struct MediaLoadCallback {
    /// Opaque pointer to the callback data.
    pub data: *mut (),
    /// Function to call with the result. This consumes the callback.
    pub call: unsafe extern "C" fn(*mut (), MediaLoadResult),
}

/// Type alias for the native media load function.
pub type MediaLoadFn = unsafe extern "C" fn(u32, MediaLoadCallback);

/// FFI-compatible MediaPickerManager implementation.
struct FFIMediaPickerManager {
    present_fn: MediaPickerPresentFn,
    load_fn: MediaLoadFn,
}

impl CustomMediaPickerManager for FFIMediaPickerManager {
    fn present(&self, filter: MediaFilter, callback: impl FnOnce(SelectedId) + 'static) {
        let callback_box: Box<Box<dyn FnOnce(SelectedId)>> = Box::new(Box::new(callback));
        let callback_data = Box::into_raw(callback_box).cast::<()>();

        unsafe extern "C" fn present_trampoline(data: *mut (), selected: SelectedId) {
            let callback = unsafe { Box::from_raw(data.cast::<Box<dyn FnOnce(SelectedId)>>()) };
            let rust_selected = unsafe { selected.into_rust() };
            callback(rust_selected);
        }

        let ffi_callback = MediaPickerPresentCallback {
            data: callback_data,
            call: present_trampoline,
        };

        let filter_type = filter.into_ffi();
        unsafe {
            (self.present_fn)(filter_type, ffi_callback);
        }
    }

    fn load(&self, selected: SelectedId, callback: impl FnOnce(Media) + 'static) {
        let callback_box: Box<Box<dyn FnOnce(Media)>> = Box::new(Box::new(callback));
        let callback_data = Box::into_raw(callback_box).cast::<()>();

        unsafe extern "C" fn load_trampoline(data: *mut (), result: MediaLoadResult) {
            let callback = unsafe { Box::from_raw(data.cast::<Box<dyn FnOnce(Media)>>()) };
            let media = unsafe { media_load_result_to_media(result) };
            callback(media);
        }

        let ffi_callback = MediaLoadCallback {
            data: callback_data,
            call: load_trampoline,
        };

        unsafe {
            (self.load_fn)(selected, ffi_callback);
        }
    }
}

/// Installs a MediaPickerManager into the environment from native function pointers.
///
/// Native backends call this during initialization to register their media picker
/// implementation. This unified manager handles both presenting the picker and loading media.
///
/// # Safety
///
/// The caller must ensure that:
/// - `env` is a valid pointer to a `WuiEnv`
/// - `present_fn` is a valid function pointer to the native media picker presentation
/// - `load_fn` is a valid function pointer to the native media loader implementation
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_media_picker_manager(
    env: *mut WuiEnv,
    present_fn: MediaPickerPresentFn,
    load_fn: MediaLoadFn,
) {
    if env.is_null() {
        return;
    }
    let env = unsafe { &mut *env };

    let manager = MediaPickerManager::new(FFIMediaPickerManager {
        present_fn,
        load_fn,
    });

    env.insert(manager);
}

/// Convert FFI MediaLoadResult to Rust Media enum.
///
/// # Safety
///
/// The caller must ensure the `url_ptr` and `video_url_ptr` (if non-null) point to
/// valid UTF-8 strings of the specified lengths.
unsafe fn media_load_result_to_media(result: MediaLoadResult) -> Media {
    // Parse the main URL
    let url_slice = unsafe { core::slice::from_raw_parts(result.url_ptr, result.url_len) };
    let url_str = unsafe { core::str::from_utf8_unchecked(url_slice) };
    let url: Url = url_str.parse().expect("Invalid URL from native");

    match result.media_type {
        media_type::IMAGE => Media::Image(url),
        media_type::VIDEO => Media::Video(url),
        media_type::LIVE_PHOTO => {
            // Parse the video URL
            let video_slice =
                unsafe { core::slice::from_raw_parts(result.video_url_ptr, result.video_url_len) };
            let video_str = unsafe { core::str::from_utf8_unchecked(video_slice) };
            let video_url: Url = video_str.parse().expect("Invalid video URL from native");

            Media::LivePhoto(LivePhotoSource::new(url, video_url))
        }
        _ => {
            tracing::warn!(
                "Unknown media type {}, treating as image",
                result.media_type
            );
            Media::Image(url)
        }
    }
}
