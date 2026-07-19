use crate::WuiStr;
use crate::reactive::{WuiBinding, WuiComputed};
use crate::{IntoFFI, IntoRust, WuiArray};
#[cfg(target_os = "android")]
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::num::NonZeroU64;
use nami::SignalExt;
use nami::signal::IntoComputed;
use waterui::{Binding, Str};
use waterui_video::{
    AspectRatio, AudioTrackInfo, Delivery, LiveWindow, PlaybackMetrics, PlaybackOutputPath,
    PlaybackPhase, PlaybackPowerPolicy, PlayerController, RepeatMode, SubtitleTrackInfo,
    SubtitleTrackOrigin, TrackCatalog, VideoProjection, VideoTrackInfo, Volume,
    video::{
        AudioTrackSelection, BoundVideoEventHandler, Event as VideoEvent, NativeVideoConfig,
        NativeVideoPlayerConfig, PlaybackConfiguration, PlaybackPolicy, SubtitleSelection,
        VideoTrackSelection,
    },
};
#[cfg(target_os = "android")]
use waterui_video_gpu::{AndroidVideoSurfaceBridge, AndroidVideoSurfaceHost};

/// JNI projection of the self-drawn player's Android secure-surface wrapper.
#[cfg(target_os = "android")]
#[repr(C)]
pub struct WuiAndroidVideoSurfaceHost {
    pub content: *mut crate::WuiAnyView,
    pub bridge: *mut AndroidVideoSurfaceBridge,
}

#[cfg(target_os = "android")]
impl IntoFFI for AndroidVideoSurfaceHost {
    type FFI = WuiAndroidVideoSurfaceHost;

    fn into_ffi(self) -> Self::FFI {
        let (content, bridge) = self.into_parts();
        Self::FFI {
            content: content.into_ffi(),
            bridge: Box::into_raw(Box::new(bridge)),
        }
    }
}

#[cfg(target_os = "android")]
ffi_view!(
    AndroidVideoSurfaceHost,
    WuiAndroidVideoSurfaceHost,
    android_video_surface_host,
    all()
);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum WuiAspectRatio {
    Fit = 0,
    Fill = 1,
    Stretch = 2,
}

/// Native projection support discriminator.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiVideoProjection {
    Rectilinear = 0,
    Equirectangular = 1,
}

impl IntoFFI for VideoProjection {
    type FFI = WuiVideoProjection;

    fn into_ffi(self) -> Self::FFI {
        match self {
            VideoProjection::Rectilinear => WuiVideoProjection::Rectilinear,
            VideoProjection::Equirectangular(_) => WuiVideoProjection::Equirectangular,
        }
    }
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

/// FFI representation of a media delivery strategy.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiVideoDelivery {
    Progressive = 0,
    Hls = 1,
    Dash = 2,
}

impl IntoFFI for Delivery {
    type FFI = WuiVideoDelivery;

    fn into_ffi(self) -> Self::FFI {
        match self {
            Delivery::Progressive => WuiVideoDelivery::Progressive,
            Delivery::Hls => WuiVideoDelivery::Hls,
            Delivery::Dash => WuiVideoDelivery::Dash,
        }
    }
}

impl IntoRust for WuiVideoDelivery {
    type Rust = Delivery;

    unsafe fn into_rust(self) -> Self::Rust {
        match self {
            Self::Progressive => Delivery::Progressive,
            Self::Hls => Delivery::Hls,
            Self::Dash => Delivery::Dash,
        }
    }
}

crate::ffi_computed!(Delivery, WuiVideoDelivery, video_delivery);

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
    ExternalPlaybackChanged = 11,
    PlaybackOutputPathChanged = 12,
}

/// Native representation of the active playback output architecture.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiVideoPlaybackOutputPath {
    PlatformManaged = 0,
    DecodedPcmGpu = 1,
    AudioOffload = 2,
    AudioVideoTunneling = 3,
}

impl From<WuiVideoPlaybackOutputPath> for PlaybackOutputPath {
    fn from(path: WuiVideoPlaybackOutputPath) -> Self {
        match path {
            WuiVideoPlaybackOutputPath::PlatformManaged => Self::PlatformManaged,
            WuiVideoPlaybackOutputPath::DecodedPcmGpu => Self::DecodedPcmGpu,
            WuiVideoPlaybackOutputPath::AudioOffload => Self::AudioOffload,
            WuiVideoPlaybackOutputPath::AudioVideoTunneling => Self::AudioVideoTunneling,
        }
    }
}

