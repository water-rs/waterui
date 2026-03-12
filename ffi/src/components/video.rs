use crate::WuiStr;
use crate::closure::WuiFn;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, IntoRust};
use alloc::string::String;
use nami::SignalExt;
use nami::signal::IntoComputed;
use waterui_video::{
    AspectRatio, Url,
    video::{Event as VideoEvent, VideoConfig, VideoPlayerConfig},
};

// Type alias for URL
pub type Volume = f32;

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

/// FFI representation of video events.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum WuiVideoEventType {
    ReadyToPlay = 0,
    Ended = 1,
    Error = 2,
    Buffering = 3,
    BufferingEnded = 4,
    BufferLevel = 5,
    PlaybackMetrics = 6,
}

/// FFI representation of a video event.
#[repr(C)]
pub struct WuiVideoEvent {
    pub event_type: WuiVideoEventType,
    pub error_message: WuiStr,
    pub buffered_ms: u32,
    pub av_drift_ms: f32,
    pub dropped_video_frames: u64,
}

impl WuiVideoEvent {
    fn empty(event_type: WuiVideoEventType) -> Self {
        Self {
            event_type,
            error_message: "".into_ffi(),
            buffered_ms: 0,
            av_drift_ms: 0.0,
            dropped_video_frames: 0,
        }
    }
}

fn into_video_event(ffi_event: WuiVideoEvent) -> VideoEvent {
    match ffi_event.event_type {
        WuiVideoEventType::ReadyToPlay => VideoEvent::ReadyToPlay,
        WuiVideoEventType::Ended => VideoEvent::Ended,
        WuiVideoEventType::Buffering => VideoEvent::Buffering,
        WuiVideoEventType::BufferingEnded => VideoEvent::BufferingEnded,
        WuiVideoEventType::BufferLevel => VideoEvent::BufferLevel {
            buffered_ms: ffi_event.buffered_ms,
        },
        WuiVideoEventType::PlaybackMetrics => VideoEvent::PlaybackMetrics {
            av_drift_ms: ffi_event.av_drift_ms,
            dropped_video_frames: ffi_event.dropped_video_frames,
        },
        WuiVideoEventType::Error => {
            let message_str = unsafe { ffi_event.error_message.into_rust() };
            VideoEvent::Error {
                message: String::from(message_str),
            }
        }
    }
}

impl IntoFFI for VideoEvent {
    type FFI = WuiVideoEvent;
    fn into_ffi(self) -> Self::FFI {
        match self {
            VideoEvent::ReadyToPlay => WuiVideoEvent::empty(WuiVideoEventType::ReadyToPlay),
            VideoEvent::Ended => WuiVideoEvent::empty(WuiVideoEventType::Ended),
            VideoEvent::Buffering => WuiVideoEvent::empty(WuiVideoEventType::Buffering),
            VideoEvent::BufferingEnded => WuiVideoEvent::empty(WuiVideoEventType::BufferingEnded),
            VideoEvent::BufferLevel { buffered_ms } => WuiVideoEvent {
                event_type: WuiVideoEventType::BufferLevel,
                buffered_ms,
                ..WuiVideoEvent::empty(WuiVideoEventType::BufferLevel)
            },
            VideoEvent::PlaybackMetrics {
                av_drift_ms,
                dropped_video_frames,
            } => WuiVideoEvent {
                event_type: WuiVideoEventType::PlaybackMetrics,
                av_drift_ms,
                dropped_video_frames,
                ..WuiVideoEvent::empty(WuiVideoEventType::PlaybackMetrics)
            },
            VideoEvent::Error { message } => WuiVideoEvent {
                event_type: WuiVideoEventType::Error,
                error_message: waterui::Str::from(message).into_ffi(),
                ..WuiVideoEvent::empty(WuiVideoEventType::Error)
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
            (self.on_event)(into_video_event(ffi_event));
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
            (self.on_event)(into_video_event(ffi_event));
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
// FFI view bindings
// =============================================================================

// Video - raw video view without controls
ffi_view!(VideoConfig, WuiVideo, video);

// VideoPlayer - full-featured player with native controls
ffi_view!(VideoPlayerConfig, WuiVideoPlayer, video_player);

// =============================================================================
// Video - Computed signal wrapper type for reactive video sources
// =============================================================================

/// A wrapper type representing a video source for reactive Computed signals.
/// This is a newtype wrapper around `Url` that allows separate FFI handling
/// for video sources in computed signals.
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

// Generate computed FFI functions for Video type.
crate::ffi_computed!(Video, WuiComputedVideo, video);
