use crate::WuiStr;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, IntoRust};
use alloc::rc::Rc;
use alloc::string::String;
use nami::SignalExt;
use nami::signal::IntoComputed;
use waterui::{Binding, Computed, Str};
use waterui_video::{
    AspectRatio, MediaItem,
    video::{
        BoundVideoEventHandler, Event as VideoEvent, NativeVideoConfig, NativeVideoPlayerConfig,
        PlaybackPolicy, SubtitleSelection,
    },
};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    PlaybackStateChanged = 10,
}

/// FFI representation of a video event.
#[repr(C)]
pub struct WuiVideoEvent {
    pub event_type: WuiVideoEventType,
    /// Rust-owned error string. Non-null exactly when `event_type` is `Error`.
    pub error_message: *mut WuiStr,
    pub buffered_ms: u32,
    pub av_drift_ms: f32,
    pub dropped_video_frames: u64,
    pub picture_in_picture_active: bool,
    pub playback_active: bool,
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
        WuiVideoEventType::PlaybackStateChanged => VideoEvent::PlaybackStateChanged {
            playing: ffi_event.playback_active,
        },
        WuiVideoEventType::BufferLevel => VideoEvent::BufferLevel {
            buffered_ms: ffi_event.buffered_ms,
        },
        WuiVideoEventType::PlaybackMetrics => VideoEvent::PlaybackMetrics {
            av_drift_ms: ffi_event.av_drift_ms,
            dropped_video_frames: ffi_event.dropped_video_frames,
        },
        WuiVideoEventType::Error => {
            let error_message = unsafe { Box::from_raw(ffi_event.error_message) };
            let error_message: waterui::Str = unsafe { (*error_message).into_rust() };
            VideoEvent::Error {
                message: String::from(error_message),
            }
        }
    }
}

/// Tagged subtitle selection used by the native player.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiSubtitleSelectionType {
    Auto = 0,
    Off = 1,
    Track = 2,
}

/// FFI representation of [`SubtitleSelection`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WuiSubtitleSelection {
    pub selection_type: WuiSubtitleSelectionType,
    /// Track index. Read only when `selection_type` is `Track`.
    pub track_index: usize,
}

impl IntoFFI for SubtitleSelection {
    type FFI = WuiSubtitleSelection;

    fn into_ffi(self) -> Self::FFI {
        match self {
            SubtitleSelection::Auto => WuiSubtitleSelection {
                selection_type: WuiSubtitleSelectionType::Auto,
                track_index: 0,
            },
            SubtitleSelection::Off => WuiSubtitleSelection {
                selection_type: WuiSubtitleSelectionType::Off,
                track_index: 0,
            },
            SubtitleSelection::Track(track_index) => WuiSubtitleSelection {
                selection_type: WuiSubtitleSelectionType::Track,
                track_index,
            },
        }
    }
}

impl IntoRust for WuiSubtitleSelection {
    type Rust = SubtitleSelection;

    unsafe fn into_rust(self) -> Self::Rust {
        match self.selection_type {
            WuiSubtitleSelectionType::Auto => SubtitleSelection::Auto,
            WuiSubtitleSelectionType::Off => SubtitleSelection::Off,
            WuiSubtitleSelectionType::Track => SubtitleSelection::Track(self.track_index),
        }
    }
}

#[cfg(feature = "c-api")]
crate::ffi_binding!(SubtitleSelection, WuiSubtitleSelection, subtitle_selection);
#[cfg(feature = "c-api")]
crate::ffi_watcher!(SubtitleSelection, WuiSubtitleSelection, subtitle_selection);

/// Immutable buffering and realtime strategy for native playback.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WuiVideoPlaybackPolicy {
    pub realtime: bool,
    pub vod_start_buffer_ms: u32,
    pub vod_resume_buffer_ms: u32,
    pub vod_stall_buffer_ms: u32,
    pub live_max_video_late_ms: u32,
}

impl From<PlaybackPolicy> for WuiVideoPlaybackPolicy {
    fn from(policy: PlaybackPolicy) -> Self {
        Self {
            realtime: policy.realtime,
            vod_start_buffer_ms: policy.vod_start_buffer_ms,
            vod_resume_buffer_ms: policy.vod_resume_buffer_ms,
            vod_stall_buffer_ms: policy.vod_stall_buffer_ms,
            live_max_video_late_ms: policy.live_max_video_late_ms,
        }
    }
}

opaque!(
    WuiVideoEventHandler,
    Rc<BoundVideoEventHandler>,
    video_event_handler,
    any()
);

impl IntoFFI for BoundVideoEventHandler {
    type FFI = *mut WuiVideoEventHandler;

    fn into_ffi(self) -> Self::FFI {
        Rc::new(self).into_ffi()
    }
}

impl IntoFFI for Option<BoundVideoEventHandler> {
    type FFI = *mut WuiVideoEventHandler;

    fn into_ffi(self) -> Self::FFI {
        self.map_or(core::ptr::null_mut(), IntoFFI::into_ffi)
    }
}

/// Delivers one native playback event to an installed Rust handler.
///
/// # Safety
///
/// `handler` must be a live pointer from a video playback descriptor. Tagged
/// event fields must satisfy the invariant documented by [`WuiVideoEvent`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_video_event_handler_call(
    handler: *const WuiVideoEventHandler,
    event: WuiVideoEvent,
) {
    let handler = Rc::clone(&unsafe { crate::borrow_ffi(handler) }.0);
    handler.call(into_video_event(event));
}