/// FFI representation of a video event.
#[repr(C)]
pub struct WuiVideoEvent {
    pub event_type: WuiVideoEventType,
    /// Rust-owned error string. Non-null exactly when `event_type` is `Error`.
    pub error_message: *mut WuiStr,
    pub position_ms: u64,
    pub buffered_ms: u32,
    pub startup_time_ms: u64,
    pub av_drift_available: bool,
    pub av_drift_ms: f32,
    pub dropped_video_frames: u64,
    pub rebuffer_count: u64,
    pub rebuffer_duration_ms: u64,
    pub observed_network_throughput_bps: u64,
    pub picture_in_picture_active: bool,
    pub external_playback_active: bool,
    pub playback_active: bool,
    pub playback_output_path: WuiVideoPlaybackOutputPath,
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
        WuiVideoEventType::ExternalPlaybackChanged => VideoEvent::ExternalPlaybackChanged {
            active: ffi_event.external_playback_active,
        },
        WuiVideoEventType::NextRequested => VideoEvent::NextRequested,
        WuiVideoEventType::PreviousRequested => VideoEvent::PreviousRequested,
        WuiVideoEventType::PlaybackStateChanged => VideoEvent::PlaybackStateChanged {
            playing: ffi_event.playback_active,
        },
        WuiVideoEventType::PlaybackOutputPathChanged => VideoEvent::PlaybackOutputPathChanged {
            path: ffi_event.playback_output_path.into(),
        },
        WuiVideoEventType::BufferLevel => VideoEvent::BufferLevel {
            buffered_ms: ffi_event.buffered_ms,
        },
        WuiVideoEventType::PlaybackMetrics => {
            let mut metrics = PlaybackMetrics::new(
                core::time::Duration::from_millis(ffi_event.position_ms),
                core::time::Duration::from_millis(u64::from(ffi_event.buffered_ms)),
                core::time::Duration::from_millis(ffi_event.startup_time_ms),
            )
            .dropped_video_frames(ffi_event.dropped_video_frames)
            .rebuffering(
                ffi_event.rebuffer_count,
                core::time::Duration::from_millis(ffi_event.rebuffer_duration_ms),
            );
            if ffi_event.av_drift_available {
                metrics = metrics.av_drift_ms(ffi_event.av_drift_ms);
            }
            if let Some(bits_per_second) =
                NonZeroU64::new(ffi_event.observed_network_throughput_bps)
            {
                metrics = metrics.observed_network_throughput(bits_per_second);
            }
            VideoEvent::PlaybackMetrics { metrics }
        }
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

/// Tagged audio track selection used by the native player.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiAudioTrackSelectionType {
    Auto = 0,
    Track = 1,
}

/// FFI representation of [`AudioTrackSelection`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WuiAudioTrackSelection {
    pub selection_type: WuiAudioTrackSelectionType,
    /// Track index. Read only when `selection_type` is `Track`.
    pub track_index: usize,
}

impl IntoFFI for AudioTrackSelection {
    type FFI = WuiAudioTrackSelection;

    fn into_ffi(self) -> Self::FFI {
        match self {
            AudioTrackSelection::Auto => WuiAudioTrackSelection {
                selection_type: WuiAudioTrackSelectionType::Auto,
                track_index: 0,
            },
            AudioTrackSelection::Track(track_index) => WuiAudioTrackSelection {
                selection_type: WuiAudioTrackSelectionType::Track,
                track_index,
            },
        }
    }
}

impl IntoRust for WuiAudioTrackSelection {
    type Rust = AudioTrackSelection;

    unsafe fn into_rust(self) -> Self::Rust {
        match self.selection_type {
            WuiAudioTrackSelectionType::Auto => AudioTrackSelection::Auto,
            WuiAudioTrackSelectionType::Track => AudioTrackSelection::Track(self.track_index),
        }
    }
}

#[cfg(feature = "c-api")]
crate::ffi_binding!(
    AudioTrackSelection,
    WuiAudioTrackSelection,
    audio_track_selection
);
#[cfg(feature = "c-api")]
crate::ffi_watcher!(
    AudioTrackSelection,
    WuiAudioTrackSelection,
    audio_track_selection
);

/// Tagged video-quality track selection used by the native player.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiVideoTrackSelectionType {
    Auto = 0,
    Track = 1,
}

/// FFI representation of [`VideoTrackSelection`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WuiVideoTrackSelection {
    pub selection_type: WuiVideoTrackSelectionType,
    /// Track index. Read only when `selection_type` is `Track`.
    pub track_index: usize,
}

impl IntoFFI for VideoTrackSelection {
    type FFI = WuiVideoTrackSelection;

