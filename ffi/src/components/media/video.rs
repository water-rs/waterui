use crate::WuiStr;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, IntoRust, WuiEnv};
use alloc::string::String;
use nami::SignalExt;
use nami::signal::IntoComputed;
use waterui_core::Binding;
use waterui_video::{
    AspectRatio, PlaybackPolicy, SubtitleSelection, Url,
    video::{Event as VideoEvent, VideoConfig, VideoPlayerConfig},
};

// Type alias for URL
pub type Volume = f32;

opaque!(
    WuiVideoEventAction,
    waterui_core::handler::BoxedEventAction<VideoEvent>,
    video_event_action
);

/// Invokes a video event action with the rendering environment that owns the view.
///
/// # Safety
///
/// `action` and `env` must be valid pointers produced by this library and remain
/// alive for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_video_event_action(
    action: *mut WuiVideoEventAction,
    event: WuiVideoEvent,
    env: *const WuiEnv,
) {
    let action =
        unsafe { crate::expect_non_null_mut(action, "waterui_call_video_event_action", "action") };
    let env = unsafe { crate::expect_non_null(env, "waterui_call_video_event_action", "env") };
    let _ = crate::ffi_boundary("waterui_call_video_event_action", || {
        (action.0)(into_video_event(event), env);
    });
}

const SUBTITLE_SELECTION_AUTO: i32 = -1;
const SUBTITLE_SELECTION_OFF: i32 = -2;

fn subtitle_selection_into_ffi(value: SubtitleSelection) -> i32 {
    match value {
        SubtitleSelection::Auto => SUBTITLE_SELECTION_AUTO,
        SubtitleSelection::Off => SUBTITLE_SELECTION_OFF,
        SubtitleSelection::Track(index) => i32::try_from(index)
            .unwrap_or_else(|_| panic!("subtitle track index must fit into i32")),
    }
}