/// Shared reactive playback state embedded by every native video descriptor.
#[repr(C)]
pub struct WuiVideoPlaybackDescriptor {
    /// The video source URL as a string (reactive).
    pub source: *mut WuiComputed<Str>,
    /// The media title shown in system media controls.
    pub title: *mut WuiComputed<Str>,
    /// The media artist shown in system media controls.
    pub artist: *mut WuiComputed<Str>,
    /// The media album shown in system media controls.
    pub album: *mut WuiComputed<Str>,
    /// Artwork URL shown in system media controls.
    pub artwork_url: *mut WuiComputed<Str>,
    /// Preferred playback duration in seconds, or `-1.0` when unknown.
    pub duration_seconds: *mut WuiComputed<f64>,
    /// Whether the active queue has a next item.
    pub has_next: *mut WuiBinding<bool>,
    /// Whether the active queue has a previous item.
    pub has_previous: *mut WuiBinding<bool>,
    /// Playback volume. A negative value is muted while preserving its absolute level.
    pub volume: *mut WuiBinding<Volume>,
    /// Subtitle selection for the current runtime track list.
    pub subtitle_selection: *mut WuiBinding<SubtitleSelection>,
    /// Playback speed (1.0 = normal speed).
    pub playback_rate: *mut WuiBinding<f32>,
    /// Whether native playback should preserve pitch when rate changes.
    pub preserve_pitch: *mut WuiBinding<bool>,
    /// Optional event handler for native playback events.
    pub on_event: *mut WuiVideoEventHandler,
    /// Buffering and realtime playback strategy.
    pub playback_policy: WuiVideoPlaybackPolicy,
}

struct VideoPlaybackInputs {
    source: Computed<MediaItem>,
    has_next: Binding<bool>,
    has_previous: Binding<bool>,
    volume: Binding<Volume>,
    subtitle_selection: Binding<SubtitleSelection>,
    playback_rate: Binding<f32>,
    preserve_pitch: Binding<bool>,
    on_event: Option<BoundVideoEventHandler>,
    playback_policy: PlaybackPolicy,
}

impl VideoPlaybackInputs {
    fn into_descriptor(self) -> WuiVideoPlaybackDescriptor {
        let Self {
            source,
            has_next,
            has_previous,
            volume,
            subtitle_selection,
            playback_rate,
            preserve_pitch,
            on_event,
            playback_policy,
        } = self;

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

        WuiVideoPlaybackDescriptor {
            source: source_str.into_ffi(),
            title: title.into_ffi(),
            artist: artist.into_ffi(),
            album: album.into_ffi(),
            artwork_url: artwork_url.into_ffi(),
            duration_seconds: duration_seconds.into_ffi(),
            has_next: has_next.into_ffi(),
            has_previous: has_previous.into_ffi(),
            volume: volume.into_ffi(),
            subtitle_selection: subtitle_selection.into_ffi(),
            playback_rate: playback_rate.into_ffi(),
            preserve_pitch: preserve_pitch.into_ffi(),
            on_event: on_event.into_ffi(),
            playback_policy: playback_policy.into(),
        }
    }
}

/// FFI representation of the raw Video component (no native controls).
#[repr(C)]
pub struct WuiVideo {
    /// Shared reactive playback state.
    pub playback: WuiVideoPlaybackDescriptor,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: WuiAspectRatio,
    /// Whether the video should loop when it ends.
    pub loops: bool,
}

impl IntoFFI for NativeVideoConfig {
    type FFI = WuiVideo;

    fn into_ffi(self) -> Self::FFI {
        let aspect_ratio = self.aspect_ratio.into_ffi();
        let loops = self.loops;
        let playback = VideoPlaybackInputs {
            source: self.source,
            has_next: self.has_next,
            has_previous: self.has_previous,
            volume: self.volume,
            subtitle_selection: self.subtitle_selection,
            playback_rate: self.playback_rate,
            preserve_pitch: self.preserve_pitch,
            on_event: self.on_event,
            playback_policy: self.playback_policy,
        }
        .into_descriptor();

        WuiVideo {
            playback,
            aspect_ratio,
            loops,
        }
    }
}

/// FFI representation of the VideoPlayer component (with native controls).
#[repr(C)]
pub struct WuiVideoPlayer {
    /// Shared reactive playback state.
    pub playback: WuiVideoPlaybackDescriptor,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: WuiAspectRatio,
    /// Whether to show native playback controls.
    pub show_controls: bool,
}

impl IntoFFI for NativeVideoPlayerConfig {
    type FFI = WuiVideoPlayer;

    fn into_ffi(self) -> Self::FFI {
        let aspect_ratio = self.aspect_ratio.into_ffi();
        let show_controls = self.show_controls;
        let playback = VideoPlaybackInputs {
            source: self.source,
            has_next: self.has_next,
            has_previous: self.has_previous,
            volume: self.volume,
            subtitle_selection: self.subtitle_selection,
            playback_rate: self.playback_rate,
            preserve_pitch: self.preserve_pitch,
            on_event: self.on_event,
            playback_policy: self.playback_policy,
        }
        .into_descriptor();

        WuiVideoPlayer {
            playback,
            aspect_ratio,
            show_controls,
        }
    }
}

#[cfg(feature = "c-api")]
ffi_view!(NativeVideoConfig, WuiVideo, video);

#[cfg(feature = "c-api")]
ffi_view!(NativeVideoPlayerConfig, WuiVideoPlayer, video_player);