    fn into_ffi(self) -> Self::FFI {
        match self {
            VideoTrackSelection::Auto => WuiVideoTrackSelection {
                selection_type: WuiVideoTrackSelectionType::Auto,
                track_index: 0,
            },
            VideoTrackSelection::Track(track_index) => WuiVideoTrackSelection {
                selection_type: WuiVideoTrackSelectionType::Track,
                track_index,
            },
        }
    }
}

impl IntoRust for WuiVideoTrackSelection {
    type Rust = VideoTrackSelection;

    unsafe fn into_rust(self) -> Self::Rust {
        match self.selection_type {
            WuiVideoTrackSelectionType::Auto => VideoTrackSelection::Auto,
            WuiVideoTrackSelectionType::Track => VideoTrackSelection::Track(self.track_index),
        }
    }
}

#[cfg(feature = "c-api")]
crate::ffi_binding!(
    VideoTrackSelection,
    WuiVideoTrackSelection,
    video_track_selection
);
#[cfg(feature = "c-api")]
crate::ffi_watcher!(
    VideoTrackSelection,
    WuiVideoTrackSelection,
    video_track_selection
);

/// Native representation of one selectable audio track.
#[repr(C)]
#[derive(Default)]
pub struct WuiVideoAudioTrackInfo {
    pub label: WuiStr,
    pub language: WuiStr,
    pub roles: WuiArray<WuiStr>,
}

impl IntoRust for WuiVideoAudioTrackInfo {
    type Rust = AudioTrackInfo;

    unsafe fn into_rust(self) -> Self::Rust {
        AudioTrackInfo::new(
            unsafe { self.label.into_rust() }.to_string(),
            optional_string(self.language),
            string_array(self.roles),
        )
    }
}

/// Native representation of one selectable video representation.
#[repr(C)]
#[derive(Default)]
pub struct WuiVideoTrackInfo {
    pub id: WuiStr,
    pub label: WuiStr,
    pub bandwidth: u64,
    pub width: u32,
    pub height: u32,
    pub codecs: WuiArray<WuiStr>,
    pub hdr: bool,
}

impl IntoRust for WuiVideoTrackInfo {
    type Rust = VideoTrackInfo;

    unsafe fn into_rust(self) -> Self::Rust {
        let bandwidth = NonZeroU64::new(self.bandwidth);
        let dimensions = match (self.width, self.height) {
            (0, 0) => None,
            (width, height) if width > 0 && height > 0 => Some((width, height)),
            _ => panic!("native video-track dimensions must both be zero or both be non-zero"),
        };
        VideoTrackInfo::new(
            unsafe { self.id.into_rust() }.to_string(),
            unsafe { self.label.into_rust() }.to_string(),
            bandwidth,
            dimensions,
            string_array(self.codecs),
            self.hdr,
        )
    }
}

/// Native subtitle-track origin.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum WuiVideoSubtitleTrackOrigin {
    Sidecar = 0,
    Embedded = 1,
    Manifest = 2,
    #[default]
    Native = 3,
}

impl From<WuiVideoSubtitleTrackOrigin> for SubtitleTrackOrigin {
    fn from(origin: WuiVideoSubtitleTrackOrigin) -> Self {
        match origin {
            WuiVideoSubtitleTrackOrigin::Sidecar => Self::Sidecar,
            WuiVideoSubtitleTrackOrigin::Embedded => Self::Embedded,
            WuiVideoSubtitleTrackOrigin::Manifest => Self::Manifest,
            WuiVideoSubtitleTrackOrigin::Native => Self::Native,
        }
    }
}

/// Native representation of one selectable subtitle track.
#[repr(C)]
#[derive(Default)]
pub struct WuiVideoSubtitleTrackInfo {
    pub label: WuiStr,
    pub language: WuiStr,
    pub roles: WuiArray<WuiStr>,
    pub forced: bool,
    pub origin: WuiVideoSubtitleTrackOrigin,
}

impl IntoRust for WuiVideoSubtitleTrackInfo {
    type Rust = SubtitleTrackInfo;

    unsafe fn into_rust(self) -> Self::Rust {
        SubtitleTrackInfo::new(
            unsafe { self.label.into_rust() }.to_string(),
            optional_string(self.language),
            string_array(self.roles),
            self.forced,
            self.origin.into(),
        )
    }
}

fn optional_string(value: WuiStr) -> Option<String> {
    let value = unsafe { value.into_rust() }.to_string();
    (!value.is_empty()).then_some(value)
}

fn string_array(values: WuiArray<WuiStr>) -> Vec<String> {
    unsafe { values.into_rust() }
        .into_iter()
        .map(|value| value.to_string())
        .collect()
}