fn subtitle_selection_into_rust(value: i32) -> SubtitleSelection {
    match value {
        SUBTITLE_SELECTION_AUTO => SubtitleSelection::Auto,
        SUBTITLE_SELECTION_OFF => SubtitleSelection::Off,
        index if index >= 0 => SubtitleSelection::Track(
            usize::try_from(index).expect("non-negative subtitle track index must fit usize"),
        ),
        _ => panic!("unsupported subtitle selection wire value {value}"),
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WuiPlaybackPolicy {
    pub realtime: bool,
    pub vod_start_buffer_ms: u32,
    pub vod_resume_buffer_ms: u32,
    pub vod_stall_buffer_ms: u32,
    pub live_max_video_late_ms: u32,
}

impl From<PlaybackPolicy> for WuiPlaybackPolicy {
    fn from(value: PlaybackPolicy) -> Self {
        Self {
            realtime: value.realtime,
            vod_start_buffer_ms: value.vod_start_buffer_ms,
            vod_resume_buffer_ms: value.vod_resume_buffer_ms,
            vod_stall_buffer_ms: value.vod_stall_buffer_ms,
            live_max_video_late_ms: value.live_max_video_late_ms,
        }
    }
}

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
    PictureInPictureChanged = 7,
    NextRequested = 8,
    PreviousRequested = 9,
}

/// FFI representation of a video event.
#[repr(C)]
pub struct WuiVideoEvent {
    pub event_type: WuiVideoEventType,
    pub error_message: WuiStr,
    pub buffered_ms: u32,
    pub av_drift_ms: f32,
    pub dropped_video_frames: u64,
    pub picture_in_picture_active: bool,
}

impl WuiVideoEvent {
    fn empty(event_type: WuiVideoEventType) -> Self {
        Self {
            event_type,
            error_message: "".into_ffi(),
            buffered_ms: 0,
            av_drift_ms: 0.0,
            dropped_video_frames: 0,
            picture_in_picture_active: false,
        }
    }
}

fn into_video_event(ffi_event: WuiVideoEvent) -> VideoEvent {
    match ffi_event.event_type {
        WuiVideoEventType::ReadyToPlay => VideoEvent::ReadyToPlay,
        WuiVideoEventType::Ended => VideoEvent::Ended,
        WuiVideoEventType::Buffering => VideoEvent::Buffering,
        WuiVideoEventType::BufferingEnded => VideoEvent::BufferingEnded,
        WuiVideoEventType::PictureInPictureChanged => VideoEvent::PictureInPictureChanged {
            active: ffi_event.picture_in_picture_active,
        },
        WuiVideoEventType::NextRequested => VideoEvent::NextRequested,
        WuiVideoEventType::PreviousRequested => VideoEvent::PreviousRequested,
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
            VideoEvent::PictureInPictureChanged { active } => WuiVideoEvent {
                event_type: WuiVideoEventType::PictureInPictureChanged,
                picture_in_picture_active: active,
                ..WuiVideoEvent::empty(WuiVideoEventType::PictureInPictureChanged)
            },
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
            VideoEvent::NextRequested => WuiVideoEvent::empty(WuiVideoEventType::NextRequested),
            VideoEvent::PreviousRequested => {
                WuiVideoEvent::empty(WuiVideoEventType::PreviousRequested)
            }
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
    /// The media title shown in system media controls.
    pub title: *mut WuiComputed<waterui::Str>,
    /// The media artist shown in system media controls.
    pub artist: *mut WuiComputed<waterui::Str>,
    /// The media album shown in system media controls.
    pub album: *mut WuiComputed<waterui::Str>,
    /// Artwork URL shown in system media controls.
    pub artwork_url: *mut WuiComputed<waterui::Str>,
    /// Preferred playback duration in seconds, or `-1.0` when unknown.
    pub duration_seconds: *mut WuiComputed<f64>,
    /// Whether the active queue has a next item.
    pub has_next: *mut WuiBinding<bool>,
    /// Whether the active queue has a previous item.
    pub has_previous: *mut WuiBinding<bool>,
    /// The volume of the video.
    pub volume: *mut WuiBinding<Volume>,
    /// Playback speed (1.0 = normal speed).
    pub playback_rate: *mut WuiBinding<f32>,
    /// Whether native playback should preserve pitch when rate changes.
    pub preserve_pitch: *mut WuiBinding<bool>,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: WuiAspectRatio,
    /// Whether the video should loop when it ends.
    pub loops: bool,
    /// Reactive subtitle selection.
    pub subtitle_selection: *mut WuiBinding<i32>,
    /// Native buffering and realtime playback policy.
    pub playback_policy: WuiPlaybackPolicy,
    /// Environment-aware event handler for video events.
    pub on_event: *mut WuiVideoEventAction,
}

impl IntoFFI for VideoConfig {
    type FFI = WuiVideo;
    fn into_ffi(self) -> Self::FFI {
        let subtitle_selection = Binding::mapping(
            &self.subtitle_selection,
            subtitle_selection_into_ffi,
            |binding, value| binding.set(subtitle_selection_into_rust(value)),
        );

        // Convert Computed<Url> to Computed<Str> for FFI boundary
        let source = self.source;
        let source_str = source
            .clone()
            .map(|item| item.source.inner())
            .into_computed();
        let title = source
            .clone()
            .map(|item| item.metadata.title().unwrap_or_default().to_owned())
            .into_computed();
        let artist = source
            .clone()
            .map(|item| item.metadata.artist().unwrap_or_default().to_owned())
            .into_computed();
        let album = source
            .clone()
            .map(|item| item.metadata.album().unwrap_or_default().to_owned())
            .into_computed();
        let artwork_url = source
            .clone()
            .map(|item| item.metadata.artwork_url().unwrap_or_default().to_owned())
            .into_computed();
        let duration_seconds = source
            .map(|item| {
                item.metadata
                    .duration()
                    .map_or(-1.0, |duration| duration.as_secs_f64())
            })
            .into_computed();

        WuiVideo {
            source: source_str.into_ffi(),
            title: title.into_ffi(),
            artist: artist.into_ffi(),
            album: album.into_ffi(),
            artwork_url: artwork_url.into_ffi(),
            duration_seconds: duration_seconds.into_ffi(),
            has_next: self.has_next.into_ffi(),
            has_previous: self.has_previous.into_ffi(),
            volume: self.volume.into_ffi(),
            playback_rate: self.playback_rate.into_ffi(),
            preserve_pitch: self.preserve_pitch.into_ffi(),
            aspect_ratio: self.aspect_ratio.into_ffi(),
            loops: self.loops,
            subtitle_selection: subtitle_selection.into_ffi(),
            playback_policy: self.playback_policy.into(),
            on_event: self.on_event.into_ffi(),
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
    /// The media title shown in system media controls.
    pub title: *mut WuiComputed<waterui::Str>,
    /// The media artist shown in system media controls.
    pub artist: *mut WuiComputed<waterui::Str>,
    /// The media album shown in system media controls.
    pub album: *mut WuiComputed<waterui::Str>,
    /// Artwork URL shown in system media controls.
    pub artwork_url: *mut WuiComputed<waterui::Str>,
    /// Preferred playback duration in seconds, or `-1.0` when unknown.
    pub duration_seconds: *mut WuiComputed<f64>,
    /// Whether the active queue has a next item.
    pub has_next: *mut WuiBinding<bool>,
    /// Whether the active queue has a previous item.
    pub has_previous: *mut WuiBinding<bool>,
    /// The volume of the video player.
    pub volume: *mut WuiBinding<Volume>,
    /// Playback speed (1.0 = normal speed).
    pub playback_rate: *mut WuiBinding<f32>,
    /// Whether native playback should preserve pitch when rate changes.
    pub preserve_pitch: *mut WuiBinding<bool>,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: WuiAspectRatio,
    /// Whether to show native playback controls.
    pub show_controls: bool,
    /// Reactive subtitle selection.
    pub subtitle_selection: *mut WuiBinding<i32>,
    /// Native buffering and realtime playback policy.
    pub playback_policy: WuiPlaybackPolicy,
    /// Environment-aware event handler for the video player.
    pub on_event: *mut WuiVideoEventAction,
}

impl IntoFFI for VideoPlayerConfig {
    type FFI = WuiVideoPlayer;
    fn into_ffi(self) -> Self::FFI {
        let subtitle_selection = Binding::mapping(
            &self.subtitle_selection,
            subtitle_selection_into_ffi,
            |binding, value| binding.set(subtitle_selection_into_rust(value)),
        );

        // Convert Computed<Url> to Computed<Str> for FFI boundary
        let source = self.source;
        let source_str = source
            .clone()
            .map(|item| item.source.inner())
            .into_computed();
        let title = source
            .clone()
            .map(|item| item.metadata.title().unwrap_or_default().to_owned())
            .into_computed();
        let artist = source
            .clone()
            .map(|item| item.metadata.artist().unwrap_or_default().to_owned())
            .into_computed();
        let album = source
            .clone()
            .map(|item| item.metadata.album().unwrap_or_default().to_owned())
            .into_computed();
        let artwork_url = source
            .clone()
            .map(|item| item.metadata.artwork_url().unwrap_or_default().to_owned())
            .into_computed();
        let duration_seconds = source
            .map(|item| {
                item.metadata
                    .duration()
                    .map_or(-1.0, |duration| duration.as_secs_f64())
            })
            .into_computed();

        WuiVideoPlayer {
            source: source_str.into_ffi(),
            title: title.into_ffi(),
            artist: artist.into_ffi(),
            album: album.into_ffi(),
            artwork_url: artwork_url.into_ffi(),
            duration_seconds: duration_seconds.into_ffi(),
            has_next: self.has_next.into_ffi(),
            has_previous: self.has_previous.into_ffi(),
            volume: self.volume.into_ffi(),
            playback_rate: self.playback_rate.into_ffi(),
            preserve_pitch: self.preserve_pitch.into_ffi(),
            aspect_ratio: self.aspect_ratio.into_ffi(),
            show_controls: self.show_controls,
            subtitle_selection: subtitle_selection.into_ffi(),
            playback_policy: self.playback_policy.into(),
            on_event: self.on_event.into_ffi(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_core::{Binding, Environment, State};

    #[test]
    fn subtitle_selection_wire_values_round_trip() {
        for selection in [
            SubtitleSelection::Auto,
            SubtitleSelection::Off,
            SubtitleSelection::Track(0),
            SubtitleSelection::Track(7),
        ] {
            let encoded = subtitle_selection_into_ffi(selection);
            assert_eq!(subtitle_selection_into_rust(encoded), selection);
        }
    }

    #[test]
    fn native_video_event_action_uses_rendering_environment() {
        let invoked = Binding::bool(false);
        let mut env = Environment::new();
        env.insert(State(invoked.clone()));
        let action = waterui_core::handler::boxed_event_handler(
            |_event: VideoEvent, State(invoked): State<waterui_core::Binding<bool>>| {
                invoked.set(true);
            },
        );
        let action = action.into_ffi();
        let env = env.into_ffi();

        unsafe {
            waterui_call_video_event_action(
                action,
                WuiVideoEvent::empty(WuiVideoEventType::ReadyToPlay),
                env,
            );
            waterui_drop_video_event_action(action);
            crate::waterui_drop_env(env);
        }

        assert!(invoked.get());
    }
}