/// Replaces the audio portion of a native player's shared track catalog.
///
/// # Safety
///
/// `binding` must be a live pointer from a video descriptor. `tracks` transfers
/// ownership of its outer array and every nested string to Rust.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_video_track_catalog_replace_audio(
    binding: *const WuiBinding<TrackCatalog>,
    tracks: WuiArray<WuiVideoAudioTrackInfo>,
) {
    let tracks = unsafe { tracks.into_rust() };
    let binding = unsafe { crate::borrow_ffi(binding) };
    binding.set(binding.get().replacing_audio(tracks));
}

/// Replaces the video portion of a native player's shared track catalog.
///
/// # Safety
///
/// `binding` must be a live pointer from a video descriptor. `tracks` transfers
/// ownership of its outer array and every nested string to Rust.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_video_track_catalog_replace_video(
    binding: *const WuiBinding<TrackCatalog>,
    tracks: WuiArray<WuiVideoTrackInfo>,
) {
    let tracks = unsafe { tracks.into_rust() };
    let binding = unsafe { crate::borrow_ffi(binding) };
    binding.set(binding.get().replacing_video(tracks));
}

/// Replaces the subtitle portion of a native player's shared track catalog.
///
/// # Safety
///
/// `binding` must be a live pointer from a video descriptor. `tracks` transfers
/// ownership of its outer array and every nested string to Rust.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_video_track_catalog_replace_subtitles(
    binding: *const WuiBinding<TrackCatalog>,
    tracks: WuiArray<WuiVideoSubtitleTrackInfo>,
) {
    let tracks = unsafe { tracks.into_rust() };
    let binding = unsafe { crate::borrow_ffi(binding) };
    binding.set(binding.get().replacing_subtitles(tracks));
}

/// Drops the native write handle for a shared selectable-track catalog.
///
/// # Safety
///
/// `binding` must be a live pointer from a video descriptor and must be
/// consumed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_video_track_catalog_binding(
    binding: *mut WuiBinding<TrackCatalog>,
) {
    unsafe { drop(alloc::boxed::Box::from_raw(binding)) };
}

/// Atomic native snapshot of a live presentation timeline.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct WuiVideoLiveWindow {
    /// Whether the remaining fields contain a live timeline.
    pub present: bool,
    /// Earliest seekable presentation position in seconds.
    pub seekable_start_seconds: f64,
    /// Latest seekable presentation position in seconds.
    pub seekable_end_seconds: f64,
    /// Current live edge in presentation seconds.
    pub live_edge_seconds: f64,
    /// Backend-recommended playback position in presentation seconds.
    pub target_position_seconds: f64,
}

/// Replaces the current live-window snapshot atomically.
///
/// # Safety
///
/// `binding` must be a live pointer from a video descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_video_live_window_replace(
    binding: *const WuiBinding<Option<LiveWindow>>,
    window: WuiVideoLiveWindow,
) {
    let binding = unsafe { crate::borrow_ffi(binding) };
    let window = window.present.then(|| {
        LiveWindow::new(
            duration_from_native_seconds(window.seekable_start_seconds, "seekable start"),
            duration_from_native_seconds(window.seekable_end_seconds, "seekable end"),
            duration_from_native_seconds(window.live_edge_seconds, "live edge"),
            duration_from_native_seconds(window.target_position_seconds, "target position"),
        )
    });
    binding.set(window);
}

fn duration_from_native_seconds(seconds: f64, field: &str) -> core::time::Duration {
    assert!(
        seconds.is_finite() && seconds >= 0.0,
        "native video {field} seconds must be finite and non-negative"
    );
    core::time::Duration::from_secs_f64(seconds)
}

/// Drops the native write handle for a live-window binding.
///
/// # Safety
///
/// `binding` must be a live pointer from a video descriptor and must be
/// consumed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_video_live_window_binding(
    binding: *mut WuiBinding<Option<LiveWindow>>,
) {
    unsafe { drop(alloc::boxed::Box::from_raw(binding)) };
}

/// Immutable buffering and realtime strategy for native playback.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WuiVideoPlaybackPolicy {
    pub realtime: bool,
    pub vod_start_buffer_ms: u32,
    pub vod_resume_buffer_ms: u32,
    pub vod_stall_buffer_ms: u32,
    pub live_max_video_late_ms: u32,
    pub power: WuiVideoPlaybackPowerPolicy,
}

/// Native projection of a required playback power path.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiVideoPlaybackPowerPolicy {
    PlatformManaged = 0,
    RequireAudioOffload = 1,
    RequireAudioVideoTunneling = 2,
}

impl From<PlaybackPowerPolicy> for WuiVideoPlaybackPowerPolicy {
    fn from(policy: PlaybackPowerPolicy) -> Self {
        match policy {
            PlaybackPowerPolicy::PlatformManaged => Self::PlatformManaged,
            PlaybackPowerPolicy::RequireAudioOffload => Self::RequireAudioOffload,
            PlaybackPowerPolicy::RequireAudioVideoTunneling => Self::RequireAudioVideoTunneling,
        }
    }
}

impl From<PlaybackPolicy> for WuiVideoPlaybackPolicy {
    fn from(policy: PlaybackPolicy) -> Self {
        Self {
            realtime: policy.realtime,
            vod_start_buffer_ms: policy.vod_start_buffer_ms,
            vod_resume_buffer_ms: policy.vod_resume_buffer_ms,
            vod_stall_buffer_ms: policy.vod_stall_buffer_ms,
            live_max_video_late_ms: policy.live_max_video_late_ms,
            power: policy.power.into(),
        }
    }
}

opaque!(
    WuiVideoEventHandler,
    Rc<BoundVideoEventHandler>,
    video_event_handler,
    any()
);

opaque!(
    WuiVideoController,
    PlayerController,
    video_controller,
    any()
);

/// Selects the next item through the session controller retained by a native player.
///
/// # Safety
///
/// `controller` must be a live pointer from a video playback descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_video_controller_next(controller: *const WuiVideoController) {
    unsafe { crate::borrow_ffi(controller) }
        .0
        .next()
        .expect("enabled native next command must resolve a playlist item");
}

/// Selects the previous item through the session controller retained by a native player.
///
/// # Safety
///
/// `controller` must be a live pointer from a video playback descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_video_controller_previous(controller: *const WuiVideoController) {
    unsafe { crate::borrow_ffi(controller) }
        .0
        .previous()
        .expect("enabled native previous command must resolve a playlist item");
}

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
    /// Retained playlist and transport controller.
    pub controller: *mut WuiVideoController,
    /// The video source URL as a string (reactive).
    pub source: *mut WuiComputed<Str>,
    /// The source delivery strategy (reactive).
    pub delivery: *mut WuiComputed<Delivery>,
    /// The media title shown in system media controls.
    pub title: *mut WuiComputed<Str>,
    /// The media artist shown in system media controls.
    pub artist: *mut WuiComputed<Str>,
    /// The media album shown in system media controls.
    pub album: *mut WuiComputed<Str>,
    /// Artwork URL shown in system media controls.
    pub artwork_url: *mut WuiComputed<Str>,
    /// Current playback duration in seconds, or `0.0` when unknown.
    pub duration_seconds: *mut WuiBinding<f64>,
    /// Current presentation position in seconds.
    pub position_seconds: *mut WuiBinding<f64>,
    /// Requested playing state.
    pub desired_playing: *mut WuiBinding<bool>,
    /// Requested absolute seek position in presentation seconds.
    pub seek_target_seconds: *mut WuiBinding<f64>,
    /// Monotonic seek-request generation encoded as a wrapping `i32` token.
    pub seek_generation: *mut WuiBinding<i32>,
    /// Monotonic forward frame-step generation encoded as a wrapping `i32` token.
    pub step_forward_generation: *mut WuiBinding<i32>,
    /// Monotonic backward frame-step generation encoded as a wrapping `i32` token.
    pub step_backward_generation: *mut WuiBinding<i32>,
    /// High-level playback phase encoded by `playback_phase_code`.
    pub phase: *mut WuiBinding<i32>,
    /// Playlist repeat mode encoded by `repeat_mode_code`.
    pub repeat_mode: *mut WuiBinding<i32>,
    /// Whether the active queue has a next item.
    pub has_next: *mut WuiBinding<bool>,
    /// Whether the active queue has a previous item.
    pub has_previous: *mut WuiBinding<bool>,
    /// Validated playback volume in the inclusive `0.0..=1.0` range.
    pub volume: *mut WuiBinding<f32>,
    /// Whether playback audio is muted.
    pub muted: *mut WuiBinding<bool>,
    /// Subtitle selection for the current runtime track list.
    pub subtitle_selection: *mut WuiBinding<SubtitleSelection>,
    /// Audio track selection for the current runtime track list.
    pub audio_track_selection: *mut WuiBinding<AudioTrackSelection>,
    /// Video-quality selection for the current runtime track list.
    pub video_track_selection: *mut WuiBinding<VideoTrackSelection>,
    /// Native write handle for the shared selectable-track catalog.
    pub track_catalog: *mut WuiBinding<TrackCatalog>,
    /// Native write handle for the current atomic live-window snapshot.
    pub live_window: *mut WuiBinding<Option<LiveWindow>>,
    /// Playback speed (1.0 = normal speed).
    pub playback_rate: *mut WuiBinding<f32>,
    /// Whether native playback should preserve pitch when rate changes.
    pub preserve_pitch: *mut WuiBinding<bool>,
    /// Optional event handler for native playback events.
    pub on_event: *mut WuiVideoEventHandler,
    /// Buffering and realtime playback strategy.
    pub playback_policy: WuiVideoPlaybackPolicy,
}

fn playback_phase_code(phase: PlaybackPhase) -> i32 {
    match phase {
        PlaybackPhase::Idle => 0,
        PlaybackPhase::Preparing => 1,
        PlaybackPhase::Ready => 2,
        PlaybackPhase::Playing => 3,
        PlaybackPhase::Paused => 4,
        PlaybackPhase::Buffering => 5,
        PlaybackPhase::Ended => 6,
        PlaybackPhase::Failed => 7,
    }
}

fn playback_phase_from_code(code: i32) -> PlaybackPhase {
    match code {
        0 => PlaybackPhase::Idle,
        1 => PlaybackPhase::Preparing,
        2 => PlaybackPhase::Ready,
        3 => PlaybackPhase::Playing,
        4 => PlaybackPhase::Paused,
        5 => PlaybackPhase::Buffering,
        6 => PlaybackPhase::Ended,
        7 => PlaybackPhase::Failed,
        _ => panic!("unsupported native video playback phase code {code}"),
    }
}

fn repeat_mode_code(mode: RepeatMode) -> i32 {
    match mode {
        RepeatMode::Off => 0,
        RepeatMode::One => 1,
        RepeatMode::All => 2,
    }
}

fn repeat_mode_from_code(code: i32) -> RepeatMode {
    match code {
        0 => RepeatMode::Off,
        1 => RepeatMode::One,
        2 => RepeatMode::All,
        _ => panic!("unsupported native video repeat mode code {code}"),
    }
}

fn generation_binding(generation: &Binding<u64>) -> Binding<i32> {
    Binding::mapping(
        generation,
        |generation| generation as i32,
        |generation, _| generation.set(generation.get().wrapping_add(1)),
    )
}

fn into_playback_descriptor(
    playback: PlaybackConfiguration<Option<BoundVideoEventHandler>>,
) -> WuiVideoPlaybackDescriptor {
    let PlaybackConfiguration {
        controller,
        source,
        has_next,
        has_previous,
        volume,
        muted,
        subtitle_selection,
        audio_track_selection,
        video_track_selection,
        track_catalog,
        live_window,
        playback_rate,
        preserve_pitch,
        desired_playing,
        seek_target_seconds,
        seek_generation,
        step_forward_generation,
        step_backward_generation,
        position_seconds,
        duration_seconds,
        phase,
        repeat,
        shuffle: _,
        on_event,
        playback_policy,
    } = playback;

    let delivery = source.clone().map(|item| item.delivery).into_computed();
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
    WuiVideoPlaybackDescriptor {
        controller: controller.into_ffi(),
        source: source_str.into_ffi(),
        delivery: delivery.into_ffi(),
        title: title.into_ffi(),
        artist: artist.into_ffi(),
        album: album.into_ffi(),
        artwork_url: artwork_url.into_ffi(),
        duration_seconds: duration_seconds.into_ffi(),
        position_seconds: position_seconds.into_ffi(),
        desired_playing: desired_playing.into_ffi(),
        seek_target_seconds: seek_target_seconds.into_ffi(),
        seek_generation: generation_binding(&seek_generation).into_ffi(),
        step_forward_generation: generation_binding(&step_forward_generation).into_ffi(),
        step_backward_generation: generation_binding(&step_backward_generation).into_ffi(),
        phase: Binding::mapping(&phase, playback_phase_code, |phase, code| {
            phase.set(playback_phase_from_code(code));
        })
        .into_ffi(),
        repeat_mode: Binding::mapping(&repeat, repeat_mode_code, |repeat, code| {
            repeat.set(repeat_mode_from_code(code));
        })
        .into_ffi(),
        has_next: has_next.into_ffi(),
        has_previous: has_previous.into_ffi(),
        volume: Binding::mapping(&volume, Volume::level, |volume, level| {
            volume.set(Volume::new(level))
        })
        .into_ffi(),
        muted: muted.into_ffi(),
        subtitle_selection: subtitle_selection.into_ffi(),
        audio_track_selection: audio_track_selection.into_ffi(),
        video_track_selection: video_track_selection.into_ffi(),
        track_catalog: track_catalog.into_ffi(),
        live_window: live_window.into_ffi(),
        playback_rate: playback_rate.into_ffi(),
        preserve_pitch: preserve_pitch.into_ffi(),
        on_event: on_event.into_ffi(),
        playback_policy: playback_policy.into(),
    }
}

/// FFI representation of the raw Video component (no native controls).
#[repr(C)]
pub struct WuiVideo {
    /// Shared reactive playback state.
    pub playback: WuiVideoPlaybackDescriptor,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: WuiAspectRatio,
    /// Projection requested by the semantic component.
    pub projection: WuiVideoProjection,
    /// Whether the video should loop when it ends.
    pub loops: bool,
}

impl IntoFFI for NativeVideoConfig {
    type FFI = WuiVideo;

    fn into_ffi(self) -> Self::FFI {
        let aspect_ratio = self.aspect_ratio.into_ffi();
        let projection = self.projection.into_ffi();
        let loops = self.loops;
        let playback = into_playback_descriptor(self.playback);

        WuiVideo {
            playback,
            aspect_ratio,
            projection,
            loops,
        }
    }
}

/// FFI representation of the interactive VideoPlayer component.
#[repr(C)]
pub struct WuiVideoPlayer {
    /// Shared reactive playback state.
    pub playback: WuiVideoPlaybackDescriptor,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: WuiAspectRatio,
    /// Projection requested by the semantic component.
    pub projection: WuiVideoProjection,
    /// Whether to show interactive playback controls.
    pub show_controls: bool,
}

impl IntoFFI for NativeVideoPlayerConfig {
    type FFI = WuiVideoPlayer;

    fn into_ffi(self) -> Self::FFI {
        let aspect_ratio = self.aspect_ratio.into_ffi();
        let projection = self.projection.into_ffi();
        let show_controls = self.show_controls;
        let playback = into_playback_descriptor(self.playback);

        WuiVideoPlayer {
            playback,
            aspect_ratio,
            projection,
            show_controls,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_core::binding;

    #[test]
    fn native_playback_policy_preserves_required_power_path() {
        let native = WuiVideoPlaybackPolicy::from(
            PlaybackPolicy::vod_default().power(PlaybackPowerPolicy::RequireAudioVideoTunneling),
        );
        assert_eq!(
            native.power,
            WuiVideoPlaybackPowerPolicy::RequireAudioVideoTunneling
        );
    }

    #[test]
    fn native_track_catalog_updates_preserve_complete_metadata() {
        let binding = binding(TrackCatalog::default()).into_ffi();

        unsafe {
            waterui_video_track_catalog_replace_audio(
                binding,
                WuiArray::new(vec![WuiVideoAudioTrackInfo {
                    label: "English".into_ffi(),
                    language: "en".into_ffi(),
                    roles: vec!["main"].into_ffi(),
                }]),
            );
            waterui_video_track_catalog_replace_video(
                binding,
                WuiArray::new(vec![WuiVideoTrackInfo {
                    id: "native-main".into_ffi(),
                    label: "3840×2160".into_ffi(),
                    bandwidth: 0,
                    width: 3840,
                    height: 2160,
                    codecs: vec!["hvc1"].into_ffi(),
                    hdr: true,
                }]),
            );
            waterui_video_track_catalog_replace_subtitles(
                binding,
                WuiArray::new(vec![WuiVideoSubtitleTrackInfo {
                    label: "English CC".into_ffi(),
                    language: "en".into_ffi(),
                    roles: vec!["caption"].into_ffi(),
                    forced: false,
                    origin: WuiVideoSubtitleTrackOrigin::Native,
                }]),
            );
        }

        let catalog = unsafe { crate::borrow_ffi(binding) }.get();
        assert_eq!(catalog.audio()[0].roles(), &[String::from("main")]);
        assert_eq!(catalog.video()[0].bandwidth(), None);
        assert_eq!(catalog.video()[0].dimensions(), Some((3840, 2160)));
        assert!(catalog.video()[0].is_hdr());
        assert_eq!(catalog.subtitles()[0].origin(), SubtitleTrackOrigin::Native);

        unsafe { waterui_drop_video_track_catalog_binding(binding) };
    }

    #[test]
    fn native_live_window_is_replaced_as_one_validated_snapshot() {
        let binding = binding(None::<LiveWindow>).into_ffi();

        unsafe {
            waterui_video_live_window_replace(
                binding,
                WuiVideoLiveWindow {
                    present: true,
                    seekable_start_seconds: 30.0,
                    seekable_end_seconds: 90.0,
                    live_edge_seconds: 92.0,
                    target_position_seconds: 86.0,
                },
            );
        }

        let window = unsafe { crate::borrow_ffi(binding) }
            .get()
            .expect("native live snapshot must be present");
        assert_eq!(window.seekable_start(), core::time::Duration::from_secs(30));
        assert_eq!(
            window.target_live_offset(),
            core::time::Duration::from_secs(6)
        );

        unsafe {
            waterui_video_live_window_replace(binding, WuiVideoLiveWindow::default());
        }
        assert!(unsafe { crate::borrow_ffi(binding) }.get().is_none());
        unsafe { waterui_drop_video_live_window_binding(binding) };
    }

    #[test]
    fn native_playback_metrics_preserve_optional_observations() {
        let event = into_video_event(WuiVideoEvent {
            event_type: WuiVideoEventType::PlaybackMetrics,
            error_message: core::ptr::null_mut(),
            position_ms: 12_000,
            buffered_ms: 3_500,
            startup_time_ms: 420,
            av_drift_available: true,
            av_drift_ms: -2.5,
            dropped_video_frames: 3,
            rebuffer_count: 2,
            rebuffer_duration_ms: 750,
            observed_network_throughput_bps: 16_000_000,
            picture_in_picture_active: false,
            external_playback_active: false,
            playback_active: true,
            playback_output_path: WuiVideoPlaybackOutputPath::DecodedPcmGpu,
        });
        let VideoEvent::PlaybackMetrics { metrics } = event else {
            panic!("native metrics event must preserve its semantic variant");
        };

        assert_eq!(metrics.position(), core::time::Duration::from_secs(12));
        assert_eq!(
            metrics.buffered_ahead(),
            core::time::Duration::from_millis(3_500)
        );
        assert_eq!(
            metrics.startup_time(),
            core::time::Duration::from_millis(420)
        );
        assert_eq!(metrics.observed_av_drift_ms(), Some(-2.5));
        assert_eq!(metrics.dropped_video_frame_count(), 3);
        assert_eq!(metrics.rebuffer_count(), 2);
        assert_eq!(
            metrics.rebuffer_duration(),
            core::time::Duration::from_millis(750)
        );
        assert_eq!(
            metrics.observed_network_throughput_bps(),
            NonZeroU64::new(16_000_000)
        );
    }

    #[test]
    fn native_external_playback_state_preserves_player_scope() {
        let event = into_video_event(WuiVideoEvent {
            event_type: WuiVideoEventType::ExternalPlaybackChanged,
            error_message: core::ptr::null_mut(),
            position_ms: 0,
            buffered_ms: 0,
            startup_time_ms: 0,
            av_drift_available: false,
            av_drift_ms: 0.0,
            dropped_video_frames: 0,
            rebuffer_count: 0,
            rebuffer_duration_ms: 0,
            observed_network_throughput_bps: 0,
            picture_in_picture_active: false,
            external_playback_active: true,
            playback_active: false,
            playback_output_path: WuiVideoPlaybackOutputPath::PlatformManaged,
        });
        assert!(matches!(
            event,
            VideoEvent::ExternalPlaybackChanged { active: true }
        ));
    }

    #[test]
    fn native_output_path_preserves_tunneling_observability() {
        let event = into_video_event(WuiVideoEvent {
            event_type: WuiVideoEventType::PlaybackOutputPathChanged,
            error_message: core::ptr::null_mut(),
            position_ms: 0,
            buffered_ms: 0,
            startup_time_ms: 0,
            av_drift_available: false,
            av_drift_ms: 0.0,
            dropped_video_frames: 0,
            rebuffer_count: 0,
            rebuffer_duration_ms: 0,
            observed_network_throughput_bps: 0,
            picture_in_picture_active: false,
            external_playback_active: false,
            playback_active: false,
            playback_output_path: WuiVideoPlaybackOutputPath::AudioVideoTunneling,
        });
        assert!(matches!(
            event,
            VideoEvent::PlaybackOutputPathChanged {
                path: PlaybackOutputPath::AudioVideoTunneling
            }
        ));
    }
}

#[cfg(feature = "c-api")]
ffi_view!(NativeVideoConfig, WuiVideo, video, any());

#[cfg(feature = "c-api")]
ffi_view!(NativeVideoPlayerConfig, WuiVideoPlayer, video_player, any());
