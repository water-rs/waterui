use async_channel::{Receiver as AsyncReceiver, Sender as AsyncSender};
use core::fmt;
use futures::FutureExt as _;
use std::{
    borrow::Cow,
    collections::VecDeque,
    fs,
    num::{NonZeroU64, NonZeroUsize},
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use executor_core::spawn_local;
use nami::{Computed, watcher::BoxWatcherGuard};
use num_traits::ToPrimitive;
use uuid::Uuid;
use waterkit_audio::{
    AudioOutput, MediaCommand, MediaMetadata, MediaSession, PlaybackState, QueueNavigationControls,
    StreamingAudioPlayer,
};
use waterkit_codec::{
    ColorOutputTarget, DecodedFrame, DecodedFrameUploader, DecodedPixelLayout,
    GpuFrame as DecodedGpuFrame, VideoColorUniform, YUV_COLOR_SHADER_WGSL, video_color_uniform,
};
#[cfg(target_os = "android")]
use waterkit_video::AndroidOffloadAudioController;
use waterkit_video::streaming::{
    AssetCache, BandwidthEstimator, DownloadEvent, ProgressiveDownloadRequest, Url as StreamingUrl,
    download,
};
use waterkit_video::{
    DecodedVideoFrame as EngineDecodedVideoFrame,
    EmbeddedSubtitleTrack as EmbeddedSubtitleSourceTrack,
    LivePlaybackRateRange as EngineLivePlaybackRateRange, LiveWindow as EngineLiveWindow,
    PictureInPictureCommand, PictureInPictureCommandStream, PictureInPictureController,
    PictureInPictureControllerState, PictureInPictureHostId, SelectableAudioTrack,
    SelectableSubtitleTrack, SelectableVideoTrack, SubtitleCue,
    SubtitleTrackSelection as EngineSubtitleTrackSelection, TimedMetadata as EngineTimedMetadata,
    VideoColorInfo, VideoReader, active_subtitle_text, embedded_subtitle_tracks,
    parse_subtitles_from_path, read_embedded_subtitle_cues,
};
use waterui_controls::{button, slider::slider};
use waterui_core::{
    AnyView, Binding, Environment, IgnorableMetadata, Metadata, SignalExt as _, State, View,
    accessibility::{AccessibilityLabel, AccessibilityRole},
    binding,
    env::With,
    gesture::{
        DragEvent, DragGesture, GestureObserver, GesturePhase, MagnificationEvent,
        MagnificationGesture,
    },
    layout::{ProposalSize, Size, StretchAxis, ViewDimensions},
};
use waterui_graphics::{Color, GpuContext, GpuFrame, GpuSurface, GpuView, RedrawHandle};
use waterui_layout::{
    frame::Frame,
    overlay,
    stack::{Alignment, hstack, vstack},
};
use waterui_text::{Text, text};

use waterui_video::video::VideoEventHandler;
use waterui_video::{
    AspectRatio, AudioTrackInfo, AudioTrackSelection, Delivery, DrmConfiguration,
    EquirectangularProjection, Event, LiveWindow, MediaItem, PlaybackConfiguration,
    PlaybackMetrics, PlaybackOutputPath, PlaybackPhase, PlaybackPolicy, PlaybackPowerPolicy,
    PlayerController, RepeatMode, SphericalStereoLayout, SphericalViewport, SubtitleSelection,
    SubtitleTrack, SubtitleTrackInfo, SubtitleTrackOrigin, TimedMetadata, TrackCatalog, Url,
    VideoConfig, VideoPlayerConfig, VideoProjection, VideoTrackInfo, VideoTrackSelection, Volume,
};

use crate::VideoGpuOptions;
#[cfg(target_os = "android")]
use crate::android_video_surface::{
    AndroidVideoSurfaceBridge, AndroidVideoSurfaceHost, AndroidVideoSurfaceReceiver,
    video_surface_channel,
};
#[cfg(target_os = "android")]
use crate::decoder_worker::{
    AndroidAudioProcessing, AndroidAudioRoute, AndroidPlaybackClock, AndroidPlaybackConfig,
    AndroidPowerCompatibility, AndroidVideoAccess,
};
use crate::decoder_worker::{
    DecoderOutput, DecoderWorker, ProgressiveDecoderConfig, SegmentedDecoderConfig,
    SegmentedProtocol,
};
use crate::latest_channel::{LatestSender, latest_channel};

const SEEK_POSITION_EPSILON: Duration = Duration::from_millis(5);
const SEEK_RESTART_THROTTLE: Duration = Duration::from_millis(40);
#[cfg(target_os = "android")]
const PICTURE_IN_PICTURE_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const PRESENT_TOLERANCE: Duration = Duration::from_millis(3);
const VOD_FRAME_DROP_THRESHOLD: Duration = Duration::from_millis(300);
const STREAMING_PROBE_INTERVAL_BYTES: usize = 256 * 1024;
const STREAMING_MIN_READY_BYTES: usize = 512 * 1024;
const DOWNLOAD_PROGRESS_REPORT_INTERVAL_BYTES: usize = 64 * 1024;
const MIN_PLAYBACK_RATE: f32 = 0.25;
const MAX_PLAYBACK_RATE: f32 = 4.0;
const NORMAL_PLAYBACK_RATE: f32 = 1.0;
const AUDIO_FOCUS_DUCK_FACTOR: f32 = 0.2;
const METRICS_REPORT_INTERVAL: Duration = Duration::from_millis(500);
const BUFFER_LEVEL_REPORT_STEP_MS: u32 = 50;
const SPHERICAL_GESTURE_DEGREES_PER_POINT: f32 = 0.2;
const SPHERICAL_VIDEO_RENDER_SHADER_WGSL: &str =
    include_str!("shaders/spherical_video_render.wgsl");

type OnEvent = Option<Rc<dyn Fn(Event) + 'static>>;

/// Bridges an optional typed [`BoxedEventAction<Event>`] to the runtime player's
/// optional shared callback by capturing the rendering [`Environment`] at hook
/// time.
///
/// The runtime player and its many sub-tasks share the callback via `Rc`
/// clones, which requires the inner closure to be `Fn(Event)`. The
/// user-facing API still takes a typed `EventHandler` (`FnMut(Event,
/// &Environment)`) so that handlers can extract `State<T>` / environment
/// values; the captured `env` here is what makes that extraction work
/// even though the runtime invocation surface is itself `Fn(Event)`.
fn bind_event_handler_to_env(handler: Option<VideoEventHandler>, env: Environment) -> OnEvent {
    handler.map(|handler| handler.bind_callback(env))
}

fn emit_event(handler: &OnEvent, event: Event) {
    if let Some(handler) = handler {
        handler(event);
    }
}

fn u32_to_f32(value: u32, name: &str) -> f32 {
    value
        .to_f32()
        .unwrap_or_else(|| panic!("{name} must fit into f32"))
}

fn usize_to_f64(value: usize, name: &str) -> f64 {
    value
        .to_f64()
        .unwrap_or_else(|| panic!("{name} must fit into f64"))
}

fn u64_to_usize(value: u64, name: &str) -> usize {
    usize::try_from(value).unwrap_or_else(|_| panic!("{name} must fit into usize"))
}

fn usize_to_u64(value: usize, name: &str) -> u64 {
    u64::try_from(value).unwrap_or_else(|_| panic!("{name} must fit into u64"))
}

fn f64_to_f32(value: f64, name: &str) -> f32 {
    value
        .to_f32()
        .unwrap_or_else(|| panic!("{name} must fit into f32"))
}

fn f64_to_u64(value: f64, name: &str) -> u64 {
    value
        .to_u64()
        .unwrap_or_else(|| panic!("{name} must fit into u64"))
}

fn downloaded_len(path: &Path) -> usize {
    fs::metadata(path)
        .ok()
        .map_or(0, |meta| u64_to_usize(meta.len(), "downloaded file length"))
}

fn shader_target_mode(format: wgpu::TextureFormat, source_is_hdr: bool) -> ColorOutputTarget {
    if matches!(
        format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    ) {
        if source_is_hdr {
            ColorOutputTarget::LinearHdr
        } else {
            ColorOutputTarget::LinearSdr
        }
    } else if format.is_srgb() {
        ColorOutputTarget::LinearSdr
    } else {
        ColorOutputTarget::GammaSdr
    }
}

fn playback_clock_position(
    audio_position: Option<Duration>,
    anchor_pts: Duration,
    anchor_elapsed: Option<Duration>,
    playback_rate: f32,
) -> Duration {
    if let Some(position) = audio_position.filter(|position| *position > Duration::ZERO) {
        return position;
    }

    anchor_elapsed.map_or(anchor_pts, |elapsed| {
        anchor_pts.saturating_add(elapsed.mul_f64(f64::from(playback_rate)))
    })
}

const fn clamp_playback_rate(rate: f32) -> f32 {
    if rate.is_finite() {
        rate.clamp(MIN_PLAYBACK_RATE, MAX_PLAYBACK_RATE)
    } else {
        1.0
    }
}

fn select_live_catch_up_rate(
    current_rate: f32,
    position: Duration,
    target: Duration,
    tolerance: Duration,
    minimum_rate: f32,
    maximum_rate: f32,
) -> f32 {
    let behind = target.saturating_sub(position);
    let ahead = position.saturating_sub(target);
    let release_tolerance = tolerance / 2;
    if current_rate > NORMAL_PLAYBACK_RATE {
        if behind > release_tolerance {
            maximum_rate
        } else {
            NORMAL_PLAYBACK_RATE
        }
    } else if current_rate < NORMAL_PLAYBACK_RATE {
        if ahead > release_tolerance {
            minimum_rate
        } else {
            NORMAL_PLAYBACK_RATE
        }
    } else if behind > tolerance {
        maximum_rate
    } else if ahead > tolerance {
        minimum_rate
    } else {
        NORMAL_PLAYBACK_RATE
    }
}

fn effective_audio_volume(requested: Volume, muted: bool, audio_focus_ducked: bool) -> f32 {
    let base_volume = if muted { 0.0 } else { requested.level() };
    if audio_focus_ducked {
        base_volume * AUDIO_FOCUS_DUCK_FACTOR
    } else {
        base_volume
    }
}

const fn should_wait_for_vod_buffering(
    policy: PlaybackPolicy,
    source_downloading: bool,
    has_download_total: bool,
    first_frame_presented: bool,
    buffered_ahead_ms: u32,
) -> bool {
    if policy.realtime || !source_downloading || !has_download_total {
        return false;
    }

    let threshold_ms = if first_frame_presented {
        policy.vod_resume_buffer_ms
    } else {
        policy.vod_start_buffer_ms
    };
    buffered_ahead_ms < threshold_ms
}

const fn should_enter_vod_stall_buffering(
    policy: PlaybackPolicy,
    source_downloading: bool,
    has_download_total: bool,
    first_frame_presented: bool,
    buffered_ahead_ms: u32,
) -> bool {
    if policy.realtime || !source_downloading || !has_download_total || !first_frame_presented {
        return false;
    }

    buffered_ahead_ms <= policy.vod_stall_buffer_ms
}

#[derive(Clone)]
struct PlayerBindings {
    is_playing: Binding<bool>,
    progress_display: Binding<f64>,
    seek_target_seconds: Binding<f64>,
    seek_generation: Binding<u64>,
    picture_in_picture_request: Binding<u64>,
    duration_seconds: Binding<f64>,
    position_seconds: Binding<f64>,
    live_window: Binding<Option<LiveWindow>>,
    is_buffering: Binding<bool>,
    playback_rate: Binding<f32>,
    preserve_pitch: Binding<bool>,
    audio_track_labels: Binding<Vec<String>>,
    video_track_labels: Binding<Vec<String>>,
}

#[derive(Clone)]
struct PlaybackUiBindings {
    desired_playing: Binding<bool>,
    seek_target_seconds: Binding<f64>,
    seek_generation: Binding<u64>,
    step_forward_generation: Binding<u64>,
    step_backward_generation: Binding<u64>,
    duration_seconds: Binding<f64>,
    position_seconds: Binding<f64>,
    live_window: Binding<Option<LiveWindow>>,
    phase: Binding<PlaybackPhase>,
    repeat: Binding<RepeatMode>,
}

#[derive(Clone)]
struct PlayerControlBindings {
    controller: PlayerController,
    audio_track_labels: Binding<Vec<String>>,
    audio_track_selection: Binding<AudioTrackSelection>,
    video_track_labels: Binding<Vec<String>>,
    video_track_selection: Binding<VideoTrackSelection>,
    subtitle_track_labels: Binding<Vec<String>>,
    subtitle_selection: Binding<SubtitleSelection>,
    has_previous: Binding<bool>,
    has_next: Binding<bool>,
    is_playing: Binding<bool>,
    progress: Binding<f64>,
    picture_in_picture_request: Binding<u64>,
    duration_seconds: Binding<f64>,
    position_seconds: Binding<f64>,
    is_buffering: Binding<bool>,
    playback_rate: Binding<f32>,
    preserve_pitch: Binding<bool>,
    volume: Binding<Volume>,
    muted: Binding<bool>,
}

struct TransportBindings {
    controller: PlayerController,
    has_previous: Binding<bool>,
    has_next: Binding<bool>,
    is_playing: Binding<bool>,
    progress: Binding<f64>,
    picture_in_picture_request: Binding<u64>,
    duration_seconds: Binding<f64>,
    muted: Binding<bool>,
    volume_level: Binding<f64>,
    audio_toggle: AnyView,
    video_toggle: AnyView,
    subtitle_toggle: AnyView,
    speed_controls: AnyView,
}

impl PlayerBindings {
    fn progress_control_binding(&self) -> Binding<f64> {
        let seek_target_seconds = self.seek_target_seconds.clone();
        let seek_generation = self.seek_generation.clone();
        let duration_seconds = self.duration_seconds.clone();
        let live_window = self.live_window.clone();
        Binding::mapping(
            &self.progress_display,
            |current: f64| current,
            move |display, requested: f64| {
                let clamped = requested.clamp(0.0, 1.0);
                display.set(clamped);
                let target = live_window.get().map_or_else(
                    || duration_seconds.get().max(0.0) * clamped,
                    |window| {
                        let start = window.seekable_start().as_secs_f64();
                        let span = window
                            .seekable_end()
                            .saturating_sub(window.seekable_start())
                            .as_secs_f64();
                        span.mul_add(clamped, start)
                    },
                );
                seek_target_seconds.set(target);
                seek_generation.set(seek_generation.get().wrapping_add(1));
            },
        )
    }
}

#[derive(Clone)]
struct SubtitleBindings {
    text: Binding<String>,
    track_labels: Binding<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeSubtitleTrack {
    label: String,
    source: RuntimeSubtitleSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeMediaItem {
    source: Url,
    delivery: Delivery,
    drm: Option<DrmConfiguration>,
    subtitle_tracks: Vec<RuntimeSubtitleTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeSubtitleSource {
    Sidecar(SubtitleTrack),
    Embedded(EmbeddedSubtitleSourceTrack),
    Manifest(SelectableSubtitleTrack),
}

#[derive(Debug)]
enum UiUpdate {
    Event(Event),
    SeekRequest(Duration),
    Duration(f64),
    Buffering(bool),
    Playing(bool),
    AudioTracks(Vec<AudioTrackInfo>),
    VideoTracks(Vec<VideoTrackInfo>),
    Subtitle(String),
    SubtitleTracks(Vec<SubtitleTrackInfo>),
    LiveWindow(Option<LiveWindow>),
}

struct UiUpdatePort {
    updates: AsyncSender<UiUpdate>,
    progress: LatestSender<f64>,
    position: LatestSender<f64>,
}

struct UiUpdateReceivers {
    updates: AsyncReceiver<UiUpdate>,
    progress: AsyncReceiver<f64>,
    position: AsyncReceiver<f64>,
}

fn ui_update_channel() -> (UiUpdatePort, UiUpdateReceivers) {
    let (updates, update_receiver) = async_channel::unbounded();
    let (progress, progress_receiver) = latest_channel();
    let (position, position_receiver) = latest_channel();
    (
        UiUpdatePort {
            updates,
            progress,
            position,
        },
        UiUpdateReceivers {
            updates: update_receiver,
            progress: progress_receiver,
            position: position_receiver,
        },
    )
}

fn apply_ui_update(
    on_event: &OnEvent,
    playback: &PlaybackUiBindings,
    track_catalog: &Binding<TrackCatalog>,
    player: Option<&PlayerBindings>,
    subtitle: Option<&SubtitleBindings>,
    update: UiUpdate,
) {
    let Some(update) = apply_track_catalog_update(track_catalog, player, subtitle, update) else {
        return;
    };
    match update {
        UiUpdate::Event(event) => {
            let phase = match &event {
                Event::ReadyToPlay => Some(if playback.desired_playing.get() {
                    PlaybackPhase::Playing
                } else {
                    PlaybackPhase::Ready
                }),
                Event::Ended => Some(PlaybackPhase::Ended),
                Event::Error { .. } => Some(PlaybackPhase::Failed),
                Event::Buffering => Some(PlaybackPhase::Buffering),
                Event::BufferingEnded => Some(if playback.desired_playing.get() {
                    PlaybackPhase::Playing
                } else {
                    PlaybackPhase::Paused
                }),
                Event::PlaybackStateChanged { playing } => Some(if *playing {
                    PlaybackPhase::Playing
                } else {
                    PlaybackPhase::Paused
                }),
                _ => None,
            };
            if let Some(phase) = phase {
                playback.phase.set(phase);
            }
            emit_event(on_event, event);
        }
        UiUpdate::SeekRequest(position) => {
            let seconds = position.as_secs_f64();
            playback.seek_target_seconds.set(seconds);
            playback.position_seconds.set(seconds);
            if let Some(player) = player {
                player.seek_target_seconds.set(seconds);
            }
        }
        UiUpdate::Duration(value) => {
            playback.duration_seconds.set(value);
            if let Some(player) = player {
                player.duration_seconds.set(value);
            }
        }
        UiUpdate::Buffering(value) => {
            if value {
                playback.phase.set(PlaybackPhase::Buffering);
            }
            if let Some(player) = player {
                player.is_buffering.set(value);
            }
        }
        UiUpdate::Playing(value) => {
            playback.desired_playing.set(value);
            playback.phase.set(if value {
                PlaybackPhase::Playing
            } else {
                PlaybackPhase::Paused
            });
            if let Some(player) = player {
                player.is_playing.set(value);
            }
        }
        UiUpdate::Subtitle(value) => {
            if let Some(subtitle) = subtitle {
                subtitle.text.set(value);
            }
        }
        UiUpdate::LiveWindow(window) => {
            playback.live_window.set(window);
            if let Some(player) = player {
                player.live_window.set(window);
            }
        }
        UiUpdate::AudioTracks(_) | UiUpdate::VideoTracks(_) | UiUpdate::SubtitleTracks(_) => {
            unreachable!("track-catalog update must be consumed before UI-state dispatch")
        }
    }
}

fn apply_track_catalog_update(
    track_catalog: &Binding<TrackCatalog>,
    player: Option<&PlayerBindings>,
    subtitle: Option<&SubtitleBindings>,
    update: UiUpdate,
) -> Option<UiUpdate> {
    match update {
        UiUpdate::AudioTracks(value) => {
            track_catalog.set(track_catalog.get().replacing_audio(value.clone()));
            if let Some(player) = player {
                player
                    .audio_track_labels
                    .set(audio_track_info_labels(&value));
            }
            None
        }
        UiUpdate::VideoTracks(value) => {
            track_catalog.set(track_catalog.get().replacing_video(value.clone()));
            if let Some(player) = player {
                player
                    .video_track_labels
                    .set(video_track_info_labels(&value));
            }
            None
        }
        UiUpdate::SubtitleTracks(value) => {
            track_catalog.set(track_catalog.get().replacing_subtitles(value.clone()));
            if let Some(subtitle) = subtitle {
                subtitle
                    .track_labels
                    .set(subtitle_track_info_labels(&value));
            }
            None
        }
        update => Some(update),
    }
}

fn start_ui_update_pump(
    receivers: UiUpdateReceivers,
    on_event: OnEvent,
    playback: PlaybackUiBindings,
    track_catalog: Binding<TrackCatalog>,
    player: Option<PlayerBindings>,
    subtitle: Option<SubtitleBindings>,
) {
    let progress_player = player.clone();
    spawn_local(async move {
        while let Ok(progress) = receivers.progress.recv().await {
            if let Some(player) = progress_player.as_ref() {
                player.progress_display.set(progress);
            }
        }
    })
    .detach();

    let position_playback = playback.clone();
    let position_player = player.clone();
    spawn_local(async move {
        while let Ok(position) = receivers.position.recv().await {
            position_playback.position_seconds.set(position);
            if let Some(player) = position_player.as_ref() {
                player.position_seconds.set(position);
            }
        }
    })
    .detach();

    spawn_local(async move {
        while let Ok(update) = receivers.updates.recv().await {
            apply_ui_update(
                &on_event,
                &playback,
                &track_catalog,
                player.as_ref(),
                subtitle.as_ref(),
                update,
            );
        }
    })
    .detach();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VertexLayoutKey {
    surface_width: u32,
    surface_height: u32,
    video_width: u32,
    video_height: u32,
    aspect_ratio: AspectRatio,
}

#[derive(Debug)]
enum FileAssetState {
    Unresolved,
    Downloading {
        path: PathBuf,
        receiver: Receiver<DownloadUpdate>,
        ready: bool,
    },
    Ready(PathBuf),
    Failed(String),
}

#[derive(Debug)]
enum DownloadUpdate {
    Ready,
    Progress {
        bytes_written: usize,
        total_bytes: Option<usize>,
    },
    Finished(PathBuf),
    Failed(String),
}

pub fn install(env: &mut Environment, options: VideoGpuOptions) {
    let video_options = options.clone();
    env.insert_hook::<VideoConfig, AnyView>(move |env, config| {
        video_hook(env, config, &video_options)
    });
    env.insert_hook::<VideoPlayerConfig, AnyView>(move |env, config| {
        video_player_hook(env, config, &options)
    });
}

fn prepare_runtime_source(
    source: &Computed<MediaItem>,
    track_catalog: &Binding<TrackCatalog>,
) -> (Computed<RuntimeMediaItem>, SubtitleBindings) {
    let source = runtime_media_item_signal(source);
    let item = waterui_core::Signal::get(&source);
    let subtitle = initial_subtitle_bindings(&item, track_catalog);
    (source, subtitle)
}

struct PreparedPlayback {
    controller: PlayerController,
    source: Computed<RuntimeMediaItem>,
    subtitle: SubtitleBindings,
    subtitle_selection: Binding<SubtitleSelection>,
    audio_track_selection: Binding<AudioTrackSelection>,
    video_track_selection: Binding<VideoTrackSelection>,
    track_catalog: Binding<TrackCatalog>,
    has_next: Binding<bool>,
    has_previous: Binding<bool>,
    volume: Binding<Volume>,
    muted: Binding<bool>,
    playback_rate: Binding<f32>,
    preserve_pitch: Binding<bool>,
    playback_ui: PlaybackUiBindings,
    playback_policy: PlaybackPolicy,
    on_event: OnEvent,
}

fn prepare_playback(
    env: &Environment,
    playback: PlaybackConfiguration<Option<VideoEventHandler>>,
) -> PreparedPlayback {
    let PlaybackConfiguration {
        controller,
        source,
        subtitle_selection,
        audio_track_selection,
        video_track_selection,
        track_catalog,
        live_window,
        has_next,
        has_previous,
        volume,
        muted,
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
        playback_policy,
        on_event,
    } = playback;
    let on_event = bind_event_handler_to_env(on_event, env.clone());
    let (source, subtitle) = prepare_runtime_source(&source, &track_catalog);
    PreparedPlayback {
        controller,
        source,
        subtitle,
        subtitle_selection,
        audio_track_selection,
        video_track_selection,
        track_catalog,
        has_next,
        has_previous,
        volume,
        muted,
        playback_rate,
        preserve_pitch,
        playback_ui: PlaybackUiBindings {
            desired_playing,
            seek_target_seconds,
            seek_generation,
            step_forward_generation,
            step_backward_generation,
            duration_seconds,
            position_seconds,
            live_window,
            phase,
            repeat,
        },
        playback_policy,
        on_event,
    }
}

fn video_hook(env: &Environment, config: VideoConfig, options: &VideoGpuOptions) -> AnyView {
    let VideoConfig {
        playback,
        aspect_ratio,
        projection,
        loops,
    } = config;
    let PreparedPlayback {
        controller,
        source,
        subtitle,
        subtitle_selection,
        audio_track_selection,
        video_track_selection,
        track_catalog,
        has_next,
        has_previous,
        volume,
        muted,
        playback_rate,
        preserve_pitch,
        playback_ui,
        playback_policy,
        on_event,
    } = prepare_playback(env, playback);
    let (ui_updates, ui_receivers) = ui_update_channel();
    start_ui_update_pump(
        ui_receivers,
        on_event,
        playback_ui.clone(),
        track_catalog,
        None,
        Some(subtitle.clone()),
    );
    let surface = VideoSurface::new(VideoSurfaceConfig {
        controller,
        playback: playback_ui,
        source,
        subtitle_selection,
        audio_track_selection,
        video_track_selection,
        has_next,
        has_previous,
        volume,
        muted,
        playback_rate,
        preserve_pitch,
        aspect_ratio,
        projection,
        loops,
        playback_policy,
        audio_output: options.audio_output.clone(),
        skip_silence: options.skip_silence.clone(),
        #[cfg(target_os = "android")]
        license_server: options.license_server.clone(),
        ui_updates,
        player: None,
    });
    AnyView::new(overlay(surface, subtitle_banner(&subtitle.text)).alignment(Alignment::Bottom))
}

fn video_player_hook(
    env: &Environment,
    config: VideoPlayerConfig,
    options: &VideoGpuOptions,
) -> AnyView {
    let VideoPlayerConfig {
        playback,
        aspect_ratio,
        projection,
        show_controls,
    } = config;
    let PreparedPlayback {
        controller,
        source,
        subtitle,
        subtitle_selection,
        audio_track_selection,
        video_track_selection,
        track_catalog,
        has_next,
        has_previous,
        volume,
        muted,
        playback_rate,
        preserve_pitch,
        playback_ui,
        playback_policy,
        on_event,
    } = prepare_playback(env, playback);
    let player = PlayerBindings {
        is_playing: playback_ui.desired_playing.clone(),
        progress_display: Binding::f64(0.0),
        seek_target_seconds: playback_ui.seek_target_seconds.clone(),
        seek_generation: playback_ui.seek_generation.clone(),
        picture_in_picture_request: binding(0_u64),
        duration_seconds: playback_ui.duration_seconds.clone(),
        position_seconds: playback_ui.position_seconds.clone(),
        live_window: playback_ui.live_window.clone(),
        is_buffering: Binding::bool(false),
        playback_rate: playback_rate.clone(),
        preserve_pitch: preserve_pitch.clone(),
        audio_track_labels: binding(Vec::new()),
        video_track_labels: binding(Vec::new()),
    };
    let (ui_updates, ui_receivers) = ui_update_channel();
    start_ui_update_pump(
        ui_receivers,
        on_event.clone(),
        playback_ui.clone(),
        track_catalog,
        Some(player.clone()),
        Some(subtitle.clone()),
    );
    video_player_chrome(
        VideoSurfaceConfig {
            controller,
            playback: playback_ui,
            source,
            subtitle_selection,
            audio_track_selection,
            video_track_selection,
            has_next,
            has_previous,
            volume,
            muted,
            playback_rate,
            preserve_pitch,
            aspect_ratio,
            projection,
            loops: false,
            playback_policy,
            audio_output: options.audio_output.clone(),
            skip_silence: options.skip_silence.clone(),
            #[cfg(target_os = "android")]
            license_server: options.license_server.clone(),
            ui_updates,
            player: Some(player.clone()),
        },
        &player,
        &subtitle,
        show_controls,
        on_event,
    )
}

fn initial_subtitle_bindings(
    item: &RuntimeMediaItem,
    track_catalog: &Binding<TrackCatalog>,
) -> SubtitleBindings {
    let tracks = runtime_subtitle_track_info(&item.subtitle_tracks);
    track_catalog.set(track_catalog.get().replacing_subtitles(tracks.clone()));
    SubtitleBindings {
        text: binding(String::new()),
        track_labels: binding(subtitle_track_info_labels(&tracks)),
    }
}

fn video_player_chrome(
    surface_config: VideoSurfaceConfig,
    player: &PlayerBindings,
    subtitle: &SubtitleBindings,
    show_controls: bool,
    on_event: OnEvent,
) -> AnyView {
    let projection = surface_config.projection.clone();
    let controls = show_controls.then(|| {
        let progress = player.progress_control_binding();
        player_controls(
            PlayerControlBindings {
                controller: surface_config.controller.clone(),
                audio_track_labels: player.audio_track_labels.clone(),
                audio_track_selection: surface_config.audio_track_selection.clone(),
                video_track_labels: player.video_track_labels.clone(),
                video_track_selection: surface_config.video_track_selection.clone(),
                subtitle_track_labels: subtitle.track_labels.clone(),
                subtitle_selection: surface_config.subtitle_selection.clone(),
                has_previous: surface_config.has_previous.clone(),
                has_next: surface_config.has_next.clone(),
                is_playing: player.is_playing.clone(),
                progress,
                picture_in_picture_request: player.picture_in_picture_request.clone(),
                duration_seconds: player.duration_seconds.clone(),
                position_seconds: player.position_seconds.clone(),
                is_buffering: player.is_buffering.clone(),
                playback_rate: player.playback_rate.clone(),
                preserve_pitch: player.preserve_pitch.clone(),
                volume: surface_config.volume.clone(),
                muted: surface_config.muted.clone(),
            },
            on_event,
        )
    });
    let surface = spherical_interaction(VideoSurface::new(surface_config), &projection);
    if let Some(controls) = controls {
        let bottom_cluster = vstack((subtitle_banner(&subtitle.text), controls)).spacing(12.0);
        AnyView::new(overlay(surface, bottom_cluster).alignment(Alignment::Bottom))
    } else {
        AnyView::new(overlay(surface, subtitle_banner(&subtitle.text)).alignment(Alignment::Bottom))
    }
}

fn subtitle_track_label(track: &SubtitleTrack, index: usize) -> String {
    track
        .label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            track.language.as_deref().and_then(|language| {
                let trimmed = language.trim();
                (!trimmed.is_empty()).then_some(trimmed.to_owned())
            })
        })
        .unwrap_or_else(|| format!("Track {}", index + 1))
}

fn runtime_sidecar_subtitle_tracks(tracks: &[SubtitleTrack]) -> Vec<RuntimeSubtitleTrack> {
    tracks
        .iter()
        .enumerate()
        .map(|(index, track)| RuntimeSubtitleTrack {
            label: subtitle_track_label(track, index),
            source: RuntimeSubtitleSource::Sidecar(track.clone()),
        })
        .collect()
}

fn embedded_subtitle_track_label(
    track: &EmbeddedSubtitleSourceTrack,
    embedded_index: usize,
) -> String {
    let language = track.language.trim();
    if language.is_empty() || language == "und" {
        format!("Embedded {}", embedded_index + 1)
    } else {
        format!("Embedded {language}")
    }
}

fn runtime_embedded_subtitle_tracks(
    tracks: &[EmbeddedSubtitleSourceTrack],
) -> Vec<RuntimeSubtitleTrack> {
    tracks
        .iter()
        .enumerate()
        .map(|(index, track)| RuntimeSubtitleTrack {
            label: embedded_subtitle_track_label(track, index),
            source: RuntimeSubtitleSource::Embedded(track.clone()),
        })
        .collect()
}

fn runtime_manifest_subtitle_tracks(
    tracks: &[SelectableSubtitleTrack],
) -> Vec<RuntimeSubtitleTrack> {
    tracks
        .iter()
        .map(|track| RuntimeSubtitleTrack {
            label: track.label().to_owned(),
            source: RuntimeSubtitleSource::Manifest(track.clone()),
        })
        .collect()
}

fn runtime_subtitle_track_info(tracks: &[RuntimeSubtitleTrack]) -> Vec<SubtitleTrackInfo> {
    tracks
        .iter()
        .map(|track| match &track.source {
            RuntimeSubtitleSource::Sidecar(sidecar) => SubtitleTrackInfo::new(
                track.label.clone(),
                sidecar.language.clone(),
                sidecar
                    .forced
                    .then(|| String::from("forced-subtitle"))
                    .into_iter()
                    .collect(),
                sidecar.forced,
                SubtitleTrackOrigin::Sidecar,
            ),
            RuntimeSubtitleSource::Embedded(embedded) => {
                let language = embedded.language.trim();
                SubtitleTrackInfo::new(
                    track.label.clone(),
                    (!language.is_empty() && language != "und").then(|| language.to_owned()),
                    Vec::new(),
                    false,
                    SubtitleTrackOrigin::Embedded,
                )
            }
            RuntimeSubtitleSource::Manifest(manifest) => SubtitleTrackInfo::new(
                track.label.clone(),
                manifest.language().map(ToOwned::to_owned),
                manifest.roles().to_vec(),
                manifest.is_forced(),
                SubtitleTrackOrigin::Manifest,
            ),
        })
        .collect()
}

fn selectable_audio_track_info(tracks: &[SelectableAudioTrack]) -> Vec<AudioTrackInfo> {
    tracks
        .iter()
        .map(|track| {
            AudioTrackInfo::new(
                track.label(),
                track.language().map(ToOwned::to_owned),
                track.roles().to_vec(),
            )
        })
        .collect()
}

fn selectable_video_track_info(tracks: &[SelectableVideoTrack]) -> Vec<VideoTrackInfo> {
    tracks
        .iter()
        .map(|track| {
            VideoTrackInfo::new(
                track.id(),
                track.label(),
                Some(track.bandwidth()),
                track.dimensions(),
                track.codecs().to_vec(),
                track.is_hdr(),
            )
        })
        .collect()
}

fn audio_track_info_labels(tracks: &[AudioTrackInfo]) -> Vec<String> {
    tracks
        .iter()
        .map(|track| track.label().to_owned())
        .collect()
}

fn video_track_info_labels(tracks: &[VideoTrackInfo]) -> Vec<String> {
    tracks
        .iter()
        .map(|track| track.label().to_owned())
        .collect()
}

fn subtitle_track_info_labels(tracks: &[SubtitleTrackInfo]) -> Vec<String> {
    tracks
        .iter()
        .map(|track| track.label().to_owned())
        .collect()
}

fn runtime_media_item(item: MediaItem) -> RuntimeMediaItem {
    RuntimeMediaItem {
        source: item.source,
        delivery: item.delivery,
        subtitle_tracks: runtime_sidecar_subtitle_tracks(&item.subtitle_tracks),
        drm: item.drm,
    }
}

fn runtime_media_item_signal(source: &Computed<MediaItem>) -> Computed<RuntimeMediaItem> {
    source.map(runtime_media_item).distinct().computed()
}

fn select_default_subtitle_track_index(tracks: &[RuntimeSubtitleTrack]) -> Option<usize> {
    tracks
        .iter()
        .position(
            |track| matches!(&track.source, RuntimeSubtitleSource::Sidecar(sidecar) if sidecar.forced),
        )
        .or_else(|| {
            tracks
                .iter()
                .position(|track| matches!(track.source, RuntimeSubtitleSource::Sidecar(_)))
        })
        .or_else(|| {
            tracks.iter().position(
                |track| matches!(&track.source, RuntimeSubtitleSource::Manifest(manifest) if manifest.is_forced()),
            )
        })
        .or_else(|| (!tracks.is_empty()).then_some(0))
}

const fn segmented_subtitle_track_selection(
    sidecar_track_count: usize,
    selection: SubtitleSelection,
) -> EngineSubtitleTrackSelection {
    match selection {
        SubtitleSelection::Auto | SubtitleSelection::Off if sidecar_track_count > 0 => {
            EngineSubtitleTrackSelection::Off
        }
        SubtitleSelection::Auto => EngineSubtitleTrackSelection::Auto,
        SubtitleSelection::Off => EngineSubtitleTrackSelection::Off,
        SubtitleSelection::Track(index) if index < sidecar_track_count => {
            EngineSubtitleTrackSelection::Off
        }
        SubtitleSelection::Track(index) => {
            EngineSubtitleTrackSelection::Track(index - sidecar_track_count)
        }
    }
}

fn resolve_selected_subtitle_index(
    tracks: &[RuntimeSubtitleTrack],
    selection: SubtitleSelection,
) -> Result<Option<usize>, String> {
    match selection {
        SubtitleSelection::Auto => Ok(select_default_subtitle_track_index(tracks)),
        SubtitleSelection::Off => Ok(None),
        SubtitleSelection::Track(index) => {
            if index < tracks.len() {
                Ok(Some(index))
            } else {
                Err(format!(
                    "subtitle track index {index} is out of range for {} tracks",
                    tracks.len()
                ))
            }
        }
    }
}

fn subtitle_selection_label(
    track_labels: &[String],
    selection: SubtitleSelection,
) -> Result<String, String> {
    match selection {
        SubtitleSelection::Auto => Ok(String::from("CC Auto")),
        SubtitleSelection::Off => Ok(String::from("CC Off")),
        SubtitleSelection::Track(index) => track_labels
            .get(index)
            .map(|label| format!("CC {label}"))
            .ok_or_else(|| {
                format!(
                    "subtitle track index {index} is out of range for {} tracks",
                    track_labels.len()
                )
            }),
    }
}

fn subtitle_accessibility_label(
    track_labels: &[String],
    selection: SubtitleSelection,
) -> Result<String, String> {
    match selection {
        SubtitleSelection::Auto => Ok(String::from("Subtitles automatic")),
        SubtitleSelection::Off => Ok(String::from("Subtitles off")),
        SubtitleSelection::Track(index) => track_labels
            .get(index)
            .map(|label| format!("Subtitles {label}"))
            .ok_or_else(|| {
                format!(
                    "subtitle track index {index} is out of range for {} tracks",
                    track_labels.len()
                )
            }),
    }
}

fn next_subtitle_selection(
    track_labels: &[String],
    selection: SubtitleSelection,
) -> Result<SubtitleSelection, String> {
    if track_labels.is_empty() {
        return Ok(SubtitleSelection::Off);
    }

    Ok(match selection {
        SubtitleSelection::Auto => SubtitleSelection::Off,
        SubtitleSelection::Off => SubtitleSelection::Track(0),
        SubtitleSelection::Track(index) => {
            if index >= track_labels.len() {
                return Err(format!(
                    "subtitle track index {index} is out of range for {} tracks",
                    track_labels.len()
                ));
            }
            if index + 1 < track_labels.len() {
                SubtitleSelection::Track(index + 1)
            } else {
                SubtitleSelection::Auto
            }
        }
    })
}

trait IndexedTrackSelection: Copy {
    fn automatic() -> Self;
    fn track(index: usize) -> Self;
    fn index(self) -> Option<usize>;
}

impl IndexedTrackSelection for AudioTrackSelection {
    fn automatic() -> Self {
        Self::Auto
    }

    fn track(index: usize) -> Self {
        Self::Track(index)
    }

    fn index(self) -> Option<usize> {
        match self {
            Self::Auto => None,
            Self::Track(index) => Some(index),
        }
    }
}

impl IndexedTrackSelection for VideoTrackSelection {
    fn automatic() -> Self {
        Self::Auto
    }

    fn track(index: usize) -> Self {
        Self::Track(index)
    }

    fn index(self) -> Option<usize> {
        match self {
            Self::Auto => None,
            Self::Track(index) => Some(index),
        }
    }
}

fn indexed_selection_label<S: IndexedTrackSelection>(
    track_labels: &[String],
    selection: S,
    automatic_label: &str,
    selected_prefix: &str,
    track_kind: &str,
) -> Result<String, String> {
    selection.index().map_or_else(
        || Ok(String::from(automatic_label)),
        |index| {
            track_labels.get(index).map_or_else(
                || {
                    Err(format!(
                        "{track_kind} track index {index} is out of range for {} tracks",
                        track_labels.len()
                    ))
                },
                |label| Ok(format!("{selected_prefix}{label}")),
            )
        },
    )
}

fn next_indexed_selection<S: IndexedTrackSelection>(
    track_labels: &[String],
    selection: S,
    track_kind: &str,
) -> Result<S, String> {
    if track_labels.is_empty() {
        return Ok(S::automatic());
    }

    Ok(match selection.index() {
        None => S::track(0),
        Some(index) => {
            if index >= track_labels.len() {
                return Err(format!(
                    "{track_kind} track index {index} is out of range for {} tracks",
                    track_labels.len()
                ));
            }
            if index + 1 < track_labels.len() {
                S::track(index + 1)
            } else {
                S::automatic()
            }
        }
    })
}

fn audio_selection_label(
    track_labels: &[String],
    selection: AudioTrackSelection,
) -> Result<String, String> {
    indexed_selection_label(track_labels, selection, "Audio Auto", "Audio ", "audio")
}

fn audio_accessibility_label(
    track_labels: &[String],
    selection: AudioTrackSelection,
) -> Result<String, String> {
    indexed_selection_label(
        track_labels,
        selection,
        "Audio track automatic",
        "Audio track ",
        "audio",
    )
}

fn next_audio_selection(
    track_labels: &[String],
    selection: AudioTrackSelection,
) -> Result<AudioTrackSelection, String> {
    next_indexed_selection(track_labels, selection, "audio")
}

fn video_selection_label(
    track_labels: &[String],
    selection: VideoTrackSelection,
) -> Result<String, String> {
    indexed_selection_label(track_labels, selection, "Quality Auto", "Quality ", "video")
}

fn video_accessibility_label(
    track_labels: &[String],
    selection: VideoTrackSelection,
) -> Result<String, String> {
    indexed_selection_label(
        track_labels,
        selection,
        "Video quality automatic",
        "Video quality ",
        "video",
    )
}

fn next_video_selection(
    track_labels: &[String],
    selection: VideoTrackSelection,
) -> Result<VideoTrackSelection, String> {
    next_indexed_selection(track_labels, selection, "video")
}

fn subtitle_banner(subtitle_text: &Binding<String>) -> impl View + use<> {
    subtitle_text
        .map(|current| {
            (!current.trim().is_empty()).then(|| {
                text(current)
                    .footnote()
                    .color(Color::srgb(255, 255, 255))
                    .background_color(Color::srgb(0, 0, 0))
            })
        })
        .computed()
}

fn picture_in_picture_button(request: &Binding<u64>) -> impl View + use<> {
    #[cfg(any(target_os = "android", target_os = "ios", target_os = "macos"))]
    {
        With::new(
            button("PiP")
                .accessibility_label("Enter picture in picture")
                .action(|State(request): State<Binding<u64>>| {
                    request.set(request.get().wrapping_add(1));
                }),
            State(request.clone()),
        )
    }

    #[cfg(not(any(target_os = "android", target_os = "ios", target_os = "macos")))]
    {
        let _ = request;
    }
}

fn player_controls(bindings: PlayerControlBindings, on_event: OnEvent) -> impl View {
    let PlayerControlBindings {
        controller,
        audio_track_labels,
        audio_track_selection,
        video_track_labels,
        video_track_selection,
        subtitle_track_labels,
        subtitle_selection,
        has_previous,
        has_next,
        is_playing,
        progress,
        picture_in_picture_request,
        duration_seconds,
        position_seconds,
        is_buffering,
        playback_rate,
        preserve_pitch,
        volume,
        muted,
    } = bindings;
    let volume_level = Binding::mapping(
        &volume,
        |current| f64::from(current.level()),
        |volume_binding, level| {
            volume_binding.set(Volume::new(
                f64_to_f32(level, "volume slider level").clamp(0.0, 1.0),
            ));
        },
    );

    let audio_toggle = audio_track_toggle(audio_track_labels, audio_track_selection);
    let subtitle_toggle = subtitle_track_toggle(subtitle_track_labels, subtitle_selection);
    let video_toggle = video_track_toggle(video_track_labels, video_track_selection);

    let speed_controls = speed_controls(&playback_rate, &preserve_pitch);
    let transport = transport_controls(
        TransportBindings {
            controller,
            has_previous,
            has_next,
            is_playing,
            progress: progress.clone(),
            picture_in_picture_request,
            duration_seconds: duration_seconds.clone(),
            muted,
            volume_level,
            audio_toggle: AnyView::new(audio_toggle),
            video_toggle: AnyView::new(video_toggle),
            subtitle_toggle: AnyView::new(subtitle_toggle),
            speed_controls: AnyView::new(speed_controls),
        },
        on_event,
    );

    let timeline = timeline_view(&position_seconds, &duration_seconds, &is_buffering);

    vstack((
        timeline,
        slider("Playback position", &progress).hide_label(),
        transport,
    ))
    .spacing(8.0)
}

fn audio_track_toggle(
    track_labels: Binding<Vec<String>>,
    selection: Binding<AudioTrackSelection>,
) -> impl View {
    let state = track_labels.zip(&selection);
    With::new(
        With::new(
            button(Text::display(state.clone().map(|(labels, selection)| {
                audio_selection_label(&labels, selection).unwrap_or_else(|message| message)
            })))
            .accessibility_label(Text::display(state.map(|(labels, selection)| {
                audio_accessibility_label(&labels, selection).unwrap_or_else(|message| message)
            })))
            .action(
                |State(selection): State<Binding<AudioTrackSelection>>,
                 State(labels): State<Binding<Vec<String>>>| {
                    let next = next_audio_selection(&labels.get(), selection.get())
                        .expect("audio selection state must resolve");
                    selection.set(next);
                },
            ),
            State(selection),
        ),
        State(track_labels),
    )
}

fn video_track_toggle(
    track_labels: Binding<Vec<String>>,
    selection: Binding<VideoTrackSelection>,
) -> impl View {
    let state = track_labels.zip(&selection);
    With::new(
        With::new(
            button(Text::display(state.clone().map(|(labels, selection)| {
                video_selection_label(&labels, selection).unwrap_or_else(|message| message)
            })))
            .accessibility_label(Text::display(state.map(|(labels, selection)| {
                video_accessibility_label(&labels, selection).unwrap_or_else(|message| message)
            })))
            .action(
                |State(selection): State<Binding<VideoTrackSelection>>,
                 State(labels): State<Binding<Vec<String>>>| {
                    let next = next_video_selection(&labels.get(), selection.get())
                        .expect("video selection state must resolve");
                    selection.set(next);
                },
            ),
            State(selection),
        ),
        State(track_labels),
    )
}

fn subtitle_track_toggle(
    track_labels: Binding<Vec<String>>,
    selection: Binding<SubtitleSelection>,
) -> impl View {
    let state = track_labels.zip(&selection);
    With::new(
        With::new(
            button(Text::display(state.clone().map(|(labels, selection)| {
                subtitle_selection_label(&labels, selection).unwrap_or_else(|message| message)
            })))
            .accessibility_label(Text::display(state.map(|(labels, selection)| {
                subtitle_accessibility_label(&labels, selection).unwrap_or_else(|message| message)
            })))
            .action(
                |State(selection): State<Binding<SubtitleSelection>>,
                 State(labels): State<Binding<Vec<String>>>| {
                    let next = next_subtitle_selection(&labels.get(), selection.get())
                        .expect("subtitle selection state must resolve");
                    selection.set(next);
                },
            ),
            State(selection),
        ),
        State(track_labels),
    )
}

fn transport_controls(bindings: TransportBindings, on_event: OnEvent) -> impl View {
    let TransportBindings {
        controller,
        has_previous,
        has_next,
        is_playing,
        progress,
        picture_in_picture_request,
        duration_seconds,
        muted,
        volume_level,
        audio_toggle,
        video_toggle,
        subtitle_toggle,
        speed_controls,
    } = bindings;
    let previous_controller = controller.clone();
    let previous_button = has_previous
        .map({
            let on_event = on_event.clone();
            move |enabled| {
                enabled.then({
                    let on_event = on_event.clone();
                    let controller = previous_controller.clone();
                    move || {
                        button("Previous").action(move || {
                            request_previous(&controller);
                            emit_event(&on_event, Event::PreviousRequested);
                        })
                    }
                })
            }
        })
        .computed();
    let next_controller = controller;
    let next_button = has_next
        .map(move |enabled| {
            enabled.then({
                let on_event = on_event.clone();
                let controller = next_controller.clone();
                move || {
                    button("Next").action(move || {
                        request_next(&controller);
                        emit_event(&on_event, Event::NextRequested);
                    })
                }
            })
        })
        .computed();

    let primary = hstack((
        previous_button,
        seek_button("Back 10", progress.clone(), duration_seconds.clone(), -10.0),
        play_pause_button(is_playing),
        seek_button("Forward 10", progress, duration_seconds, 10.0),
        next_button,
    ))
    .spacing(8.0);

    let volume_controls = hstack((
        mute_button(&muted),
        Frame::new(slider("Volume", &volume_level).hide_label()).width(160.0),
        picture_in_picture_button(&picture_in_picture_request),
    ))
    .spacing(8.0);
    let playback_options =
        hstack((video_toggle, audio_toggle, subtitle_toggle, speed_controls)).spacing(8.0);
    let secondary = vstack((volume_controls, playback_options)).spacing(8.0);

    vstack((primary, secondary)).spacing(8.0)
}

fn request_next(controller: &PlayerController) {
    controller
        .next()
        .expect("enabled next control must resolve a playlist item");
}

fn request_previous(controller: &PlayerController) {
    controller
        .previous()
        .expect("enabled previous control must resolve a playlist item");
}

fn seek_button(
    label: &'static str,
    progress: Binding<f64>,
    duration: Binding<f64>,
    seconds: f64,
) -> impl View {
    let accessibility_label = if seconds.is_sign_negative() {
        "Rewind 10 seconds"
    } else {
        "Forward 10 seconds"
    };
    With::new(
        With::new(
            button(label)
                .accessibility_label(accessibility_label)
                .action(
                    move |State(value): State<Binding<f64>>,
                          State(duration): State<Binding<f64>>| {
                        let duration = duration.get();
                        if duration <= f64::EPSILON {
                            return;
                        }

                        let delta = (seconds.abs() / duration).min(1.0);
                        let requested = if seconds.is_sign_negative() {
                            value.get() - delta
                        } else {
                            value.get() + delta
                        };
                        value.set(requested.clamp(0.0, 1.0));
                    },
                ),
            State(progress),
        ),
        State(duration),
    )
}

fn play_pause_button(is_playing: Binding<bool>) -> impl View {
    With::new(
        button(Text::display(
            is_playing
                .clone()
                .map(|playing| if playing { "Pause" } else { "Play" }),
        ))
        .action(|State(playing): State<Binding<bool>>| playing.set(!playing.get())),
        State(is_playing),
    )
}

fn mute_button(muted: &Binding<bool>) -> impl View + use<> {
    let action_binding = muted.clone();
    button(Text::display(
        muted
            .clone()
            .map(|is_muted| if is_muted { "Unmute" } else { "Mute" }),
    ))
    .action(move || action_binding.set(!action_binding.get()))
}

fn speed_controls(
    playback_rate: &Binding<f32>,
    preserve_pitch: &Binding<bool>,
) -> impl View + use<> {
    let playback_rate_label = playback_rate
        .clone()
        .map(|rate| format!("Playback speed {:.1} times", clamp_playback_rate(rate)));
    let preserve_pitch_label = preserve_pitch.clone().map(|enabled| {
        if enabled {
            "Disable pitch preservation"
        } else {
            "Enable pitch preservation"
        }
    });
    hstack((
        With::new(
            button(Text::display(
                playback_rate
                    .clone()
                    .map(|rate| format!("{:.1}x", clamp_playback_rate(rate))),
            ))
            .accessibility_label(Text::display(playback_rate_label))
            .action(|State(rate): State<Binding<f32>>| {
                let next = match rate.get() {
                    current if current < 0.75 => 1.0,
                    current if current < 1.25 => 1.5,
                    current if current < 1.75 => 2.0,
                    _ => 0.5,
                };
                rate.set(next);
            }),
            State(playback_rate.clone()),
        ),
        With::new(
            button(Text::display(
                preserve_pitch.clone().map(
                    |enabled| {
                        if enabled { "Pitch On" } else { "Pitch Off" }
                    },
                ),
            ))
            .accessibility_label(Text::display(preserve_pitch_label))
            .action(|State(enabled): State<Binding<bool>>| enabled.set(!enabled.get())),
            State(preserve_pitch.clone()),
        ),
    ))
    .spacing(8.0)
}

fn timeline_view(
    position_seconds: &Binding<f64>,
    duration_seconds: &Binding<f64>,
    is_buffering: &Binding<bool>,
) -> impl View + use<> {
    Text::display(
        position_seconds
            .zip(duration_seconds)
            .zip(is_buffering)
            .map(|((position, duration), is_buffering)| {
                let status = if is_buffering { "  (Buffering)" } else { "" };
                format!(
                    "{} / {}{}",
                    format_timestamp(position),
                    format_timestamp(duration.max(0.0)),
                    status
                )
            }),
    )
    .footnote()
}

fn format_timestamp(seconds: f64) -> String {
    let total = f64_to_u64(seconds.max(0.0).round(), "timestamp seconds");
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let secs = total % 60;

    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes:02}:{secs:02}")
    }
}

fn progress_for_position(duration: Duration, position: Duration) -> f64 {
    if duration.is_zero() {
        0.0
    } else {
        (position.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0)
    }
}

fn new_picture_in_picture_host_id() -> PictureInPictureHostId {
    loop {
        let uuid_bytes = *Uuid::new_v4().as_bytes();
        let raw = u64::from_le_bytes(
            uuid_bytes[..8]
                .try_into()
                .expect("UUID v4 byte slice must have eight bytes"),
        );
        if let Some(raw) = std::num::NonZeroU64::new(raw) {
            return PictureInPictureHostId::new(raw);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SphericalProjectionUniform {
    yaw_radians: f32,
    pitch_radians: f32,
    vertical_field_of_view_radians: f32,
    stereo_layout: u32,
    surface_aspect_ratio: f32,
}

impl SphericalProjectionUniform {
    fn read(
        projection: &EquirectangularProjection,
        surface_width: u32,
        surface_height: u32,
    ) -> Self {
        let viewport = projection.viewport();
        Self {
            yaw_radians: viewport.yaw_degrees().to_radians(),
            pitch_radians: viewport.pitch_degrees().to_radians(),
            vertical_field_of_view_radians: viewport.vertical_field_of_view_degrees().to_radians(),
            stereo_layout: match projection.layout() {
                SphericalStereoLayout::Mono => 0,
                SphericalStereoLayout::TopBottom => 1,
                SphericalStereoLayout::LeftRight => 2,
            },
            surface_aspect_ratio: u32_to_f32(surface_width.max(1), "spherical surface width")
                / u32_to_f32(surface_height.max(1), "spherical surface height"),
        }
    }

    fn to_bytes(self) -> [u8; 32] {
        let mut bytes = [0_u8; 32];
        bytes[0..4].copy_from_slice(&self.yaw_radians.to_ne_bytes());
        bytes[4..8].copy_from_slice(&self.pitch_radians.to_ne_bytes());
        bytes[8..12].copy_from_slice(&self.vertical_field_of_view_radians.to_ne_bytes());
        bytes[12..16].copy_from_slice(&self.stereo_layout.to_ne_bytes());
        bytes[16..20].copy_from_slice(&self.surface_aspect_ratio.to_ne_bytes());
        bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SphericalGestureAnchor {
    yaw: f32,
    pitch: f32,
    vertical_field_of_view: f32,
}

impl SphericalGestureAnchor {
    fn read(viewport: &SphericalViewport) -> Self {
        Self {
            yaw: viewport.yaw_degrees(),
            pitch: viewport.pitch_degrees(),
            vertical_field_of_view: viewport.vertical_field_of_view_degrees(),
        }
    }
}

fn spherical_interaction(view: impl View, projection: &VideoProjection) -> AnyView {
    let VideoProjection::Equirectangular(projection) = projection else {
        return AnyView::new(view);
    };
    let viewport = projection.viewport().clone();
    let anchor = binding(SphericalGestureAnchor::read(&viewport));
    let gesture = DragGesture::new(0.0).simultaneously_with(MagnificationGesture::new(1.0));
    AnyView::new(Metadata::new(
        view,
        GestureObserver::new(gesture, move |env: Environment| {
            if let Some(event) = env.get::<DragEvent>() {
                match event.phase {
                    GesturePhase::Started => anchor.set(SphericalGestureAnchor::read(&viewport)),
                    GesturePhase::Updated => {
                        let start = anchor.get();
                        viewport.set_orientation(
                            event
                                .translation
                                .x
                                .mul_add(-SPHERICAL_GESTURE_DEGREES_PER_POINT, start.yaw),
                            event
                                .translation
                                .y
                                .mul_add(-SPHERICAL_GESTURE_DEGREES_PER_POINT, start.pitch)
                                .clamp(-90.0, 90.0),
                        );
                    }
                    GesturePhase::Ended | GesturePhase::Cancelled => {}
                }
            }
            if let Some(event) = env.get::<MagnificationEvent>() {
                match event.phase {
                    GesturePhase::Started => anchor.set(SphericalGestureAnchor::read(&viewport)),
                    GesturePhase::Updated => {
                        let start = anchor.get();
                        viewport.set_vertical_field_of_view_degrees(
                            (start.vertical_field_of_view / event.scale).clamp(30.0, 120.0),
                        );
                    }
                    GesturePhase::Ended | GesturePhase::Cancelled => {}
                }
            }
        }),
    ))
}

struct VideoSurface {
    picture_in_picture_host_id: PictureInPictureHostId,
    renderer: VideoRenderer,
    #[cfg(target_os = "android")]
    android_surface_bridge: AndroidVideoSurfaceBridge,
}

struct VideoSurfaceConfig {
    controller: PlayerController,
    playback: PlaybackUiBindings,
    source: Computed<RuntimeMediaItem>,
    subtitle_selection: Binding<SubtitleSelection>,
    audio_track_selection: Binding<AudioTrackSelection>,
    video_track_selection: Binding<VideoTrackSelection>,
    has_next: Binding<bool>,
    has_previous: Binding<bool>,
    volume: Binding<Volume>,
    muted: Binding<bool>,
    playback_rate: Binding<f32>,
    preserve_pitch: Binding<bool>,
    aspect_ratio: AspectRatio,
    projection: VideoProjection,
    loops: bool,
    playback_policy: PlaybackPolicy,
    audio_output: AudioOutput,
    skip_silence: Binding<bool>,
    #[cfg(target_os = "android")]
    license_server: waterkit_video::AnyLicenseServer,
    ui_updates: UiUpdatePort,
    player: Option<PlayerBindings>,
}

impl VideoSurface {
    fn new(config: VideoSurfaceConfig) -> Self {
        let picture_in_picture_host_id = new_picture_in_picture_host_id();
        #[cfg(target_os = "android")]
        {
            let (android_surface_bridge, video_surface_receiver) = video_surface_channel();
            Self {
                picture_in_picture_host_id,
                renderer: VideoRenderer::new(
                    picture_in_picture_host_id,
                    config,
                    video_surface_receiver,
                ),
                android_surface_bridge,
            }
        }
        #[cfg(not(target_os = "android"))]
        Self {
            picture_in_picture_host_id,
            renderer: VideoRenderer::new(picture_in_picture_host_id, config),
        }
    }
}

impl fmt::Debug for VideoSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoSurface").finish_non_exhaustive()
    }
}

impl View for VideoSurface {
    fn body(self, _env: &Environment) -> impl View {
        let surface = GpuSurface::new(self.renderer)
            .picture_in_picture_host_id(self.picture_in_picture_host_id.get());
        #[cfg(target_os = "android")]
        let surface = AndroidVideoSurfaceHost::new(surface, self.android_surface_bridge);
        IgnorableMetadata::new(
            IgnorableMetadata::new(surface, AccessibilityRole::Image),
            AccessibilityLabel::new("Video content"),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecoderDrain {
    Continue,
    Break,
    Return,
}

#[derive(Debug, Default)]
struct ColorStateFlags {
    profile_initialized: bool,
    uniform_dirty: bool,
}

#[cfg(target_os = "android")]
#[derive(Debug, Clone, Copy)]
struct ProtectedPendingFrame {
    sequence: u64,
    presentation_time: Duration,
}

impl ColorStateFlags {
    const fn initial() -> Self {
        Self {
            profile_initialized: false,
            uniform_dirty: true,
        }
    }
}

#[derive(Debug, Default)]
struct AudioStateFlags {
    resume_after_focus_gain: bool,
    focus_ducked: bool,
}

#[derive(Default)]
struct PlaybackAudioState {
    player: Option<PlaybackAudio>,
    flags: AudioStateFlags,
    last_applied_volume: Option<f32>,
    last_applied_playback_rate: Option<f32>,
    last_applied_preserve_pitch: Option<bool>,
    last_applied_skip_silence: Option<bool>,
}

#[derive(Debug, Default)]
struct PlaybackFlags {
    first_frame_presented: bool,
    ready_sent: bool,
    ended_sent: bool,
    decoder_lifecycle: DecoderLifecycle,
}

#[derive(Debug)]
struct PlaybackObservability {
    source_selected_at: Instant,
    first_frame_at: Option<Instant>,
    active_rebuffer_started_at: Option<Instant>,
    rebuffer_count: u64,
    rebuffer_duration: Duration,
    dropped_video_frames: u64,
    observed_network_throughput: Option<NonZeroU64>,
    bandwidth_estimator: BandwidthEstimator,
    progressive_sample: Option<(usize, Instant)>,
    last_report_at: Option<Instant>,
}

impl PlaybackObservability {
    const fn new(initial_bandwidth: NonZeroU64, now: Instant) -> Self {
        Self {
            source_selected_at: now,
            first_frame_at: None,
            active_rebuffer_started_at: None,
            rebuffer_count: 0,
            rebuffer_duration: Duration::ZERO,
            dropped_video_frames: 0,
            observed_network_throughput: None,
            bandwidth_estimator: BandwidthEstimator::new(initial_bandwidth),
            progressive_sample: None,
            last_report_at: None,
        }
    }

    const fn reset(&mut self, initial_bandwidth: NonZeroU64, now: Instant) {
        *self = Self::new(initial_bandwidth, now);
    }

    const fn begin_progressive_transfer(&mut self, now: Instant) {
        self.progressive_sample = Some((0, now));
    }

    fn record_progressive_transfer(&mut self, bytes_written: usize, now: Instant) {
        let Some((previous_bytes, previous_at)) = self.progressive_sample else {
            self.progressive_sample = Some((bytes_written, now));
            return;
        };
        let transferred = bytes_written.saturating_sub(previous_bytes);
        let elapsed = now.saturating_duration_since(previous_at);
        self.progressive_sample = Some((bytes_written, now));
        let Some(transferred) = u64::try_from(transferred).ok().and_then(NonZeroU64::new) else {
            return;
        };
        if elapsed.is_zero() {
            return;
        }
        self.bandwidth_estimator
            .add_sample(transferred, elapsed)
            .expect("non-zero progressive bandwidth samples must be valid");
        self.observed_network_throughput = Some(self.bandwidth_estimator.estimate());
    }

    const fn record_network_throughput(&mut self, bits_per_second: NonZeroU64) {
        self.observed_network_throughput = Some(bits_per_second);
    }

    const fn record_dropped_video_frame(&mut self) {
        self.dropped_video_frames = self.dropped_video_frames.saturating_add(1);
    }

    const fn record_first_frame(&mut self, now: Instant) {
        if self.first_frame_at.is_none() {
            self.first_frame_at = Some(now);
        }
    }

    fn record_buffering(&mut self, buffering: bool, count_as_rebuffer: bool, now: Instant) {
        if buffering {
            if count_as_rebuffer && self.active_rebuffer_started_at.is_none() {
                self.rebuffer_count = self.rebuffer_count.saturating_add(1);
                self.active_rebuffer_started_at = Some(now);
            }
            return;
        }
        if let Some(started_at) = self.active_rebuffer_started_at.take() {
            self.rebuffer_duration = self
                .rebuffer_duration
                .saturating_add(now.saturating_duration_since(started_at));
        }
    }

    fn should_report(&self, now: Instant) -> bool {
        self.first_frame_at.is_some()
            && self
                .last_report_at
                .is_none_or(|last| now.saturating_duration_since(last) >= METRICS_REPORT_INTERVAL)
    }

    fn snapshot(
        &self,
        now: Instant,
        position: Duration,
        buffered_ahead: Duration,
        av_drift_ms: Option<f32>,
    ) -> PlaybackMetrics {
        let first_frame_at = self
            .first_frame_at
            .expect("metrics snapshots require a presented first frame");
        let active_rebuffer_duration = self
            .active_rebuffer_started_at
            .map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
        let mut metrics = PlaybackMetrics::new(
            position,
            buffered_ahead,
            first_frame_at.saturating_duration_since(self.source_selected_at),
        )
        .dropped_video_frames(self.dropped_video_frames)
        .rebuffering(
            self.rebuffer_count,
            self.rebuffer_duration
                .saturating_add(active_rebuffer_duration),
        );
        if let Some(av_drift_ms) = av_drift_ms {
            metrics = metrics.av_drift_ms(av_drift_ms);
        }
        if let Some(bits_per_second) = self.observed_network_throughput {
            metrics = metrics.observed_network_throughput(bits_per_second);
        }
        metrics
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum DecoderLifecycle {
    #[default]
    Active,
    Exhausted,
    Failed,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum CompletionState {
    #[default]
    Pending,
    Complete,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ErrorReportState {
    #[default]
    Clear,
    Reported,
}

#[derive(Debug, Default)]
struct SourceFlags {
    embedded_subtitle_tracks: CompletionState,
    manifest_subtitle_tracks: CompletionState,
    source_error: ErrorReportState,
    subtitle_error: ErrorReportState,
}

#[derive(Debug)]
struct ControlFlags {
    is_buffering: bool,
    play_requested: bool,
    seek_inflight: bool,
}

impl ControlFlags {
    const fn initial(play_requested: bool) -> Self {
        Self {
            is_buffering: false,
            play_requested,
            seek_inflight: false,
        }
    }
}

struct PictureInPictureCommands {
    stream: PictureInPictureCommandStream,
    poller: Option<RedrawCommandPoller<PictureInPictureCommand>>,
}

struct InitialPlaybackState {
    item: RuntimeMediaItem,
    play_requested: bool,
    playback_rate: f32,
    subtitle_selection: SubtitleSelection,
    audio_track_selection: AudioTrackSelection,
    video_track_selection: VideoTrackSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameStepDirection {
    Forward,
    Backward,
}

struct PresentedFrameHistory {
    entries: VecDeque<Duration>,
    retention: Duration,
}

impl PresentedFrameHistory {
    const fn new(retention: Duration) -> Self {
        Self {
            entries: VecDeque::new(),
            retention,
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn record(&mut self, presentation_time: Duration) {
        match self.entries.back().copied() {
            Some(current) if current == presentation_time => return,
            Some(current) if presentation_time < current => self.entries.clear(),
            Some(_) | None => {}
        }
        self.entries.push_back(presentation_time);
        while self
            .entries
            .front()
            .is_some_and(|oldest| presentation_time.saturating_sub(*oldest) > self.retention)
        {
            self.entries.pop_front();
        }
    }

    fn rewind(&mut self) -> Option<Duration> {
        if self.entries.len() < 2 {
            return None;
        }
        self.entries.pop_back();
        self.entries.back().copied()
    }
}

impl InitialPlaybackState {
    fn read(config: &VideoSurfaceConfig) -> Self {
        Self {
            item: waterui_core::Signal::get(&config.source),
            play_requested: config.playback.desired_playing.get(),
            playback_rate: clamp_playback_rate(config.playback_rate.get()),
            subtitle_selection: config.subtitle_selection.get(),
            audio_track_selection: config.audio_track_selection.get(),
            video_track_selection: config.video_track_selection.get(),
        }
    }
}

impl PictureInPictureCommands {
    fn new(host_id: PictureInPictureHostId) -> Self {
        Self {
            stream: PictureInPictureCommandStream::new(host_id),
            poller: None,
        }
    }
}

struct VideoRenderer {
    picture_in_picture_host_id: PictureInPictureHostId,
    picture_in_picture_controller: PictureInPictureController,
    controller: PlayerController,
    playback: PlaybackUiBindings,
    source_signal: Computed<RuntimeMediaItem>,
    source: Url,
    delivery: Delivery,
    drm: Option<DrmConfiguration>,
    sidecar_subtitle_tracks: Vec<RuntimeSubtitleTrack>,
    subtitle_tracks: Vec<RuntimeSubtitleTrack>,
    subtitle_selection: Binding<SubtitleSelection>,
    last_subtitle_selection: SubtitleSelection,
    audio_track_selection: Binding<AudioTrackSelection>,
    last_audio_track_selection: AudioTrackSelection,
    video_track_selection: Binding<VideoTrackSelection>,
    last_video_track_selection: VideoTrackSelection,
    active_subtitle_track: Option<usize>,
    has_next: Binding<bool>,
    has_previous: Binding<bool>,
    volume: Binding<Volume>,
    muted: Binding<bool>,
    playback_rate: Binding<f32>,
    preserve_pitch: Binding<bool>,
    aspect_ratio: AspectRatio,
    projection: VideoProjection,
    viewport: Option<(u32, u32)>,
    loops: bool,
    playback_policy: PlaybackPolicy,
    audio_output: AudioOutput,
    skip_silence: Binding<bool>,
    #[cfg(target_os = "android")]
    license_server: waterkit_video::AnyLicenseServer,
    ui_updates: UiUpdatePort,
    player: Option<PlayerBindings>,
    decode_worker: Option<DecoderWorker>,
    #[cfg(target_os = "android")]
    video_surface_receiver: AndroidVideoSurfaceReceiver,
    render_pipeline: Option<wgpu::RenderPipeline>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    sampler: Option<wgpu::Sampler>,
    surface_format: Option<wgpu::TextureFormat>,
    color_profile: VideoColorInfo,
    color_uniform_buffer: Option<wgpu::Buffer>,
    spherical_projection_uniform_buffer: Option<wgpu::Buffer>,
    last_spherical_projection_uniform: Option<SphericalProjectionUniform>,
    spherical_projection_watchers: Vec<BoxWatcherGuard>,
    color_flags: ColorStateFlags,
    decoded_gpu_frame: Option<DecodedGpuFrame>,
    frame_uploader: DecodedFrameUploader,
    bind_group: Option<wgpu::BindGroup>,
    vertex_buffer: Option<wgpu::Buffer>,
    vertex_layout_key: Option<VertexLayoutKey>,
    pending_frame: Option<EngineDecodedVideoFrame>,
    #[cfg(target_os = "android")]
    pending_protected_frame: Option<ProtectedPendingFrame>,
    video_dimensions: Option<(u32, u32)>,
    presented_frame_history: PresentedFrameHistory,
    pending_frame_step: Option<FrameStepDirection>,
    pending_forward_steps: u64,
    pending_backward_steps: u64,
    last_step_forward_generation: u64,
    last_step_backward_generation: u64,
    duration: Duration,
    live_window: Option<EngineLiveWindow>,
    live_playback_rate_range: Option<EngineLivePlaybackRateRange>,
    live_catch_up_rate: f32,
    source_path: Option<PathBuf>,
    audio: PlaybackAudioState,
    playback_output_path: PlaybackOutputPath,
    media_session: Option<MediaSessionState>,
    media_command_poller: Option<RedrawCommandPoller<MediaCommand>>,
    picture_in_picture_commands: PictureInPictureCommands,
    redraw_handle: Option<RedrawHandle>,
    playback_flags: PlaybackFlags,
    playback_anchor_pts: Duration,
    playback_anchor_instant: Option<Instant>,
    pending_play_request_sync: Option<bool>,
    control_flags: ControlFlags,
    last_playing_state: bool,
    last_playback_rate: f32,
    last_reported_progress: f64,
    last_handled_picture_in_picture_request: Option<u64>,
    last_picture_in_picture_controller_state: Option<PictureInPictureControllerState>,
    last_picture_in_picture_active: Option<bool>,
    #[cfg(target_os = "android")]
    next_picture_in_picture_status_poll: Instant,
    last_handled_seek_generation: Option<u64>,
    pending_seek_request: Option<Duration>,
    last_seek_restart_at: Option<Instant>,
    source_asset: FileAssetState,
    subtitle_asset: Option<FileAssetState>,
    source_flags: SourceFlags,
    subtitle_cues: Vec<SubtitleCue>,
    timed_metadata: Vec<EngineTimedMetadata>,
    last_subtitle_text: Option<String>,
    download_generation: u64,
    decoder_waiting_for_download: Option<u64>,
    downloaded_bytes: usize,
    download_total_bytes: Option<usize>,
    last_reported_buffer_level_ms: Option<u32>,
    observability: PlaybackObservability,
}

trait PlaybackAudioControl {
    fn play(&self) -> Result<(), String>;
    fn pause(&self) -> Result<(), String>;
    fn set_volume(&self, volume: f32) -> Result<(), String>;
    fn set_playback_rate(&self, rate: f32) -> Result<(), String>;
    fn set_preserve_pitch(&self, preserve_pitch: bool) -> Result<(), String>;
    fn set_skip_silence(&self, enabled: bool) -> Result<(), String>;
    fn finish(&self) -> Result<(), String>;
    fn position(&self) -> Result<Duration, String>;
    fn buffered_duration(&self) -> Result<Duration, String>;
}

impl PlaybackAudioControl for StreamingAudioPlayer {
    fn play(&self) -> Result<(), String> {
        self.play().map_err(|error| error.to_string())
    }

    fn pause(&self) -> Result<(), String> {
        self.pause().map_err(|error| error.to_string())
    }

    fn set_volume(&self, volume: f32) -> Result<(), String> {
        self.set_volume(volume).map_err(|error| error.to_string())
    }

    fn set_playback_rate(&self, rate: f32) -> Result<(), String> {
        self.set_playback_rate(rate)
            .map_err(|error| error.to_string())
    }

    fn set_preserve_pitch(&self, preserve_pitch: bool) -> Result<(), String> {
        self.set_preserve_pitch(preserve_pitch)
            .map_err(|error| error.to_string())
    }

    fn set_skip_silence(&self, enabled: bool) -> Result<(), String> {
        self.set_skip_silence(enabled)
            .map_err(|error| error.to_string())
    }

    fn finish(&self) -> Result<(), String> {
        self.finish().map_err(|error| error.to_string())
    }

    fn position(&self) -> Result<Duration, String> {
        Ok(self.position())
    }

    fn buffered_duration(&self) -> Result<Duration, String> {
        Ok(self.buffered_duration())
    }
}

#[cfg(target_os = "android")]
impl PlaybackAudioControl for AndroidOffloadAudioController {
    fn play(&self) -> Result<(), String> {
        self.play().map_err(|error| error.to_string())
    }

    fn pause(&self) -> Result<(), String> {
        self.pause().map_err(|error| error.to_string())
    }

    fn set_volume(&self, volume: f32) -> Result<(), String> {
        self.set_volume(volume).map_err(|error| error.to_string())
    }

    fn set_playback_rate(&self, rate: f32) -> Result<(), String> {
        if (rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001 {
            Ok(())
        } else {
            Err(format!(
                "required Android audio offload cannot apply playback rate {rate}"
            ))
        }
    }

    fn set_preserve_pitch(&self, _preserve_pitch: bool) -> Result<(), String> {
        Ok(())
    }

    fn set_skip_silence(&self, enabled: bool) -> Result<(), String> {
        if enabled {
            Err(String::from(
                "required Android audio offload cannot enable skip-silence",
            ))
        } else {
            Ok(())
        }
    }

    fn finish(&self) -> Result<(), String> {
        self.finish().map_err(|error| error.to_string())
    }

    fn position(&self) -> Result<Duration, String> {
        self.position().map_err(|error| error.to_string())
    }

    fn buffered_duration(&self) -> Result<Duration, String> {
        self.buffered_duration().map_err(|error| error.to_string())
    }
}

struct PlaybackAudio(Box<dyn PlaybackAudioControl>);

impl PlaybackAudio {
    fn new(player: StreamingAudioPlayer) -> Self {
        Self(Box::new(player))
    }

    #[cfg(target_os = "android")]
    fn new_offloaded(player: AndroidOffloadAudioController) -> Self {
        Self(Box::new(player))
    }

    fn play(&self) -> Result<(), String> {
        self.0.play()
    }

    fn pause(&self) -> Result<(), String> {
        self.0.pause()
    }

    fn stop(&self) {
        if let Err(error) = self.pause() {
            tracing::error!(%error, "failed to stop playback audio output");
        }
    }

    fn set_volume(&self, volume: f32) -> Result<(), String> {
        self.0.set_volume(volume)
    }

    fn set_playback_rate(&self, rate: f32) -> Result<(), String> {
        self.0.set_playback_rate(rate)
    }

    fn set_preserve_pitch(&self, preserve_pitch: bool) -> Result<(), String> {
        self.0.set_preserve_pitch(preserve_pitch)
    }

    fn set_skip_silence(&self, enabled: bool) -> Result<(), String> {
        self.0.set_skip_silence(enabled)
    }

    fn finish(&self) -> Result<(), String> {
        self.0.finish()
    }

    fn position(&self) -> Duration {
        self.0
            .position()
            .expect("playback audio clock query must succeed")
    }

    fn buffered_duration(&self) -> Duration {
        self.0
            .buffered_duration()
            .expect("playback audio buffer query must succeed")
    }
}

impl fmt::Debug for VideoRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoRenderer")
            .field("source", &self.source)
            .field("subtitle_track_count", &self.subtitle_tracks.len())
            .field("active_subtitle_track", &self.active_subtitle_track)
            .field("loops", &self.loops)
            .field("aspect_ratio", &self.aspect_ratio)
            .field("source_asset", &self.source_asset)
            .field("has_decode", &self.decode_worker.is_some())
            .field("has_pending_frame", &self.pending_frame.is_some())
            .finish_non_exhaustive()
    }
}

fn merge_timed_metadata(
    queue: &mut Vec<EngineTimedMetadata>,
    mut incoming: Vec<EngineTimedMetadata>,
) {
    queue.append(&mut incoming);
    queue.sort_by(|left, right| {
        left.presentation_time()
            .cmp(&right.presentation_time())
            .then_with(|| left.scheme_id_uri().cmp(right.scheme_id_uri()))
            .then_with(|| left.value().cmp(right.value()))
            .then_with(|| left.id().cmp(&right.id()))
            .then_with(|| left.message_data().cmp(right.message_data()))
    });
    queue.dedup();
}

fn take_due_timed_metadata(
    queue: &mut Vec<EngineTimedMetadata>,
    position: Duration,
) -> Vec<EngineTimedMetadata> {
    let due = queue.partition_point(|metadata| metadata.presentation_time() <= position);
    queue.drain(..due).collect()
}

impl VideoRenderer {
    #[expect(
        clippy::too_many_lines,
        reason = "renderer construction explicitly initializes every owned playback and GPU state field"
    )]
    fn new(
        picture_in_picture_host_id: PictureInPictureHostId,
        config: VideoSurfaceConfig,
        #[cfg(target_os = "android")] video_surface_receiver: AndroidVideoSurfaceReceiver,
    ) -> Self {
        let initial = InitialPlaybackState::read(&config);
        let VideoSurfaceConfig {
            controller,
            playback,
            source,
            subtitle_selection,
            audio_track_selection,
            video_track_selection,
            has_next,
            has_previous,
            volume,
            muted,
            playback_rate,
            preserve_pitch,
            aspect_ratio,
            projection,
            loops,
            playback_policy,
            audio_output,
            skip_silence,
            #[cfg(target_os = "android")]
            license_server,
            ui_updates,
            player,
        } = config;
        assert!(
            !projection.is_spherical() || initial.item.drm.is_none(),
            "spherical projection cannot sample a platform-protected video surface"
        );
        let initial_step_forward_generation = playback.step_forward_generation.get();
        let initial_step_backward_generation = playback.step_backward_generation.get();

        Self {
            picture_in_picture_host_id,
            picture_in_picture_controller: PictureInPictureController::new(
                picture_in_picture_host_id,
            ),
            controller,
            playback,
            source_signal: source,
            source: initial.item.source,
            delivery: initial.item.delivery,
            drm: initial.item.drm.clone(),
            subtitle_tracks: initial.item.subtitle_tracks.clone(),
            sidecar_subtitle_tracks: initial.item.subtitle_tracks,
            subtitle_selection,
            last_subtitle_selection: initial.subtitle_selection,
            audio_track_selection,
            last_audio_track_selection: initial.audio_track_selection,
            video_track_selection,
            last_video_track_selection: initial.video_track_selection,
            active_subtitle_track: None,
            has_next,
            has_previous,
            volume,
            muted,
            playback_rate,
            preserve_pitch,
            aspect_ratio,
            projection,
            viewport: None,
            loops,
            playback_policy,
            audio_output,
            skip_silence,
            #[cfg(target_os = "android")]
            license_server,
            ui_updates,
            player,
            decode_worker: None,
            #[cfg(target_os = "android")]
            video_surface_receiver,
            render_pipeline: None,
            bind_group_layout: None,
            sampler: None,
            surface_format: None,
            color_profile: VideoColorInfo::default(),
            color_uniform_buffer: None,
            spherical_projection_uniform_buffer: None,
            last_spherical_projection_uniform: None,
            spherical_projection_watchers: Vec::new(),
            color_flags: ColorStateFlags::initial(),
            decoded_gpu_frame: None,
            frame_uploader: DecodedFrameUploader::new(),
            bind_group: None,
            vertex_buffer: None,
            vertex_layout_key: None,
            pending_frame: None,
            #[cfg(target_os = "android")]
            pending_protected_frame: None,
            video_dimensions: None,
            presented_frame_history: PresentedFrameHistory::new(
                playback_policy.network.maximum_prefetch_buffer(),
            ),
            pending_frame_step: None,
            pending_forward_steps: 0,
            pending_backward_steps: 0,
            last_step_forward_generation: initial_step_forward_generation,
            last_step_backward_generation: initial_step_backward_generation,
            duration: Duration::ZERO,
            live_window: None,
            live_playback_rate_range: None,
            live_catch_up_rate: NORMAL_PLAYBACK_RATE,
            source_path: None,
            audio: PlaybackAudioState::default(),
            playback_output_path: PlaybackOutputPath::DecodedPcmGpu,
            media_session: None,
            media_command_poller: None,
            picture_in_picture_commands: PictureInPictureCommands::new(picture_in_picture_host_id),
            redraw_handle: None,
            playback_flags: PlaybackFlags::default(),
            playback_anchor_pts: Duration::ZERO,
            playback_anchor_instant: None,
            pending_play_request_sync: None,
            control_flags: ControlFlags::initial(initial.play_requested),
            last_playing_state: initial.play_requested,
            last_playback_rate: initial.playback_rate,
            last_reported_progress: 0.0,
            last_handled_picture_in_picture_request: Some(0),
            last_picture_in_picture_controller_state: None,
            last_picture_in_picture_active: Some(false),
            #[cfg(target_os = "android")]
            next_picture_in_picture_status_poll: Instant::now(),
            last_handled_seek_generation: None,
            pending_seek_request: None,
            last_seek_restart_at: None,
            source_asset: FileAssetState::Unresolved,
            subtitle_asset: None,
            source_flags: SourceFlags::default(),
            subtitle_cues: Vec::new(),
            timed_metadata: Vec::new(),
            last_subtitle_text: None,
            download_generation: 0,
            decoder_waiting_for_download: None,
            downloaded_bytes: 0,
            download_total_bytes: None,
            last_reported_buffer_level_ms: None,
            observability: PlaybackObservability::new(
                playback_policy.network.initial_bandwidth(),
                Instant::now(),
            ),
        }
    }

    fn push_ui_update(&self, update: UiUpdate) {
        let _ = self.ui_updates.updates.try_send(update);
    }

    fn push_progress_update(&self, progress: f64) {
        let _ = self.ui_updates.progress.send(progress);
    }

    fn push_position_update(&self, position: f64) {
        let _ = self.ui_updates.position.send(position);
    }

    fn ensure_media_command_poller(&mut self) {
        if self.media_command_poller.is_some() {
            return;
        }
        let Some(redraw_handle) = self.redraw_handle.clone() else {
            return;
        };
        let Some(command_receiver) = self
            .media_session
            .as_ref()
            .map(MediaSessionState::command_receiver)
        else {
            return;
        };
        self.media_command_poller =
            Some(RedrawCommandPoller::spawn(command_receiver, redraw_handle));
    }

    fn ensure_picture_in_picture_command_poller(&mut self) {
        if self.picture_in_picture_commands.poller.is_some() {
            return;
        }
        let Some(redraw_handle) = self.redraw_handle.clone() else {
            return;
        };
        self.picture_in_picture_commands.poller = Some(RedrawCommandPoller::spawn(
            self.picture_in_picture_commands.stream.receiver(),
            redraw_handle,
        ));
    }

    fn current_color_uniform(&self) -> VideoColorUniform {
        let surface_format = self
            .surface_format
            .unwrap_or(wgpu::TextureFormat::Rgba8UnormSrgb);
        let layout = self
            .decoded_gpu_frame
            .as_ref()
            .map_or(DecodedPixelLayout::Nv12, DecodedGpuFrame::pixel_layout);
        video_color_uniform(
            self.color_profile,
            layout,
            shader_target_mode(surface_format, self.color_profile.is_hdr()),
        )
    }

    fn update_color_profile(&mut self, profile: VideoColorInfo, source_label: &str) {
        if self.color_flags.profile_initialized && self.color_profile == profile {
            return;
        }

        tracing::info!(
            "video GPU color profile source={} matrix={:?} primaries={:?} range={:?} transfer={:?} hdr={} wide_gamut={}",
            source_label,
            profile.matrix,
            profile.primaries,
            profile.range,
            profile.transfer,
            profile.is_hdr(),
            profile.is_wide_gamut(),
        );
        self.color_profile = profile;
        self.color_flags.profile_initialized = true;
        self.color_flags.uniform_dirty = true;
    }

    fn upload_color_uniform_if_needed(&mut self, queue: &wgpu::Queue) {
        if !self.color_flags.uniform_dirty {
            return;
        }

        let Some(buffer) = self.color_uniform_buffer.as_ref() else {
            return;
        };

        let bytes = self.current_color_uniform().to_bytes();
        queue.write_buffer(buffer, 0, &bytes);
        self.color_flags.uniform_dirty = false;
    }

    fn upload_spherical_projection_uniform_if_needed(
        &mut self,
        queue: &wgpu::Queue,
        surface_width: u32,
        surface_height: u32,
    ) {
        let VideoProjection::Equirectangular(projection) = &self.projection else {
            return;
        };
        let uniform = SphericalProjectionUniform::read(
            projection,
            surface_width.max(1),
            surface_height.max(1),
        );
        if self.last_spherical_projection_uniform == Some(uniform) {
            return;
        }
        let buffer = self
            .spherical_projection_uniform_buffer
            .as_ref()
            .expect("spherical render pipeline must own its projection uniform");
        queue.write_buffer(buffer, 0, &uniform.to_bytes());
        self.last_spherical_projection_uniform = Some(uniform);
    }

    fn install_spherical_projection_watchers(&mut self, redraw: &RedrawHandle) {
        let VideoProjection::Equirectangular(projection) = &self.projection else {
            return;
        };
        if !self.spherical_projection_watchers.is_empty() {
            return;
        }
        let viewport = projection.viewport();
        self.spherical_projection_watchers = [
            viewport.yaw_signal(),
            viewport.pitch_signal(),
            viewport.vertical_field_of_view_signal(),
        ]
        .into_iter()
        .map(|signal| {
            let redraw = redraw.clone();
            nami::Signal::watch(&signal, move |_| redraw.request_redraw())
        })
        .collect();
    }

    fn ensure_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if self.render_pipeline.is_some() {
            return;
        }
        self.surface_format = Some(format);
        self.color_flags.uniform_dirty = true;
        tracing::info!("create video GPU render pipeline surface_format={format:?}");

        let spherical = self.projection.is_spherical();
        let bind_group_layout = create_video_bind_group_layout(device, spherical);
        let render_pipeline =
            create_video_render_pipeline(device, &bind_group_layout, format, spherical);
        let sampler = create_video_sampler(device);
        let color_uniform_buffer =
            create_color_uniform_buffer(device, self.current_color_uniform());
        let spherical_projection_uniform_buffer = spherical.then(|| {
            let VideoProjection::Equirectangular(projection) = &self.projection else {
                unreachable!("spherical projection mode must retain its configuration");
            };
            create_spherical_projection_uniform_buffer(
                device,
                SphericalProjectionUniform::read(projection, 1, 1),
            )
        });

        self.bind_group_layout = Some(bind_group_layout);
        self.render_pipeline = Some(render_pipeline);
        self.sampler = Some(sampler);
        self.color_uniform_buffer = Some(color_uniform_buffer);
        self.spherical_projection_uniform_buffer = spherical_projection_uniform_buffer;
    }

    fn reconcile_source(&mut self) {
        let item = waterui_core::Signal::get(&self.source_signal);
        assert!(
            !self.projection.is_spherical() || item.drm.is_none(),
            "spherical projection cannot sample a platform-protected video surface"
        );
        let source_unchanged = item.source == self.source;
        let delivery_unchanged = item.delivery == self.delivery;
        let sidecars_unchanged = item.subtitle_tracks == self.sidecar_subtitle_tracks;
        let drm_unchanged = item.drm == self.drm;
        if source_unchanged && delivery_unchanged && sidecars_unchanged && drm_unchanged {
            return;
        }

        self.stop_decode_worker();
        if let Some(player) = self.audio.player.take() {
            player.stop();
        }
        if let Some(mut poller) = self.media_command_poller.take() {
            poller.stop();
        }
        self.media_session = None;

        self.source = item.source;
        self.delivery = item.delivery;
        self.drm = item.drm;
        self.sidecar_subtitle_tracks = item.subtitle_tracks;
        self.subtitle_tracks = self.sidecar_subtitle_tracks.clone();
        self.active_subtitle_track = None;
        self.pending_frame = None;
        self.presented_frame_history.clear();
        self.pending_frame_step = None;
        self.pending_forward_steps = 0;
        self.pending_backward_steps = 0;
        self.last_step_forward_generation = self.playback.step_forward_generation.get();
        self.last_step_backward_generation = self.playback.step_backward_generation.get();
        self.video_dimensions = None;
        self.duration = Duration::ZERO;
        self.live_window = None;
        self.live_playback_rate_range = None;
        self.live_catch_up_rate = NORMAL_PLAYBACK_RATE;
        self.source_path = None;
        self.source_asset = FileAssetState::Unresolved;
        self.subtitle_asset = None;
        self.source_flags = SourceFlags::default();
        self.subtitle_cues.clear();
        self.timed_metadata.clear();
        self.last_subtitle_text = None;
        self.download_generation = 0;
        self.decoder_waiting_for_download = None;
        self.downloaded_bytes = 0;
        self.download_total_bytes = None;
        self.last_reported_buffer_level_ms = None;
        self.observability.reset(
            self.playback_policy.network.initial_bandwidth(),
            Instant::now(),
        );
        self.playback_flags = PlaybackFlags::default();
        self.playback_anchor_pts = Duration::ZERO;
        self.playback_anchor_instant = None;
        self.control_flags.seek_inflight = false;
        self.pending_seek_request = None;
        self.last_handled_seek_generation = Some(self.playback.seek_generation.get());
        self.last_reported_progress = 0.0;
        self.color_profile = VideoColorInfo::default();
        self.color_flags.profile_initialized = false;
        self.color_flags.uniform_dirty = true;
        self.decoded_gpu_frame = None;
        self.bind_group = None;
        self.vertex_buffer = None;
        self.vertex_layout_key = None;

        self.push_progress_update(0.0);
        self.push_ui_update(UiUpdate::Duration(0.0));
        self.push_position_update(0.0);
        self.push_ui_update(UiUpdate::LiveWindow(None));
        self.push_ui_update(UiUpdate::AudioTracks(Vec::new()));
        self.push_ui_update(UiUpdate::VideoTracks(Vec::new()));
        self.push_ui_update(UiUpdate::Subtitle(String::new()));
        self.push_ui_update(UiUpdate::SubtitleTracks(runtime_subtitle_track_info(
            &self.subtitle_tracks,
        )));
        if let Some(redraw) = self.redraw_handle.as_ref() {
            redraw.request_redraw();
        }
    }

    fn reconcile_audio_track_selection(&mut self) {
        let selection = self.audio_track_selection.get();
        if selection == self.last_audio_track_selection {
            return;
        }
        self.last_audio_track_selection = selection;
        if self.decode_worker.is_none() {
            return;
        }

        let position = self.playback_position(Instant::now());
        if let Err(message) = self.restart_decoder_from_position(position) {
            self.emit_event(Event::Error { message });
            self.set_buffering(false);
        }
    }

    fn reconcile_video_track_selection(&mut self) {
        let selection = self.video_track_selection.get();
        if selection == self.last_video_track_selection {
            return;
        }
        self.last_video_track_selection = selection;
        if self.decode_worker.is_none() {
            return;
        }
        if matches!(self.delivery, Delivery::Progressive) {
            if let VideoTrackSelection::Track(index) = selection {
                self.emit_event(Event::Error {
                    message: format!(
                        "progressive playback has no adaptive video representation at index {index}"
                    ),
                });
            }
            return;
        }

        let position = self.playback_position(Instant::now());
        if let Err(message) = self.restart_decoder_from_position(position) {
            self.emit_event(Event::Error { message });
            self.set_buffering(false);
        }
    }

    fn reconcile_subtitle_track_selection(&mut self) {
        let selection = self.subtitle_selection.get();
        if selection == self.last_subtitle_selection {
            return;
        }

        let sidecar_track_count = self.sidecar_subtitle_tracks.len();
        let previous_segmented_selection =
            segmented_subtitle_track_selection(sidecar_track_count, self.last_subtitle_selection);
        let next_segmented_selection =
            segmented_subtitle_track_selection(sidecar_track_count, selection);
        self.last_subtitle_selection = selection;
        self.active_subtitle_track = None;
        self.subtitle_asset = None;
        self.subtitle_cues.clear();
        self.set_subtitle_text(None);

        if self.decode_worker.is_none()
            || matches!(self.delivery, Delivery::Progressive)
            || previous_segmented_selection == next_segmented_selection
        {
            return;
        }

        let position = self.playback_position(Instant::now());
        if let Err(message) = self.restart_decoder_from_position(position) {
            self.emit_event(Event::Error { message });
            self.set_buffering(false);
        }
    }

    const fn should_poll_source(&self) -> bool {
        matches!(self.delivery, Delivery::Progressive)
            && matches!(
                self.source_asset,
                FileAssetState::Unresolved | FileAssetState::Downloading { .. }
            )
    }

    const fn is_source_downloading(&self) -> bool {
        matches!(self.delivery, Delivery::Progressive)
            && matches!(self.source_asset, FileAssetState::Downloading { .. })
    }

    fn poll_source_download_updates(&mut self) {
        if !self.is_source_downloading() {
            return;
        }

        if let Err(message) = self.resolve_source_path() {
            if self.source_flags.source_error == ErrorReportState::Clear {
                self.emit_event(Event::Error { message });
                self.source_flags.source_error = ErrorReportState::Reported;
            }
            self.set_buffering(false);
            return;
        }
        self.resume_decoder_after_download_progress();
    }

    fn resume_decoder_after_download_progress(&mut self) {
        let Some(waiting_generation) = self.decoder_waiting_for_download else {
            return;
        };
        if self.download_generation <= waiting_generation {
            return;
        }
        let now = Instant::now();
        if !self.is_realtime_policy()
            && self.estimated_buffered_ahead_ms(now) < self.playback_policy.vod_resume_buffer_ms
        {
            return;
        }

        self.decoder_waiting_for_download = None;
        self.stop_decode_worker();
        let position = self.duration.mul_f64(self.last_reported_progress);
        if let Err(message) = self.restart_decoder_from_position(position) {
            self.emit_event(Event::Error { message });
            self.stop_decode_worker();
            self.set_buffering(false);
        }
    }

    fn resolve_source_path(&mut self) -> Result<Option<PathBuf>, String> {
        if !matches!(self.delivery, Delivery::Progressive) {
            return Err(String::from(
                "segmented media must be opened by the HLS/DASH session, not the progressive asset resolver",
            ));
        }
        loop {
            match &mut self.source_asset {
                FileAssetState::Unresolved => {
                    if is_remote_url(&self.source) {
                        return Ok(self.begin_remote_source_download());
                    }

                    let local_path = local_source_path(&self.source);
                    self.source_asset = FileAssetState::Ready(local_path.clone());
                    self.downloaded_bytes = downloaded_len(&local_path);
                    self.download_total_bytes = Some(self.downloaded_bytes);
                    return Ok(Some(local_path));
                }
                FileAssetState::Downloading {
                    path,
                    receiver,
                    ready,
                } => match receiver.try_recv() {
                    Ok(DownloadUpdate::Progress {
                        bytes_written,
                        total_bytes,
                    }) => {
                        if bytes_written > self.downloaded_bytes {
                            self.download_generation = self.download_generation.wrapping_add(1);
                        }
                        self.downloaded_bytes = bytes_written;
                        self.download_total_bytes = total_bytes;
                        self.observability
                            .record_progressive_transfer(bytes_written, Instant::now());
                    }
                    Ok(DownloadUpdate::Ready) => {
                        tracing::info!("video download became playable: {}", path.display());
                        self.download_generation = self.download_generation.wrapping_add(1);
                        *ready = true;
                        return Ok(Some(path.clone()));
                    }
                    Ok(DownloadUpdate::Finished(committed_path)) => {
                        tracing::info!("video download finished: {}", committed_path.display());
                        // Retry audio open once the file is complete. Early attempts on
                        // partial downloads may fail when container metadata isn't ready.
                        self.download_generation = self.download_generation.wrapping_add(1);
                        self.downloaded_bytes = downloaded_len(&committed_path);
                        self.download_total_bytes = Some(self.downloaded_bytes);
                        if self.source_path.as_ref() == Some(path) {
                            self.source_path = Some(committed_path.clone());
                        }
                        let resolved = committed_path;
                        self.source_asset = FileAssetState::Ready(resolved.clone());
                        return Ok(Some(resolved));
                    }
                    Ok(DownloadUpdate::Failed(message)) => {
                        tracing::warn!("video download failed: {message}");
                        self.source_asset = FileAssetState::Failed(message.clone());
                        return Err(message);
                    }
                    Err(TryRecvError::Empty) => {
                        return if *ready {
                            Ok(Some(path.clone()))
                        } else {
                            Ok(None)
                        };
                    }
                    Err(TryRecvError::Disconnected) => {
                        let message = String::from(
                            "video download channel disconnected before atomic cache commit",
                        );
                        self.source_asset = FileAssetState::Failed(message.clone());
                        return Err(message);
                    }
                },
                FileAssetState::Ready(path) => return Ok(Some(path.clone())),
                FileAssetState::Failed(message) => return Err(message.clone()),
            }
        }
    }

    fn begin_remote_source_download(&mut self) -> Option<PathBuf> {
        let cache_path = cached_video_path(&self.source);
        if cache_path.exists() {
            tracing::info!("using cached video source: {}", cache_path.display());
            self.source_asset = FileAssetState::Ready(cache_path.clone());
            self.downloaded_bytes = downloaded_len(&cache_path);
            self.download_total_bytes = Some(self.downloaded_bytes);
            return Some(cache_path);
        }

        tracing::info!("starting video download: {}", self.source.as_str());
        self.downloaded_bytes = 0;
        self.download_total_bytes = None;
        self.observability
            .begin_progressive_transfer(Instant::now());
        let (path, receiver) = start_asset_download(self.source.as_str(), cache_path);
        self.source_asset = FileAssetState::Downloading {
            path,
            receiver,
            ready: false,
        };
        None
    }

    fn set_subtitle_text(&mut self, next: Option<String>) {
        if self.last_subtitle_text == next {
            return;
        }

        let update = next.clone().unwrap_or_default();
        self.last_subtitle_text = next;
        self.push_ui_update(UiUpdate::Subtitle(update));
    }

    fn ensure_embedded_subtitle_tracks(&mut self) -> Result<(), String> {
        if self.source_flags.embedded_subtitle_tracks == CompletionState::Complete {
            return Ok(());
        }
        if !matches!(self.delivery, Delivery::Progressive) {
            return Ok(());
        }
        if !self.playback_flags.ready_sent {
            return Ok(());
        }

        let source_path = if let Some(path) = self.source_path.clone() {
            path
        } else {
            match self.resolve_source_path()? {
                Some(path) => path,
                None => return Ok(()),
            }
        };

        let embedded_tracks =
            embedded_subtitle_tracks(&source_path).map_err(|error| error.to_string())?;
        if !embedded_tracks.is_empty() {
            self.subtitle_tracks
                .extend(runtime_embedded_subtitle_tracks(&embedded_tracks));
            self.push_ui_update(UiUpdate::SubtitleTracks(runtime_subtitle_track_info(
                &self.subtitle_tracks,
            )));
        }
        self.source_flags.embedded_subtitle_tracks = CompletionState::Complete;
        Ok(())
    }

    const fn subtitle_track_discovery_pending(&self) -> bool {
        match self.delivery {
            Delivery::Progressive => {
                matches!(
                    self.source_flags.embedded_subtitle_tracks,
                    CompletionState::Pending
                )
            }
            Delivery::Hls | Delivery::Dash => {
                matches!(
                    self.source_flags.manifest_subtitle_tracks,
                    CompletionState::Pending
                )
            }
        }
    }

    fn sync_selected_subtitle_track(&mut self) -> Result<(), String> {
        self.ensure_embedded_subtitle_tracks()?;
        let selection = self.subtitle_selection.get();
        let next = match resolve_selected_subtitle_index(&self.subtitle_tracks, selection) {
            Ok(next) => next,
            Err(_)
                if self.subtitle_track_discovery_pending()
                    && matches!(selection, SubtitleSelection::Track(_)) =>
            {
                return Ok(());
            }
            Err(message) => return Err(message),
        };
        if self.active_subtitle_track == next {
            return Ok(());
        }

        self.active_subtitle_track = next;
        self.subtitle_asset = next.and_then(|index| {
            self.subtitle_tracks.get(index).and_then(|track| {
                matches!(track.source, RuntimeSubtitleSource::Sidecar(_))
                    .then_some(FileAssetState::Unresolved)
            })
        });
        self.source_flags.subtitle_error = ErrorReportState::Clear;
        self.subtitle_cues.clear();
        self.set_subtitle_text(None);
        Ok(())
    }

    fn active_subtitle_is_manifest(&self) -> bool {
        self.active_subtitle_track.is_some_and(|index| {
            self.subtitle_tracks
                .get(index)
                .is_some_and(|track| matches!(track.source, RuntimeSubtitleSource::Manifest(_)))
        })
    }

    fn replace_manifest_subtitle_tracks(&mut self, tracks: &[SelectableSubtitleTrack]) {
        let had_active_manifest = self.active_subtitle_is_manifest();
        self.subtitle_tracks
            .retain(|track| !matches!(track.source, RuntimeSubtitleSource::Manifest(_)));
        self.subtitle_tracks
            .extend(runtime_manifest_subtitle_tracks(tracks));
        self.source_flags.manifest_subtitle_tracks = CompletionState::Complete;
        if had_active_manifest {
            self.active_subtitle_track = None;
            self.subtitle_cues.clear();
            self.set_subtitle_text(None);
        }
        self.push_ui_update(UiUpdate::SubtitleTracks(runtime_subtitle_track_info(
            &self.subtitle_tracks,
        )));
    }

    fn append_manifest_subtitle_cues(&mut self, mut cues: Vec<SubtitleCue>) {
        if !self.active_subtitle_is_manifest() {
            return;
        }
        self.subtitle_cues.append(&mut cues);
        self.subtitle_cues.sort_by_key(|cue| (cue.start, cue.end));
        self.subtitle_cues.dedup_by(|later, earlier| {
            later.start == earlier.start && later.end == earlier.end && later.text == earlier.text
        });
    }

    fn ensure_subtitle_cues(&mut self) -> Result<(), String> {
        self.sync_selected_subtitle_track()?;

        let Some(track_index) = self.active_subtitle_track else {
            self.set_subtitle_text(None);
            return Ok(());
        };
        let track = self
            .subtitle_tracks
            .get(track_index)
            .ok_or_else(|| {
                format!(
                    "active subtitle track index {track_index} is out of range for {} tracks",
                    self.subtitle_tracks.len()
                )
            })?
            .clone();
        if !self.subtitle_cues.is_empty() {
            return Ok(());
        }

        match track.source {
            RuntimeSubtitleSource::Embedded(track) => self.load_embedded_subtitle_cues(&track),
            RuntimeSubtitleSource::Sidecar(track) => self.load_sidecar_subtitle_cues(track),
            RuntimeSubtitleSource::Manifest(_) => Ok(()),
        }
    }

    fn load_embedded_subtitle_cues(
        &mut self,
        track: &EmbeddedSubtitleSourceTrack,
    ) -> Result<(), String> {
        let source_path = if let Some(path) = self.source_path.clone() {
            path
        } else {
            match self.resolve_source_path()? {
                Some(path) => path,
                None => return Ok(()),
            }
        };
        self.subtitle_cues = read_embedded_subtitle_cues(&source_path, track.track_id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|cue| SubtitleCue {
                start: cue.start,
                end: cue.end,
                text: cue.text,
            })
            .collect();
        Ok(())
    }

    fn load_sidecar_subtitle_cues(&mut self, track: SubtitleTrack) -> Result<(), String> {
        let subtitle_source = track.source;
        let Some(asset) = self.subtitle_asset.as_mut() else {
            return Ok(());
        };

        loop {
            match asset {
                FileAssetState::Unresolved => {
                    if is_remote_url(&subtitle_source) {
                        let cache_path = cached_subtitle_path(&subtitle_source);
                        if cache_path.exists() {
                            self.subtitle_cues = parse_subtitles_from_path(&cache_path)
                                .map_err(|error| error.to_string())?;
                            *asset = FileAssetState::Ready(cache_path);
                            return Ok(());
                        }

                        let (path, receiver) =
                            start_asset_download(subtitle_source.as_str(), cache_path);
                        *asset = FileAssetState::Downloading {
                            path,
                            receiver,
                            ready: false,
                        };
                        return Ok(());
                    }

                    let local_path = local_source_path(&subtitle_source);
                    self.subtitle_cues = parse_subtitles_from_path(&local_path)
                        .map_err(|error| error.to_string())?;
                    *asset = FileAssetState::Ready(local_path);
                    return Ok(());
                }
                FileAssetState::Downloading { receiver, .. } => match receiver.try_recv() {
                    Ok(DownloadUpdate::Progress { .. } | DownloadUpdate::Ready) => {}
                    Ok(DownloadUpdate::Finished(committed_path)) => {
                        self.subtitle_cues = parse_subtitles_from_path(&committed_path)
                            .map_err(|error| error.to_string())?;
                        *asset = FileAssetState::Ready(committed_path);
                        return Ok(());
                    }
                    Err(TryRecvError::Disconnected) => {
                        let message =
                            String::from("subtitle download channel disconnected before commit");
                        *asset = FileAssetState::Failed(message.clone());
                        return Err(message);
                    }
                    Ok(DownloadUpdate::Failed(message)) => {
                        let message = format!("subtitle download failed: {message}");
                        *asset = FileAssetState::Failed(message.clone());
                        return Err(message);
                    }
                    Err(TryRecvError::Empty) => return Ok(()),
                },
                FileAssetState::Ready(_) => return Ok(()),
                FileAssetState::Failed(message) => return Err(message.clone()),
            }
        }
    }

    fn sync_subtitle_text(&mut self, now: Instant) {
        let position = self.playback_position(now);
        if self.active_subtitle_is_manifest() {
            let expired = self
                .subtitle_cues
                .partition_point(|cue| cue.end <= position);
            self.subtitle_cues.drain(..expired);
        }
        let next = active_subtitle_text(&self.subtitle_cues, position)
            .map(ToOwned::to_owned)
            .filter(|text| !text.trim().is_empty());
        self.set_subtitle_text(next);
    }

    fn append_timed_metadata(&mut self, metadata: Vec<EngineTimedMetadata>) {
        merge_timed_metadata(&mut self.timed_metadata, metadata);
    }

    fn sync_timed_metadata(&mut self, now: Instant) {
        let position = self.playback_position(now);
        let events = take_due_timed_metadata(&mut self.timed_metadata, position);
        for event in events {
            self.emit_event(Event::TimedMetadata {
                metadata: TimedMetadata::new(
                    event.scheme_id_uri(),
                    event.value(),
                    event.id(),
                    event.presentation_time(),
                    event.duration(),
                    event.message_data().to_vec(),
                ),
            });
        }
    }

    fn stop_decode_worker(&mut self) {
        if let Some(mut worker) = self.decode_worker.take() {
            worker.stop();
        }
        #[cfg(target_os = "android")]
        {
            self.pending_protected_frame = None;
        }
    }

    fn start_decode_worker(&mut self, source_path: &Path, start_progress: f64) {
        tracing::info!(
            "start video decoder worker source={} progress={start_progress:.3}",
            source_path.display()
        );
        self.stop_decode_worker();
        if let Err(message) = self.validate_self_drawn_power_policy() {
            self.handle_decoder_error(message);
            return;
        }
        let config = ProgressiveDecoderConfig::new(
            self.audio_output.clone(),
            self.audio_track_selection.get(),
            self.playback_policy.network.maximum_prefetch_buffer(),
        );
        #[cfg(target_os = "android")]
        let config = config.android_playback(self.android_playback_config());
        self.decode_worker = Some(DecoderWorker::spawn_progressive(
            source_path.to_path_buf(),
            config,
            start_progress,
        ));
        self.source_path = Some(source_path.to_path_buf());
        self.prepare_decode_worker();
    }

    fn start_segmented_decode_worker(
        &mut self,
        protocol: SegmentedProtocol,
        viewport: Option<(u32, u32)>,
        start_progress: f64,
    ) {
        tracing::info!(
            "start segmented video decoder source={} protocol={protocol:?} progress={start_progress:.3}",
            self.source.as_str()
        );
        self.stop_decode_worker();
        if let Err(message) = self.validate_self_drawn_power_policy() {
            self.handle_decoder_error(message);
            return;
        }
        if let Some(audio) = self.audio.player.take() {
            audio.stop();
        }
        if self.active_subtitle_is_manifest() {
            self.subtitle_cues.clear();
            self.set_subtitle_text(None);
        }
        let config = SegmentedDecoderConfig::new(
            self.playback_policy.network,
            viewport,
            self.audio_output.clone(),
            self.last_audio_track_selection,
            self.last_video_track_selection,
            segmented_subtitle_track_selection(
                self.sidecar_subtitle_tracks.len(),
                self.last_subtitle_selection,
            ),
        );
        #[cfg(target_os = "android")]
        let config = config.android_playback(self.android_playback_config());
        self.decode_worker = Some(DecoderWorker::spawn_segmented(
            self.source.as_str().to_owned(),
            protocol,
            config,
            start_progress,
        ));
        self.source_path = None;
        self.prepare_decode_worker();
    }

    fn validate_self_drawn_power_policy(&self) -> Result<(), String> {
        if cfg!(target_os = "android")
            || self.playback_policy.power == PlaybackPowerPolicy::PlatformManaged
        {
            return Ok(());
        }
        Err(format!(
            "required {:?} playback is unavailable on the self-drawn {} backend",
            self.playback_policy.power,
            std::env::consts::OS
        ))
    }

    #[cfg(target_os = "android")]
    fn android_playback_config(&self) -> AndroidPlaybackConfig {
        let clock = if self.playback_policy.realtime {
            AndroidPlaybackClock::Realtime
        } else {
            AndroidPlaybackClock::Fixed(self.playback_rate.get())
        };
        let video_access = if self.projection.is_spherical() {
            AndroidVideoAccess::GpuSamplingRequired
        } else {
            AndroidVideoAccess::DirectSurface
        };
        let audio_processing = if self.skip_silence.get() {
            AndroidAudioProcessing::SkipSilence
        } else {
            AndroidAudioProcessing::DirectCompressedEligible
        };
        let audio_route = if self.audio_output.selected_device().is_some() {
            AndroidAudioRoute::ExplicitDevice
        } else {
            AndroidAudioRoute::PlatformSelected
        };
        AndroidPlaybackConfig::new(
            self.drm.clone(),
            self.license_server.clone(),
            self.video_surface_receiver.clone(),
            self.playback_policy.power,
            AndroidPowerCompatibility::new(clock, video_access, audio_processing, audio_route),
        )
    }

    fn prepare_decode_worker(&mut self) {
        self.playback_output_path = PlaybackOutputPath::DecodedPcmGpu;
        self.pending_frame = None;
        self.timed_metadata.clear();
        #[cfg(target_os = "android")]
        {
            self.pending_protected_frame = None;
        }
        self.playback_flags.first_frame_presented = false;
        self.playback_flags.ended_sent = false;
        self.playback_flags.decoder_lifecycle = DecoderLifecycle::Active;
        self.last_handled_seek_generation = Some(self.playback.seek_generation.get());
        self.pending_seek_request = None;
        self.control_flags.seek_inflight = false;
        self.last_seek_restart_at = None;
        self.decoder_waiting_for_download = None;
        self.last_reported_buffer_level_ms = None;
        self.set_buffering(true);
    }

    fn restart_decoder_from_position(&mut self, position: Duration) -> Result<(), String> {
        match self.delivery {
            Delivery::Hls => {
                self.start_segmented_decode_worker(SegmentedProtocol::Hls, self.viewport, 0.0);
                if !position.is_zero()
                    && let Some(worker) = self.decode_worker.as_ref()
                {
                    worker.request_seek(position);
                }
                return Ok(());
            }
            Delivery::Dash => {
                self.start_segmented_decode_worker(SegmentedProtocol::Dash, self.viewport, 0.0);
                if !position.is_zero()
                    && let Some(worker) = self.decode_worker.as_ref()
                {
                    worker.request_seek(position);
                }
                return Ok(());
            }
            Delivery::Progressive => {}
        }
        let source_path = if let Some(path) = self.source_path.clone() {
            path
        } else {
            match self.resolve_source_path()? {
                Some(path) => path,
                None => return Ok(()),
            }
        };

        self.start_decode_worker(&source_path, progress_for_position(self.duration, position));
        Ok(())
    }

    fn open_decode_state(&mut self, viewport: (u32, u32)) {
        let restart_from_beginning =
            self.playback_flags.ended_sent && self.last_reported_progress >= 0.999;
        let start_progress = if restart_from_beginning {
            0.0
        } else {
            self.last_reported_progress.clamp(0.0, 1.0)
        };
        if restart_from_beginning {
            self.last_reported_progress = 0.0;
            self.playback_anchor_pts = Duration::ZERO;
            self.playback_anchor_instant = None;
            self.playback_flags.ended_sent = false;
        }

        match self.delivery {
            Delivery::Hls => {
                self.start_segmented_decode_worker(
                    SegmentedProtocol::Hls,
                    Some(viewport),
                    start_progress,
                );
                self.source_flags.source_error = ErrorReportState::Clear;
                return;
            }
            Delivery::Dash => {
                self.start_segmented_decode_worker(
                    SegmentedProtocol::Dash,
                    Some(viewport),
                    start_progress,
                );
                self.source_flags.source_error = ErrorReportState::Clear;
                return;
            }
            Delivery::Progressive => {}
        }

        let source_path = match self.resolve_source_path() {
            Ok(Some(path)) => path,
            Ok(None) => {
                self.set_buffering(true);
                return;
            }
            Err(message) => {
                if self.source_flags.source_error == ErrorReportState::Clear {
                    self.emit_event(Event::Error { message });
                    self.source_flags.source_error = ErrorReportState::Reported;
                }
                self.set_buffering(false);
                return;
            }
        };

        self.start_decode_worker(&source_path, start_progress);
        self.source_flags.source_error = ErrorReportState::Clear;
    }

    fn reconcile_play_request_from_ui(&mut self) {
        let ui_playing = self.playback.desired_playing.get();
        let user_initiated_change = self.pending_play_request_sync.is_none()
            && ui_playing != self.control_flags.play_requested;
        if let Some(pending) = self.pending_play_request_sync {
            if ui_playing == pending {
                self.pending_play_request_sync = None;
            } else {
                return;
            }
        }

        if user_initiated_change {
            self.audio.flags.resume_after_focus_gain = false;
            self.set_audio_focus_ducked(false);
        }

        self.update_play_requested(ui_playing);
    }

    fn set_play_requested(&mut self, playing: bool) {
        self.update_play_requested(playing);
        self.pending_play_request_sync = Some(playing);
        self.push_ui_update(UiUpdate::Playing(playing));
    }

    fn update_play_requested(&mut self, playing: bool) {
        if self.control_flags.play_requested == playing {
            return;
        }
        if playing && self.playback_flags.decoder_lifecycle == DecoderLifecycle::Failed {
            self.playback_flags.decoder_lifecycle = DecoderLifecycle::Active;
            self.source_flags.source_error = ErrorReportState::Clear;
        }
        self.control_flags.play_requested = playing;
        self.emit_event(Event::PlaybackStateChanged { playing });
    }

    fn set_audio_focus_ducked(&mut self, ducked: bool) {
        if self.audio.flags.focus_ducked == ducked {
            return;
        }

        self.audio.flags.focus_ducked = ducked;
        self.audio.last_applied_volume = None;
        self.sync_audio_volume();
    }

    fn queue_seek_request(&mut self, position: Duration, sync_player: bool) {
        let requested = self.clamp_seek_position(position);
        self.pending_seek_request = Some(requested);
        if sync_player {
            self.push_ui_update(UiUpdate::SeekRequest(requested));
        }
    }

    fn clamp_seek_position(&self, position: Duration) -> Duration {
        self.playback.live_window.get().map_or_else(
            || position.min(self.duration),
            |window| position.clamp(window.seekable_start(), window.seekable_end()),
        )
    }

    fn timeline_progress(&self, position: Duration) -> f64 {
        self.playback.live_window.get().map_or_else(
            || progress_for_position(self.duration, position),
            |window| {
                progress_for_position(
                    window
                        .seekable_end()
                        .saturating_sub(window.seekable_start()),
                    position.saturating_sub(window.seekable_start()),
                )
            },
        )
    }

    fn queue_navigation_controls(&self) -> QueueNavigationControls {
        QueueNavigationControls::disabled()
            .with_next_enabled(self.has_next.get())
            .with_previous_enabled(self.has_previous.get())
    }

    fn handle_media_command(&mut self, command: &MediaCommand) {
        let now = Instant::now();
        match command {
            MediaCommand::Play => {
                self.audio.flags.resume_after_focus_gain = false;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(true);
            }
            MediaCommand::Pause
            | MediaCommand::AudioFocusLost
            | MediaCommand::AudioBecomingNoisy => {
                self.audio.flags.resume_after_focus_gain = false;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(false);
            }
            MediaCommand::PlayPause => {
                self.audio.flags.resume_after_focus_gain = false;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(!self.control_flags.play_requested);
            }
            MediaCommand::Stop => {
                self.audio.flags.resume_after_focus_gain = false;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(false);
                let start = self
                    .playback
                    .live_window
                    .get()
                    .map_or(Duration::ZERO, LiveWindow::seekable_start);
                self.queue_seek_request(start, true);
            }
            MediaCommand::Seek(position) => {
                self.queue_seek_request(*position, true);
            }
            MediaCommand::SeekForward(delta) => {
                let target = self.playback_position(now).saturating_add(*delta);
                self.queue_seek_request(target, true);
            }
            MediaCommand::SeekBackward(delta) => {
                let target = self.playback_position(now).saturating_sub(*delta);
                self.queue_seek_request(target, true);
            }
            MediaCommand::AudioFocusGained => {
                self.set_audio_focus_ducked(false);
                if self.audio.flags.resume_after_focus_gain {
                    self.audio.flags.resume_after_focus_gain = false;
                    self.set_play_requested(true);
                }
            }
            MediaCommand::AudioFocusLostTransient => {
                self.audio.flags.resume_after_focus_gain = self.control_flags.play_requested;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(false);
            }
            MediaCommand::AudioFocusLostDuck => {
                self.audio.flags.resume_after_focus_gain = false;
                self.set_audio_focus_ducked(true);
            }
            MediaCommand::Next => {
                self.controller
                    .next()
                    .expect("enabled next media command must resolve a playlist item");
                self.emit_event(Event::NextRequested);
            }
            MediaCommand::Previous => {
                self.controller
                    .previous()
                    .expect("enabled previous media command must resolve a playlist item");
                self.emit_event(Event::PreviousRequested);
            }
            _ => {
                tracing::warn!("ignoring unsupported video media command {command:?}");
            }
        }
    }

    fn poll_media_commands(&mut self) {
        loop {
            let command = self
                .media_command_poller
                .as_ref()
                .and_then(RedrawCommandPoller::poll_command);
            let Some(command) = command else {
                break;
            };
            self.handle_media_command(&command);
        }
    }

    fn poll_picture_in_picture_commands(&mut self) {
        loop {
            let command = self
                .picture_in_picture_commands
                .poller
                .as_ref()
                .and_then(RedrawCommandPoller::poll_command);
            let Some(command) = command else {
                break;
            };

            match command {
                PictureInPictureCommand::Play => self.handle_media_command(&MediaCommand::Play),
                PictureInPictureCommand::Pause => self.handle_media_command(&MediaCommand::Pause),
                PictureInPictureCommand::SeekForward(delta) => {
                    self.handle_media_command(&MediaCommand::SeekForward(delta));
                }
                PictureInPictureCommand::SeekBackward(delta) => {
                    self.handle_media_command(&MediaCommand::SeekBackward(delta));
                }
                PictureInPictureCommand::ActiveChanged(active) => {
                    self.emit_picture_in_picture_changed(active);
                }
            }
        }
    }

    fn should_open_decode_worker(&self) -> bool {
        self.playback_flags.decoder_lifecycle != DecoderLifecycle::Failed
            && (self.source_path.is_none()
                || self.control_flags.play_requested
                || self.pending_seek_request.is_some())
    }

    const fn should_play(&self) -> bool {
        self.control_flags.play_requested
    }

    fn requested_playback_rate(&self) -> f32 {
        clamp_playback_rate(self.playback_rate.get() * self.live_catch_up_rate)
    }

    fn sync_live_catch_up_rate(&mut self, should_play: bool, now: Instant) {
        let user_rate = clamp_playback_rate(self.playback_rate.get());
        if !should_play
            || !self.playback_policy.realtime
            || (user_rate - NORMAL_PLAYBACK_RATE).abs() > 0.001
        {
            self.live_catch_up_rate = NORMAL_PLAYBACK_RATE;
            return;
        }
        let Some(window) = self.live_window else {
            self.live_catch_up_rate = NORMAL_PLAYBACK_RATE;
            return;
        };
        let network = self.playback_policy.network;
        let mut minimum = network.live_minimum_playback_rate();
        let mut maximum = network.live_maximum_playback_rate();
        if let Some(manifest) = self.live_playback_rate_range {
            minimum = minimum.max(manifest.minimum());
            maximum = maximum.min(manifest.maximum());
        }
        self.live_catch_up_rate = select_live_catch_up_rate(
            self.live_catch_up_rate,
            self.playback_position(now),
            window.target_position(),
            network.live_catch_up_tolerance(),
            minimum,
            maximum,
        );
    }

    fn requested_preserve_pitch(&self) -> bool {
        self.preserve_pitch.get()
    }

    const fn is_realtime_policy(&self) -> bool {
        self.playback_policy.realtime
    }

    fn live_drop_threshold(&self) -> Duration {
        Duration::from_millis(u64::from(
            self.playback_policy.live_max_video_late_ms.max(1),
        ))
    }

    fn estimated_buffered_ahead(&self, now: Instant) -> Duration {
        if let Some(buffered) = self
            .audio
            .player
            .as_ref()
            .map(PlaybackAudio::buffered_duration)
        {
            return buffered;
        }
        let playback = self.playback_position(now);
        if self.duration.is_zero() {
            return Duration::ZERO;
        }

        if self.is_source_downloading() {
            let downloaded_ahead =
                self.download_total_bytes
                    .filter(|total| *total > 0)
                    .map(|total| {
                        let ratio = (usize_to_f64(self.downloaded_bytes, "downloaded byte count")
                            / usize_to_f64(total, "total byte count"))
                        .clamp(0.0, 1.0);
                        self.duration.mul_f64(ratio).saturating_sub(playback)
                    });
            if let Some(buffered) = downloaded_ahead {
                return buffered;
            }
        }

        if let Some(frame) = self.pending_frame.as_ref() {
            return frame.timing().presentation_time().saturating_sub(playback);
        }
        #[cfg(target_os = "android")]
        if let Some(frame) = self.pending_protected_frame {
            return frame.presentation_time.saturating_sub(playback);
        }

        if matches!(self.source_asset, FileAssetState::Ready(_)) {
            return self.duration.saturating_sub(playback);
        }

        Duration::ZERO
    }

    fn estimated_buffered_ahead_ms(&self, now: Instant) -> u32 {
        self.estimated_buffered_ahead(now)
            .as_millis()
            .min(u128::from(u32::MAX))
            .try_into()
            .expect("buffered milliseconds are clamped to u32")
    }

    fn maybe_emit_buffer_level(&mut self, now: Instant) {
        let buffered_ms = self.estimated_buffered_ahead_ms(now);
        let should_emit = self
            .last_reported_buffer_level_ms
            .is_none_or(|last| last.abs_diff(buffered_ms) >= BUFFER_LEVEL_REPORT_STEP_MS);
        if !should_emit {
            return;
        }

        self.last_reported_buffer_level_ms = Some(buffered_ms);
        self.emit_event(Event::BufferLevel { buffered_ms });
    }

    fn video_timeline_position(&self, now: Instant) -> Duration {
        self.playback_anchor_instant
            .map_or(self.playback_anchor_pts, |anchor| {
                self.playback_anchor_pts.saturating_add(
                    now.saturating_duration_since(anchor)
                        .mul_f64(f64::from(self.last_playback_rate)),
                )
            })
    }

    fn av_drift_ms(&self, now: Instant) -> Option<f32> {
        let audio = self.audio.player.as_ref()?;
        let audio_position = audio.position();
        let video_position = self.video_timeline_position(now);
        let delta = audio_position.as_secs_f64() - video_position.as_secs_f64();
        Some(f64_to_f32(delta * 1000.0, "audio-video drift milliseconds"))
    }

    fn maybe_emit_playback_metrics(&mut self, now: Instant) {
        if !self.observability.should_report(now) {
            return;
        }
        let metrics = self.observability.snapshot(
            now,
            self.playback_position(now),
            self.estimated_buffered_ahead(now),
            self.av_drift_ms(now),
        );
        self.observability.last_report_at = Some(now);
        self.emit_event(Event::PlaybackMetrics { metrics });
    }

    fn should_wait_for_vod_buffer(&self, now: Instant) -> bool {
        should_wait_for_vod_buffering(
            self.playback_policy,
            self.is_source_downloading(),
            self.download_total_bytes.is_some(),
            self.playback_flags.first_frame_presented,
            self.estimated_buffered_ahead_ms(now),
        )
    }

    fn should_enter_vod_stall_buffering(&self, now: Instant) -> bool {
        should_enter_vod_stall_buffering(
            self.playback_policy,
            self.is_source_downloading(),
            self.download_total_bytes.is_some(),
            self.playback_flags.first_frame_presented,
            self.estimated_buffered_ahead_ms(now),
        )
    }

    fn playback_position(&self, now: Instant) -> Duration {
        let audio_position = self
            .last_playing_state
            .then(|| self.audio.player.as_ref().map(PlaybackAudio::position))
            .flatten();
        let anchor_elapsed = self
            .playback_anchor_instant
            .map(|anchor| now.saturating_duration_since(anchor));
        playback_clock_position(
            audio_position,
            self.playback_anchor_pts,
            anchor_elapsed,
            self.last_playback_rate,
        )
    }

    fn set_playback_position(&mut self, pts: Duration, should_play: bool) {
        self.playback_anchor_pts = pts;
        self.playback_anchor_instant = should_play.then(Instant::now);
        self.last_playing_state = should_play;
        if self.player.is_some() {
            self.push_position_update(pts.as_secs_f64());
        }
    }

    fn update_playing_state(&mut self, should_play: bool) {
        if self.last_playing_state == should_play {
            return;
        }

        if should_play {
            self.playback_anchor_instant = Some(Instant::now());
        } else {
            self.playback_anchor_pts = self.playback_position(Instant::now());
            self.playback_anchor_instant = None;
        }
        self.last_playing_state = should_play;
        self.sync_audio_playback_params();
        self.sync_audio_playback(should_play);
        self.sync_picture_in_picture_controller(should_play);
    }

    fn sync_playback_rate(&mut self, should_play: bool) {
        let requested_rate = self.requested_playback_rate();
        if (requested_rate - self.last_playback_rate).abs() <= 0.001 {
            return;
        }

        let now = Instant::now();
        let current_pts = self.playback_position(now);
        self.playback_anchor_pts = current_pts;
        self.playback_anchor_instant = should_play.then_some(now);
        self.last_playback_rate = requested_rate;
        self.sync_audio_playback_params();
        self.sync_media_session(should_play);
    }

    fn sync_media_session(&mut self, should_play: bool) {
        let position = self.playback_position(Instant::now());
        let hold_audio_focus = should_play || self.audio.flags.resume_after_focus_gain;
        let queue_navigation_controls = self.queue_navigation_controls();
        if let Some(session) = self.media_session.as_mut() {
            session.sync(
                should_play,
                hold_audio_focus,
                position,
                self.last_playback_rate,
                queue_navigation_controls,
            );
        }
    }

    fn emit_event(&self, event: Event) {
        self.push_ui_update(UiUpdate::Event(event));
    }

    fn set_buffering(&mut self, buffering: bool) {
        if self.control_flags.is_buffering == buffering {
            return;
        }

        let now = Instant::now();
        let count_as_rebuffer = self.playback_flags.first_frame_presented
            && self.should_play()
            && !self.control_flags.seek_inflight;
        self.observability
            .record_buffering(buffering, count_as_rebuffer, now);
        self.control_flags.is_buffering = buffering;
        if self.player.is_some() {
            self.push_ui_update(UiUpdate::Buffering(buffering));
        }

        self.emit_event(if buffering {
            Event::Buffering
        } else {
            Event::BufferingEnded
        });
        self.maybe_emit_buffer_level(now);
    }

    fn sync_audio_volume(&mut self) {
        let Some(audio) = self.audio.player.as_ref() else {
            return;
        };

        let volume = effective_audio_volume(
            self.volume.get(),
            self.muted.get(),
            self.audio.flags.focus_ducked,
        );

        if self
            .audio
            .last_applied_volume
            .is_some_and(|last| (last - volume).abs() <= 0.01)
        {
            return;
        }

        if let Err(error) = audio.set_volume(volume) {
            self.handle_decoder_error(format!("failed to set playback audio volume: {error}"));
            return;
        }
        self.audio.last_applied_volume = Some(volume);
    }

    fn sync_audio_playback_params(&mut self) {
        let Some(audio) = self.audio.player.as_ref() else {
            return;
        };

        let rate = self.requested_playback_rate();
        let preserve_pitch = self.requested_preserve_pitch();
        let streaming_rate_change = self
            .audio
            .last_applied_playback_rate
            .is_some_and(|last| (last - rate).abs() > 0.001)
            || self
                .audio
                .last_applied_preserve_pitch
                .is_some_and(|last| last != preserve_pitch);
        let rate_changed = self
            .audio
            .last_applied_playback_rate
            .is_none_or(|last| (last - rate).abs() > 0.001);
        let preserve_pitch_changed = self
            .audio
            .last_applied_preserve_pitch
            .is_none_or(|last| last != preserve_pitch);
        if !rate_changed && !preserve_pitch_changed {
            return;
        }

        if let Err(error) = audio.set_playback_rate(rate) {
            self.handle_decoder_error(format!("failed to set playback audio rate: {error}"));
            return;
        }
        if let Err(error) = audio.set_preserve_pitch(preserve_pitch) {
            self.handle_decoder_error(format!(
                "failed to set playback audio pitch policy: {error}"
            ));
            return;
        }

        self.audio.last_applied_playback_rate = Some(rate);
        self.audio.last_applied_preserve_pitch = Some(preserve_pitch);
        if streaming_rate_change {
            self.queue_seek_request(self.playback_position(Instant::now()), false);
        }
    }

    fn sync_audio_skip_silence(&mut self) {
        let enabled = self.skip_silence.get();
        let Some(audio) = self.audio.player.as_ref() else {
            return;
        };
        if self.audio.last_applied_skip_silence == Some(enabled) {
            return;
        }
        if let Err(error) = audio.set_skip_silence(enabled) {
            self.handle_decoder_error(format!(
                "failed to set playback audio skip-silence policy: {error}"
            ));
            return;
        }
        self.audio.last_applied_skip_silence = Some(enabled);
    }

    fn sync_audio_playback(&mut self, should_play: bool) {
        let Some(audio) = self.audio.player.as_ref() else {
            return;
        };

        let result = if should_play {
            audio.play()
        } else {
            audio.pause()
        };
        if let Err(error) = result {
            self.handle_decoder_error(format!("failed to update playback audio state: {error}"));
        }
    }

    fn pause_audio_for_seek(&mut self) {
        let Some(audio) = self.audio.player.as_ref() else {
            return;
        };
        if let Err(error) = audio.pause() {
            self.handle_decoder_error(format!("failed to pause playback audio for seek: {error}"));
        }
    }

    fn drain_decoder_outputs(&mut self, should_play: bool) {
        if self.playback_flags.decoder_lifecycle == DecoderLifecycle::Exhausted {
            return;
        }
        loop {
            if !self.drop_late_pending_frame_if_needed(should_play) {
                break;
            }

            let Some(output) = self.next_decoder_output() else {
                break;
            };

            match self.handle_decoder_output(output, should_play) {
                DecoderDrain::Continue => {}
                DecoderDrain::Break => break,
                DecoderDrain::Return => return,
            }
        }
    }

    fn drop_late_pending_frame_if_needed(&mut self, should_play: bool) -> bool {
        let Some(presentation_time) = self.pending_video_presentation_time() else {
            return true;
        };
        let late_by = self
            .playback_position(Instant::now())
            .saturating_sub(presentation_time);
        let late_frame_threshold = if self.is_realtime_policy() {
            self.live_drop_threshold()
        } else {
            VOD_FRAME_DROP_THRESHOLD
        };
        if !should_play || late_by <= late_frame_threshold {
            return false;
        }

        tracing::warn!(
            "dropping stale video frame late_by_ms={} progress={:.3}",
            late_by.as_millis(),
            self.last_reported_progress
        );
        self.observability.record_dropped_video_frame();
        self.pending_frame = None;
        #[cfg(target_os = "android")]
        if let Some(frame) = self.pending_protected_frame.take()
            && let Some(worker) = self.decode_worker.as_ref()
        {
            worker.discard_protected(frame.sequence);
        }
        true
    }

    fn pending_video_presentation_time(&self) -> Option<Duration> {
        let clear = self
            .pending_frame
            .as_ref()
            .map(|frame| frame.timing().presentation_time());
        #[cfg(target_os = "android")]
        {
            clear.or_else(|| {
                self.pending_protected_frame
                    .map(|frame| frame.presentation_time)
            })
        }
        #[cfg(not(target_os = "android"))]
        clear
    }

    fn next_decoder_output(&mut self) -> Option<DecoderOutput> {
        if self.decoder_waiting_for_download.is_some() {
            return None;
        }
        let worker = self.decode_worker.as_mut()?;
        match worker.try_recv() {
            Ok(output) => Some(output),
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => Some(DecoderOutput::Error(String::from(
                "Decoder worker disconnected",
            ))),
        }
    }

    fn handle_decoder_output(&mut self, output: DecoderOutput, should_play: bool) -> DecoderDrain {
        match output {
            DecoderOutput::Opened {
                duration,
                has_audio,
                video_dimensions,
                color_info,
            } => self.handle_decoder_opened(
                duration,
                has_audio,
                video_dimensions,
                color_info,
                should_play,
            ),
            DecoderOutput::SeekCompleted { pts } => {
                self.handle_decoder_seek_completed(pts, should_play);
                DecoderDrain::Continue
            }
            DecoderOutput::LiveWindow {
                window,
                playback_rate_range,
            } => self.handle_live_window_output(window, playback_rate_range),
            DecoderOutput::Frame(decoded) => self.handle_decoder_frame(decoded),
            #[cfg(target_os = "android")]
            DecoderOutput::ProtectedFrame {
                sequence,
                presentation_time,
            } => {
                if self.control_flags.seek_inflight {
                    if let Some(worker) = self.decode_worker.as_ref() {
                        worker.discard_protected(sequence);
                    }
                    DecoderDrain::Continue
                } else {
                    self.pending_protected_frame = Some(ProtectedPendingFrame {
                        sequence,
                        presentation_time,
                    });
                    self.playback_flags.ended_sent = false;
                    DecoderDrain::Break
                }
            }
            DecoderOutput::AudioTracks(tracks) => {
                self.push_ui_update(UiUpdate::AudioTracks(selectable_audio_track_info(&tracks)));
                DecoderDrain::Continue
            }
            DecoderOutput::VideoTracks(tracks) => {
                self.push_ui_update(UiUpdate::VideoTracks(selectable_video_track_info(&tracks)));
                DecoderDrain::Continue
            }
            DecoderOutput::SubtitleTracks(tracks) => self.handle_subtitle_tracks_output(&tracks),
            DecoderOutput::SubtitleCues(cues) => {
                self.append_manifest_subtitle_cues(cues);
                DecoderDrain::Continue
            }
            DecoderOutput::TimedMetadata(metadata) => {
                self.append_timed_metadata(metadata);
                DecoderDrain::Continue
            }
            DecoderOutput::NetworkThroughput(bits_per_second) => {
                self.observability
                    .record_network_throughput(bits_per_second);
                DecoderDrain::Continue
            }
            #[cfg(target_os = "android")]
            DecoderOutput::OfflineDrmKeySetChanged(key_set) => {
                self.emit_event(Event::OfflineDrmKeySetChanged { key_set });
                DecoderDrain::Continue
            }
            DecoderOutput::StreamingAudioOpened(player) => {
                self.handle_streaming_audio_opened(player, should_play)
            }
            #[cfg(target_os = "android")]
            DecoderOutput::OffloadedAudioOpened {
                controller,
                output_path,
            } => {
                self.playback_output_path = output_path;
                self.handle_playback_audio_opened(
                    PlaybackAudio::new_offloaded(controller),
                    should_play,
                )
            }
            #[cfg(target_os = "android")]
            DecoderOutput::TunneledVideoOutput { presentation_time } => {
                if !self.control_flags.seek_inflight {
                    self.commit_presented_video_frame(presentation_time, should_play);
                }
                DecoderDrain::Continue
            }
            DecoderOutput::Ended => self.handle_decoder_ended(),
            DecoderOutput::Error(message) => {
                self.handle_decoder_error(message);
                DecoderDrain::Return
            }
        }
    }

    fn handle_live_window_output(
        &mut self,
        window: Option<EngineLiveWindow>,
        playback_rate_range: Option<EngineLivePlaybackRateRange>,
    ) -> DecoderDrain {
        self.live_window = window;
        self.live_playback_rate_range = playback_rate_range;
        let window = window.map(|window| {
            LiveWindow::new(
                window.seekable_start(),
                window.seekable_end(),
                window.live_edge(),
                window.target_position(),
            )
        });
        self.push_ui_update(UiUpdate::LiveWindow(window));
        DecoderDrain::Continue
    }

    fn handle_subtitle_tracks_output(
        &mut self,
        tracks: &[SelectableSubtitleTrack],
    ) -> DecoderDrain {
        self.replace_manifest_subtitle_tracks(tracks);
        if let Err(message) = self.sync_selected_subtitle_track() {
            self.set_subtitle_text(None);
            if self.source_flags.subtitle_error == ErrorReportState::Clear {
                self.emit_event(Event::Error { message });
                self.source_flags.subtitle_error = ErrorReportState::Reported;
            }
        }
        DecoderDrain::Continue
    }

    fn handle_decoder_opened(
        &mut self,
        duration: Duration,
        has_audio: bool,
        video_dimensions: (u32, u32),
        color_info: VideoColorInfo,
        should_play: bool,
    ) -> DecoderDrain {
        tracing::info!(
            "video decoder opened duration={:.3}s",
            duration.as_secs_f64()
        );
        self.duration = duration;
        self.video_dimensions = Some(video_dimensions);
        let source_label = self.source.as_str().to_owned();
        self.update_color_profile(color_info, &source_label);
        if has_audio && self.audio.player.is_none() {
            self.handle_decoder_error(String::from(
                "decoder declared audio without transferring its playback output owner",
            ));
            return DecoderDrain::Return;
        }
        if !has_audio && let Some(player) = self.audio.player.take() {
            player.stop();
        }
        self.update_ui_progress();
        if self.media_session.is_none() {
            self.media_session = MediaSessionState::new(&self.source, duration);
            if self.media_session.is_some() {
                self.ensure_media_command_poller();
            }
        }
        if !self.playback_flags.ready_sent {
            self.emit_event(Event::PlaybackOutputPathChanged {
                path: self.playback_output_path,
            });
            self.emit_event(Event::ReadyToPlay);
            self.playback_flags.ready_sent = true;
        }
        self.sync_audio_playback(should_play);
        self.sync_picture_in_picture_controller(should_play);
        DecoderDrain::Continue
    }

    fn handle_decoder_seek_completed(&mut self, pts: Duration, should_play: bool) {
        self.control_flags.seek_inflight = false;
        self.pending_frame = None;
        self.timed_metadata.clear();
        #[cfg(target_os = "android")]
        {
            self.pending_protected_frame = None;
        }
        if self.active_subtitle_is_manifest() {
            self.subtitle_cues.clear();
            self.set_subtitle_text(None);
        }
        self.playback_flags.ended_sent = false;
        self.last_reported_progress = self.timeline_progress(pts);
        self.set_playback_position(pts, should_play);
        self.sync_audio_playback(should_play);
        self.sync_media_session(should_play);
        self.update_ui_progress();
    }

    fn handle_decoder_frame(&mut self, decoded: EngineDecodedVideoFrame) -> DecoderDrain {
        if self.control_flags.seek_inflight {
            return DecoderDrain::Continue;
        }
        if decoded.color_info() != self.color_profile {
            let source_label = self.source.as_str().to_owned();
            self.update_color_profile(decoded.color_info(), &source_label);
        }
        self.pending_frame = Some(decoded);
        self.playback_flags.ended_sent = false;
        self.decoder_waiting_for_download = None;
        DecoderDrain::Break
    }

    fn handle_streaming_audio_opened(
        &mut self,
        player: StreamingAudioPlayer,
        should_play: bool,
    ) -> DecoderDrain {
        self.handle_playback_audio_opened(PlaybackAudio::new(player), should_play)
    }

    fn handle_playback_audio_opened(
        &mut self,
        player: PlaybackAudio,
        should_play: bool,
    ) -> DecoderDrain {
        if let Some(previous) = self.audio.player.take() {
            previous.stop();
        }
        if let Err(error) = player.pause() {
            self.handle_decoder_error(format!(
                "failed to initialize playback audio state: {error}"
            ));
            return DecoderDrain::Return;
        }
        self.audio.player = Some(player);
        self.audio.last_applied_volume = None;
        self.audio.last_applied_playback_rate = None;
        self.audio.last_applied_preserve_pitch = None;
        self.audio.last_applied_skip_silence = None;
        self.sync_audio_volume();
        self.sync_audio_skip_silence();
        if self.playback_flags.ready_sent {
            self.sync_audio_playback(should_play);
        }
        DecoderDrain::Continue
    }

    fn handle_decoder_ended(&mut self) -> DecoderDrain {
        if self.control_flags.seek_inflight {
            return DecoderDrain::Continue;
        }
        tracing::info!("video decoder reached end");
        if self.is_source_downloading() {
            self.handle_downloading_decoder_end();
            return DecoderDrain::Return;
        }

        self.set_buffering(false);
        if let Some(audio) = self.audio.player.as_ref()
            && let Err(error) = audio.finish()
        {
            self.handle_decoder_error(format!("failed to finalize streaming audio: {error}"));
            return DecoderDrain::Return;
        }
        self.playback_flags.decoder_lifecycle = DecoderLifecycle::Exhausted;
        DecoderDrain::Break
    }

    fn handle_downloading_decoder_end(&mut self) {
        let now = Instant::now();
        if self.is_realtime_policy() {
            self.set_buffering(false);
        } else {
            self.set_buffering(true);
            self.maybe_emit_buffer_level(now);
            if self.estimated_buffered_ahead_ms(now) < self.playback_policy.vod_resume_buffer_ms {
                return;
            }
        }

        self.decoder_waiting_for_download = Some(self.download_generation);
    }

    fn loop_decoder_from_start(&mut self, should_play: bool) {
        if let Err(message) = self.restart_decoder_from_position(Duration::ZERO) {
            self.emit_event(Event::Error { message });
            self.stop_decode_worker();
            self.set_buffering(false);
            return;
        }
        self.last_reported_progress = 0.0;
        self.set_playback_position(Duration::ZERO, should_play);
        self.pause_audio_for_seek();
        self.update_ui_progress();
    }

    fn finish_decoder_playback(&mut self) {
        self.set_play_requested(false);
        self.stop_decode_worker();
        self.set_playback_position(self.duration, false);
        self.sync_audio_playback(false);
        self.sync_media_session(false);
    }

    fn maybe_finish_exhausted_playback(&mut self, should_play: bool, now: Instant) -> bool {
        if self.playback_flags.decoder_lifecycle != DecoderLifecycle::Exhausted
            || self.pending_frame.is_some()
        {
            return false;
        }
        if self
            .audio
            .player
            .as_ref()
            .map(PlaybackAudio::buffered_duration)
            .is_some_and(|buffered| buffered > PRESENT_TOLERANCE)
        {
            return false;
        }
        if self
            .playback_position(now)
            .saturating_add(PRESENT_TOLERANCE)
            < self.duration
        {
            return false;
        }

        self.playback_flags.decoder_lifecycle = DecoderLifecycle::Active;
        if !self.playback_flags.ended_sent {
            self.emit_event(Event::Ended);
            self.playback_flags.ended_sent = true;
        }
        if self.loops || self.playback.repeat.get() == RepeatMode::One {
            self.loop_decoder_from_start(should_play);
        } else if self.has_next.get() {
            request_next(&self.controller);
            self.stop_decode_worker();
            self.sync_audio_playback(false);
        } else {
            self.finish_decoder_playback();
        }
        true
    }

    fn handle_decoder_error(&mut self, message: String) {
        tracing::warn!("video decoder error: {message}");
        self.control_flags.seek_inflight = false;
        self.stop_decode_worker();
        self.pending_frame = None;
        self.set_play_requested(false);
        self.playback_flags.decoder_lifecycle = DecoderLifecycle::Failed;
        self.emit_event(Event::Error { message });
        self.set_buffering(false);
    }

    fn maybe_seek_from_ui(&mut self) {
        let generation = self.playback.seek_generation.get();
        if self.last_handled_seek_generation == Some(generation) {
            return;
        }
        self.last_handled_seek_generation = Some(generation);

        let requested = Duration::from_secs_f64(self.playback.seek_target_seconds.get().max(0.0));
        if requested.abs_diff(self.playback_position(Instant::now())) <= SEEK_POSITION_EPSILON {
            self.pending_seek_request = None;
            return;
        }

        self.queue_seek_request(requested, false);
    }

    fn reconcile_frame_step_requests(&mut self) {
        let forward_generation = self.playback.step_forward_generation.get();
        let backward_generation = self.playback.step_backward_generation.get();
        self.pending_forward_steps = self
            .pending_forward_steps
            .saturating_add(forward_generation.wrapping_sub(self.last_step_forward_generation));
        self.pending_backward_steps = self
            .pending_backward_steps
            .saturating_add(backward_generation.wrapping_sub(self.last_step_backward_generation));
        self.last_step_forward_generation = forward_generation;
        self.last_step_backward_generation = backward_generation;

        if self.playback_policy.power != PlaybackPowerPolicy::PlatformManaged
            && (self.pending_forward_steps > 0 || self.pending_backward_steps > 0)
        {
            self.pending_forward_steps = 0;
            self.pending_backward_steps = 0;
            self.emit_event(Event::Error {
                message: String::from(
                    "frame stepping requires decoded video-frame access and is incompatible with a required offload/tunneling path",
                ),
            });
            return;
        }

        if self.pending_frame_step.is_some() {
            return;
        }
        if self.pending_backward_steps > 0 {
            self.pending_backward_steps -= 1;
            let Some(target) = self.presented_frame_history.rewind() else {
                self.emit_event(Event::Error {
                    message: String::from(
                        "backward frame stepping requires a previous frame retained within the playback buffer",
                    ),
                });
                return;
            };
            self.pending_frame_step = Some(FrameStepDirection::Backward);
            self.queue_seek_request(target, false);
            return;
        }
        if self.pending_forward_steps == 0 {
            return;
        }
        self.pending_forward_steps -= 1;
        if self.decode_worker.is_none()
            || (self.playback_flags.decoder_lifecycle == DecoderLifecycle::Exhausted
                && self.pending_video_presentation_time().is_none())
        {
            self.emit_event(Event::Error {
                message: String::from("forward frame stepping reached the end of decoded media"),
            });
            return;
        }
        self.pending_frame_step = Some(FrameStepDirection::Forward);
    }

    fn maybe_enter_picture_in_picture(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };

        let request = player.picture_in_picture_request.get();
        if self.last_handled_picture_in_picture_request == Some(request) {
            return;
        }
        self.last_handled_picture_in_picture_request = Some(request);

        if request == 0 {
            return;
        }

        let aspect_ratio = self.current_video_dimensions();
        if let Err(error) = self.picture_in_picture_controller.enter(aspect_ratio) {
            self.emit_event(Event::Error {
                message: error.to_string(),
            });
        }
    }

    fn picture_in_picture_controller_state(
        &self,
        should_play: bool,
    ) -> PictureInPictureControllerState {
        let active = self.player.is_some()
            && (self.playback_flags.ready_sent || self.playback_flags.first_frame_presented);
        PictureInPictureControllerState::new(
            self.picture_in_picture_host_id,
            active,
            active && should_play,
            self.current_video_dimensions(),
        )
    }

    fn sync_picture_in_picture_controller(&mut self, should_play: bool) {
        let state = self.picture_in_picture_controller_state(should_play);
        if self.last_picture_in_picture_controller_state == Some(state) {
            return;
        }
        self.last_picture_in_picture_controller_state = Some(state);

        if let Err(error) = self.picture_in_picture_controller.sync(state) {
            self.emit_event(Event::Error {
                message: error.to_string(),
            });
        }
    }

    fn clear_picture_in_picture_controller(&mut self) {
        self.last_picture_in_picture_controller_state = None;
        let result = self
            .picture_in_picture_controller
            .sync(PictureInPictureControllerState::new(
                self.picture_in_picture_host_id,
                false,
                false,
                None,
            ));
        if let Err(error) = result {
            tracing::error!(%error, "failed to clear picture-in-picture controller");
        }
    }

    fn emit_picture_in_picture_changed(&mut self, active: bool) {
        if self.last_picture_in_picture_active == Some(active) {
            return;
        }

        self.last_picture_in_picture_active = Some(active);
        self.emit_event(Event::PictureInPictureChanged { active });
    }

    #[cfg(target_os = "android")]
    fn reconcile_picture_in_picture_status(&mut self, now: Instant) {
        if now < self.next_picture_in_picture_status_poll {
            return;
        }
        self.next_picture_in_picture_status_poll = now + PICTURE_IN_PICTURE_STATUS_POLL_INTERVAL;
        let active = match self.picture_in_picture_controller.is_active() {
            Ok(active) => active,
            Err(error) => {
                self.emit_event(Event::Error {
                    message: error.to_string(),
                });
                return;
            }
        };
        self.emit_picture_in_picture_changed(active);
    }

    fn apply_pending_seek_if_due(&mut self, should_play: bool) {
        let Some(requested) = self.pending_seek_request else {
            return;
        };

        let now = Instant::now();
        if self
            .last_seek_restart_at
            .is_some_and(|last| now.saturating_duration_since(last) < SEEK_RESTART_THROTTLE)
        {
            return;
        }

        self.last_seek_restart_at = Some(now);
        self.pending_seek_request = None;

        self.pending_frame = None;
        if self.pending_frame_step != Some(FrameStepDirection::Backward) {
            self.presented_frame_history.clear();
        }
        #[cfg(target_os = "android")]
        {
            self.pending_protected_frame = None;
        }
        self.playback_flags.ended_sent = false;
        self.control_flags.seek_inflight = true;

        self.set_playback_position(requested, should_play);
        self.last_reported_progress = self.timeline_progress(requested);
        self.pause_audio_for_seek();

        if let Some(worker) = self.decode_worker.as_ref() {
            worker.request_seek(requested);
        } else if let Err(message) = self.restart_decoder_from_position(requested) {
            self.control_flags.seek_inflight = false;
            self.emit_event(Event::Error { message });
            self.set_buffering(false);
            return;
        }

        self.set_buffering(true);
        self.sync_media_session(should_play);
        self.update_ui_progress();
    }

    fn update_ui_progress(&self) {
        if self.player.is_none() {
            return;
        }

        let position = self.playback_position(Instant::now());
        let progress = self.timeline_progress(position);
        self.push_progress_update(progress);
        self.push_ui_update(UiUpdate::Duration(self.duration.as_secs_f64()));
        self.push_position_update(position.as_secs_f64());
    }

    fn upload_frame_texture(&mut self, frame: &GpuFrame, decoded: DecodedFrame) {
        let decoded_gpu_frame = self
            .frame_uploader
            .upload(decoded, frame.device, frame.queue);
        if self
            .decoded_gpu_frame
            .as_ref()
            .is_none_or(|current| current.pixel_layout() != decoded_gpu_frame.pixel_layout())
        {
            self.color_flags.uniform_dirty = true;
        }
        let Some(bind_group_layout) = self.bind_group_layout.as_ref() else {
            return;
        };
        let Some(sampler) = self.sampler.as_ref() else {
            return;
        };
        let Some(color_uniform_buffer) = self.color_uniform_buffer.as_ref() else {
            return;
        };

        let y_view = decoded_gpu_frame
            .y_texture()
            .create_view(&wgpu::TextureViewDescriptor::default());
        let uv_view = decoded_gpu_frame
            .uv_texture()
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&y_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&uv_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: color_uniform_buffer.as_entire_binding(),
            },
        ];
        if let Some(projection_uniform) = self.spherical_projection_uniform_buffer.as_ref() {
            entries.push(wgpu::BindGroupEntry {
                binding: 5,
                resource: projection_uniform.as_entire_binding(),
            });
        }
        let bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Video bind group"),
            layout: bind_group_layout,
            entries: &entries,
        });

        self.decoded_gpu_frame = Some(decoded_gpu_frame);
        self.bind_group = Some(bind_group);
        self.vertex_buffer = None;
        self.vertex_layout_key = None;
    }

    fn ensure_vertex_buffer(
        &mut self,
        device: &wgpu::Device,
        surface_width: u32,
        surface_height: u32,
    ) {
        let Some(texture) = self.decoded_gpu_frame.as_ref() else {
            self.vertex_buffer = None;
            self.vertex_layout_key = None;
            return;
        };

        let key = VertexLayoutKey {
            surface_width: surface_width.max(1),
            surface_height: surface_height.max(1),
            video_width: texture.width().max(1),
            video_height: texture.height().max(1),
            aspect_ratio: if self.projection.is_spherical() {
                AspectRatio::Stretch
            } else {
                self.aspect_ratio
            },
        };

        if self.vertex_layout_key.is_some_and(|cached| cached == key) {
            return;
        }

        let vertices = build_vertices(
            key.aspect_ratio,
            key.video_width,
            key.video_height,
            key.surface_width,
            key.surface_height,
        );
        let mut bytes = Vec::with_capacity(vertices.len() * 4 * core::mem::size_of::<f32>());
        for vertex in vertices {
            for value in vertex {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Video quad vertex buffer"),
            size: usize_to_u64(bytes.len(), "vertex buffer length"),
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: true,
        });
        {
            let mut mapped = buffer.slice(..).get_mapped_range_mut();
            mapped.copy_from_slice(&bytes);
        }
        buffer.unmap();

        self.vertex_buffer = Some(buffer);
        self.vertex_layout_key = Some(key);
    }

    fn step_decoder_if_needed(&mut self, frame: &GpuFrame) {
        self.viewport = Some((frame.width.max(1), frame.height.max(1)));
        self.reconcile_source();
        self.reconcile_audio_track_selection();
        self.reconcile_video_track_selection();
        self.reconcile_subtitle_track_selection();
        self.poll_source_download_updates();
        if let Err(message) = self.ensure_subtitle_cues() {
            self.set_subtitle_text(None);
            if self.source_flags.subtitle_error == ErrorReportState::Clear {
                self.emit_event(Event::Error { message });
                self.source_flags.subtitle_error = ErrorReportState::Reported;
            }
        }

        self.reconcile_play_request_from_ui();
        self.poll_media_commands();
        self.poll_picture_in_picture_commands();

        if self.decode_worker.is_none() && self.should_open_decode_worker() {
            self.open_decode_state((frame.width.max(1), frame.height.max(1)));
        }

        let should_play = self.should_play();
        self.update_playing_state(should_play);
        self.sync_live_catch_up_rate(should_play, Instant::now());
        self.sync_playback_rate(should_play);
        self.sync_audio_volume();
        self.sync_audio_playback_params();
        self.sync_audio_skip_silence();
        self.maybe_seek_from_ui();
        self.reconcile_frame_step_requests();
        self.sync_picture_in_picture_controller(should_play);
        self.maybe_enter_picture_in_picture();
        #[cfg(target_os = "android")]
        self.reconcile_picture_in_picture_status(Instant::now());
        self.apply_pending_seek_if_due(should_play);
        self.drain_decoder_outputs(should_play);
        let now = Instant::now();
        if let Some(worker) = self.decode_worker.as_ref() {
            worker.update_buffered_duration(self.estimated_buffered_ahead(now));
        }
        self.sync_subtitle_text(now);
        self.sync_timed_metadata(now);
        self.maybe_emit_buffer_level(now);
        self.maybe_emit_playback_metrics(now);

        if self.decode_worker.is_none() {
            return;
        }

        if !should_play
            && self.pending_video_presentation_time().is_none()
            && self.decoded_gpu_frame.is_some()
        {
            self.set_buffering(false);
            self.sync_media_session(false);
            return;
        }

        if should_play && self.should_enter_vod_stall_buffering(now) {
            self.set_buffering(true);
        }

        if should_play && self.should_wait_for_vod_buffer(now) {
            self.set_buffering(true);
            return;
        }

        if self.maybe_finish_exhausted_playback(should_play, now) {
            return;
        }

        #[cfg(target_os = "android")]
        if self.present_protected_frame_if_due(should_play, now) {
            return;
        }

        let Some(pending) = self.pending_frame.as_ref() else {
            return;
        };
        let pending_pts = pending.timing().presentation_time();

        let present_immediately =
            self.decoded_gpu_frame.is_none() || self.pending_frame_step.is_some();
        let due = should_play
            && self
                .playback_position(now)
                .saturating_add(PRESENT_TOLERANCE)
                >= pending_pts;

        if !(present_immediately || due) {
            return;
        }

        let Some(decoded) = self.pending_frame.take() else {
            return;
        };

        let pts = decoded.timing().presentation_time();
        let decoded_frame = decoded.into_frame();
        self.upload_frame_texture(frame, decoded_frame);
        self.commit_presented_video_frame(pts, should_play);
    }

    #[cfg(target_os = "android")]
    fn present_protected_frame_if_due(&mut self, should_play: bool, now: Instant) -> bool {
        let Some(pending) = self.pending_protected_frame else {
            return false;
        };
        let present_immediately =
            !self.playback_flags.first_frame_presented || self.pending_frame_step.is_some();
        let due = should_play
            && self
                .playback_position(now)
                .saturating_add(PRESENT_TOLERANCE)
                >= pending.presentation_time;
        if !(present_immediately || due) {
            return true;
        }
        let Some(worker) = self.decode_worker.as_ref() else {
            return true;
        };
        worker.present_protected(pending.sequence, Duration::ZERO);
        self.pending_protected_frame = None;
        self.commit_presented_video_frame(pending.presentation_time, should_play);
        true
    }

    fn commit_presented_video_frame(&mut self, pts: Duration, should_play: bool) {
        self.presented_frame_history.record(pts);
        self.pending_frame_step = None;
        let progress = self.timeline_progress(pts);
        let now = Instant::now();
        if !self.playback_flags.first_frame_presented {
            tracing::info!(
                "first video frame presented pts={:.3}s progress={:.3}",
                pts.as_secs_f64(),
                progress
            );
            self.playback_flags.first_frame_presented = true;
            self.observability.record_first_frame(now);
        }
        self.set_playback_position(pts, should_play);
        self.sync_subtitle_text(now);
        self.sync_timed_metadata(now);
        self.set_buffering(false);
        self.sync_picture_in_picture_controller(should_play);

        if self.player.is_some() {
            self.push_progress_update(progress);
            self.push_position_update(pts.as_secs_f64());
            self.last_reported_progress = progress;
        }

        self.sync_media_session(should_play);
        self.maybe_finish_exhausted_playback(should_play, Instant::now());
    }

    fn render_surface(&mut self, frame: &GpuFrame) {
        self.ensure_vertex_buffer(frame.device, frame.width, frame.height);
        self.upload_color_uniform_if_needed(frame.queue);
        self.upload_spherical_projection_uniform_if_needed(frame.queue, frame.width, frame.height);

        let Some(pipeline) = self.render_pipeline.as_ref() else {
            return;
        };
        let Some(bind_group) = self.bind_group.as_ref() else {
            return;
        };
        let Some(vertex_buffer) = self.vertex_buffer.as_ref() else {
            return;
        };

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Video render encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Video render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.draw(0..6, 0..1);
        }

        frame.queue.submit([encoder.finish()]);
    }

    fn current_video_dimensions(&self) -> Option<(u32, u32)> {
        if let Some(key) = self.vertex_layout_key {
            return Some((key.video_width.max(1), key.video_height.max(1)));
        }

        if let Some(frame) = self.pending_frame.as_ref() {
            return Some((frame.width().max(1), frame.height().max(1)));
        }

        self.decoded_gpu_frame
            .as_ref()
            .map(|frame| (frame.width().max(1), frame.height().max(1)))
            .or(self.video_dimensions)
    }

    fn current_video_aspect_ratio(&self) -> Option<f32> {
        self.current_video_dimensions().map(|(width, height)| {
            u32_to_f32(width, "video width") / u32_to_f32(height, "video height")
        })
    }
}

impl GpuView for VideoRenderer {
    fn preferred_surface_hdr(&self) -> Option<bool> {
        // The persistent renderer may switch between SDR and HDR sources. A float
        // swapchain preserves both; the shader maps SDR into its linear target.
        Some(true)
    }

    fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        _env: &mut waterui_core::Environment,
    ) -> impl core::future::Future<Output = ()> {
        self.redraw_handle = Some(ctx.redraw_handle.clone());
        self.install_spherical_projection_watchers(&ctx.redraw_handle);
        self.ensure_picture_in_picture_command_poller();
        self.ensure_pipeline(ctx.device, ctx.surface_format);
        core::future::ready(())
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        self.step_decoder_if_needed(frame);
        self.render_surface(frame);

        // Request continuous redraw while playback/buffering is active
        let needs_redraw = if self.decode_worker.is_none() {
            self.should_poll_source() || self.should_play() || self.control_flags.is_buffering
        } else {
            self.pending_video_presentation_time().is_some()
                || self.should_play()
                || self.control_flags.is_buffering
        };
        if needs_redraw {
            frame.request_redraw();
        }
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        if self.projection.is_spherical() || self.aspect_ratio != AspectRatio::Fit {
            return ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            ));
        }

        let ratio = self.current_video_aspect_ratio();
        let Some(ratio) = ratio else {
            return ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            ));
        };

        match (proposal.width, proposal.height) {
            (Some(width), Some(height)) => ViewDimensions::new(Size::new(width, height)),
            (Some(width), None) => ViewDimensions::new(Size::new(width, width / ratio)),
            (None, Some(height)) => ViewDimensions::new(Size::new(height * ratio, height)),
            (None, None) => {
                if let Some((video_width, video_height)) = self.current_video_dimensions() {
                    ViewDimensions::new(Size::new(
                        u32_to_f32(video_width, "video width"),
                        u32_to_f32(video_height, "video height"),
                    ))
                } else {
                    ViewDimensions::new(Size::zero())
                }
            }
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        if self.projection.is_spherical() {
            return StretchAxis::Both;
        }
        match self.aspect_ratio {
            AspectRatio::Fit => StretchAxis::Horizontal,
            AspectRatio::Fill | AspectRatio::Stretch => StretchAxis::Both,
        }
    }
}

impl Drop for VideoRenderer {
    fn drop(&mut self) {
        self.clear_picture_in_picture_controller();
        self.stop_decode_worker();
        if let Some(mut poller) = self.media_command_poller.take() {
            poller.stop();
        }
        if let Some(mut poller) = self.picture_in_picture_commands.poller.take() {
            poller.stop();
        }
        if let Some(player) = self.audio.player.take() {
            player.stop();
        }
    }
}

struct RedrawCommandPoller<T: Send + 'static> {
    stop: AsyncSender<()>,
    commands: Receiver<T>,
    handle: Option<thread::JoinHandle<()>>,
}

impl<T: Send + 'static> RedrawCommandPoller<T> {
    fn spawn(command_source: AsyncReceiver<T>, redraw_handle: RedrawHandle) -> Self {
        let (stop, stop_receiver) = async_channel::bounded(1);
        let (command_sender, commands) = mpsc::channel();
        let handle = thread::spawn(move || {
            futures::executor::block_on(async move {
                loop {
                    let command = command_source.recv().fuse();
                    let stopped = stop_receiver.recv().fuse();
                    futures::pin_mut!(command, stopped);
                    futures::select_biased! {
                        _ = stopped => break,
                        received = command => match received {
                            Ok(command) => {
                                if command_sender.send(command).is_err() {
                                    break;
                                }
                                redraw_handle.request_redraw();
                            }
                            Err(_) => break,
                        },
                    }
                }
            });
        });

        Self {
            stop,
            commands,
            handle: Some(handle),
        }
    }

    fn poll_command(&self) -> Option<T> {
        self.commands.try_recv().ok()
    }

    fn stop(&mut self) {
        self.stop.close();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl<T: Send + 'static> Drop for RedrawCommandPoller<T> {
    fn drop(&mut self) {
        self.stop();
    }
}

struct MediaSessionState {
    session: MediaSession,
    queue_navigation_controls: QueueNavigationControls,
    has_audio_focus: bool,
}

impl MediaSessionState {
    fn new(source: &Url, duration: Duration) -> Option<Self> {
        let session = match MediaSession::new() {
            Ok(session) => session,
            Err(error) => {
                tracing::warn!(%error, "system media session is unavailable for video playback");
                return None;
            }
        };
        if let Err(error) = session.set_metadata(
            &MediaMetadata::new()
                .with_title(source.as_str())
                .with_duration(duration),
        ) {
            tracing::warn!(%error, "failed to publish video metadata to the system media session");
        }
        Some(Self {
            session,
            queue_navigation_controls: QueueNavigationControls::disabled(),
            has_audio_focus: false,
        })
    }

    fn command_receiver(&self) -> AsyncReceiver<MediaCommand> {
        self.session.command_receiver()
    }

    fn sync(
        &mut self,
        playing: bool,
        hold_audio_focus: bool,
        position: Duration,
        playback_rate: f32,
        queue_navigation_controls: QueueNavigationControls,
    ) {
        if hold_audio_focus && !self.has_audio_focus {
            match self.session.request_audio_focus() {
                Ok(()) => self.has_audio_focus = true,
                Err(error) => {
                    tracing::warn!(%error, "failed to acquire audio focus for video playback");
                }
            }
        } else if !hold_audio_focus && self.has_audio_focus {
            if let Err(error) = self.session.abandon_audio_focus() {
                tracing::warn!(%error, "failed to release audio focus for video playback");
            }
            self.has_audio_focus = false;
        }

        self.queue_navigation_controls = queue_navigation_controls;

        let playback = if playing {
            PlaybackState::playing(position)
        } else {
            PlaybackState::paused(position)
        };
        let playback = if playing {
            playback.with_rate(f64::from(playback_rate))
        } else {
            playback
        }
        .with_queue_navigation_controls(self.queue_navigation_controls);
        if let Err(error) = self.session.set_playback_state(&playback) {
            tracing::warn!(%error, "failed to update the system video playback state");
        }
    }
}

impl Drop for MediaSessionState {
    fn drop(&mut self) {
        if let Err(error) = self.session.set_playback_state(
            &PlaybackState::stopped()
                .with_queue_navigation_controls(self.queue_navigation_controls),
        ) {
            tracing::warn!(%error, "failed to stop the system video playback state during teardown");
        }
        if self.has_audio_focus
            && let Err(error) = self.session.abandon_audio_focus()
        {
            tracing::warn!(%error, "failed to release video audio focus during teardown");
        }
        if let Err(error) = self.session.clear() {
            tracing::warn!(%error, "failed to clear the system video media session during teardown");
        }
    }
}

fn create_video_bind_group_layout(device: &wgpu::Device, spherical: bool) -> wgpu::BindGroupLayout {
    let mut entries = vec![
        video_texture_layout_entry(0),
        video_texture_layout_entry(1),
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
        uniform_layout_entry(3),
    ];
    if spherical {
        entries.push(uniform_layout_entry(5));
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Video bind group layout"),
        entries: &entries,
    })
}

const fn uniform_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: Some(
                NonZeroU64::new(32).expect("video uniform min binding size is non-zero"),
            ),
        },
        count: None,
    }
}

const fn video_texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn create_video_render_pipeline(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    spherical: bool,
) -> wgpu::RenderPipeline {
    let shader_source = if spherical {
        Cow::Owned([YUV_COLOR_SHADER_WGSL, SPHERICAL_VIDEO_RENDER_SHADER_WGSL].concat())
    } else {
        Cow::Borrowed(YUV_COLOR_SHADER_WGSL)
    };
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Video render shader"),
        source: wgpu::ShaderSource::Wgsl(shader_source),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Video pipeline layout"),
        bind_group_layouts: &[bind_group_layout],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Video render pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: usize_to_u64(4 * core::mem::size_of::<f32>(), "video vertex stride"),
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 0,
                        format: wgpu::VertexFormat::Float32x2,
                    },
                    wgpu::VertexAttribute {
                        offset: usize_to_u64(
                            2 * core::mem::size_of::<f32>(),
                            "video vertex UV offset",
                        ),
                        shader_location: 1,
                        format: wgpu::VertexFormat::Float32x2,
                    },
                ],
            }],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(if spherical { "fs_spherical" } else { "fs_main" }),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn create_video_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Video sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

fn create_color_uniform_buffer(device: &wgpu::Device, uniform: VideoColorUniform) -> wgpu::Buffer {
    create_uniform_buffer(device, "Video color uniform", &uniform.to_bytes())
}

fn create_spherical_projection_uniform_buffer(
    device: &wgpu::Device,
    uniform: SphericalProjectionUniform,
) -> wgpu::Buffer {
    create_uniform_buffer(
        device,
        "Spherical video projection uniform",
        &uniform.to_bytes(),
    )
}

fn create_uniform_buffer(device: &wgpu::Device, label: &str, bytes: &[u8; 32]) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: 32,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: true,
    });
    {
        let mut mapped = buffer.slice(..).get_mapped_range_mut();
        mapped.copy_from_slice(bytes);
    }
    buffer.unmap();
    buffer
}

fn is_remote_url(url: &Url) -> bool {
    matches!(url.scheme(), Some("http" | "https"))
}

fn local_source_path(url: &Url) -> PathBuf {
    PathBuf::from(url.as_str())
}

fn cached_remote_asset_path(url: &Url, default_extension: &str) -> PathBuf {
    let cache_root = dirs::cache_dir()
        .expect("self-drawn video playback requires a platform cache directory")
        .join("waterui")
        .join("video");
    let remote_url = StreamingUrl::parse(url.as_str())
        .expect("remote WaterUI video URL must be a valid absolute URL");
    AssetCache::new(cache_root).path_for(&remote_url, default_extension)
}

fn cached_video_path(url: &Url) -> PathBuf {
    cached_remote_asset_path(url, "mp4")
}

fn cached_subtitle_path(url: &Url) -> PathBuf {
    cached_remote_asset_path(url, "vtt")
}

fn start_asset_download(url: &str, destination: PathBuf) -> (PathBuf, Receiver<DownloadUpdate>) {
    let (sender, receiver) = mpsc::channel();

    let remote_url = match StreamingUrl::parse(url) {
        Ok(url) => url,
        Err(error) => {
            let _ = sender.send(DownloadUpdate::Failed(error.to_string()));
            return (destination, receiver);
        }
    };
    let progress_quantum = NonZeroUsize::new(DOWNLOAD_PROGRESS_REPORT_INTERVAL_BYTES)
        .expect("download progress interval must be non-zero");
    let request = match ProgressiveDownloadRequest::new_cached(
        remote_url,
        destination.clone(),
        progress_quantum,
    ) {
        Ok(request) => request,
        Err(error) => {
            let _ = sender.send(DownloadUpdate::Failed(error.to_string()));
            return (destination, receiver);
        }
    };
    let growing_path = request.destination().to_owned();
    let probe_path = growing_path.clone();
    spawn_local(async move {
        let mut last_probe = 0usize;
        let mut ready_sent = false;
        let result = download(request, |event| {
            let transfer_finished = matches!(event, DownloadEvent::Finished(_));
            let progress = match event {
                DownloadEvent::Started(progress)
                | DownloadEvent::Progress(progress)
                | DownloadEvent::Finished(progress) => progress,
            };
            let _ = sender.send(DownloadUpdate::Progress {
                bytes_written: progress.bytes_written,
                total_bytes: progress.total_bytes,
            });

            let should_probe = !transfer_finished
                && !ready_sent
                && progress.bytes_written >= STREAMING_MIN_READY_BYTES
                && progress.bytes_written.saturating_sub(last_probe)
                    >= STREAMING_PROBE_INTERVAL_BYTES;
            if should_probe {
                last_probe = progress.bytes_written;
                if VideoReader::open(&probe_path).is_ok() {
                    ready_sent = true;
                    let _ = sender.send(DownloadUpdate::Ready);
                }
            }
        })
        .await;
        match result {
            Ok(receipt) => {
                let _ = sender.send(DownloadUpdate::Finished(receipt.destination().to_owned()));
            }
            Err(error) => {
                let _ = sender.send(DownloadUpdate::Failed(error.to_string()));
            }
        }
    })
    .detach();

    (growing_path, receiver)
}

fn build_vertices(
    aspect_ratio: AspectRatio,
    video_width: u32,
    video_height: u32,
    surface_width: u32,
    surface_height: u32,
) -> [[f32; 4]; 6] {
    let video_ratio = u32_to_f32(video_width.max(1), "video width")
        / u32_to_f32(video_height.max(1), "video height");
    let surface_ratio = u32_to_f32(surface_width.max(1), "surface width")
        / u32_to_f32(surface_height.max(1), "surface height");

    let mut scale_x = 1.0;
    let mut scale_y = 1.0;
    let mut u_min = 0.0;
    let mut u_max = 1.0;
    let mut v_min = 0.0;
    let mut v_max = 1.0;

    match aspect_ratio {
        AspectRatio::Fit => {
            if surface_ratio > video_ratio {
                scale_x = (video_ratio / surface_ratio).clamp(0.0, 1.0);
            } else {
                scale_y = (surface_ratio / video_ratio).clamp(0.0, 1.0);
            }
        }
        AspectRatio::Fill => {
            if surface_ratio > video_ratio {
                let visible_vertical = (video_ratio / surface_ratio).clamp(0.0, 1.0);
                let crop = (1.0 - visible_vertical) * 0.5;
                v_min = crop;
                v_max = 1.0 - crop;
            } else {
                let visible_horizontal = (surface_ratio / video_ratio).clamp(0.0, 1.0);
                let crop = (1.0 - visible_horizontal) * 0.5;
                u_min = crop;
                u_max = 1.0 - crop;
            }
        }
        AspectRatio::Stretch => {}
    }

    [
        [-scale_x, -scale_y, u_min, v_max],
        [scale_x, -scale_y, u_max, v_max],
        [scale_x, scale_y, u_max, v_min],
        [-scale_x, -scale_y, u_min, v_max],
        [scale_x, scale_y, u_max, v_min],
        [-scale_x, scale_y, u_min, v_min],
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        AspectRatio, ColorOutputTarget, DecodedPixelLayout, PlaybackObservability, PlaybackPolicy,
        PresentedFrameHistory, SphericalProjectionUniform, VideoColorInfo, Volume, build_vertices,
        create_color_uniform_buffer, create_spherical_projection_uniform_buffer,
        create_video_bind_group_layout, create_video_render_pipeline, create_video_sampler,
        effective_audio_volume, next_audio_selection, next_subtitle_selection,
        next_video_selection, playback_clock_position, progress_for_position,
        resolve_selected_subtitle_index, runtime_sidecar_subtitle_tracks,
        runtime_subtitle_track_info, segmented_subtitle_track_selection,
        select_default_subtitle_track_index, select_live_catch_up_rate, shader_target_mode,
        should_enter_vod_stall_buffering, should_wait_for_vod_buffering,
        subtitle_track_info_labels, take_due_timed_metadata, usize_to_u64, video_color_uniform,
    };
    use std::{
        fs,
        num::NonZeroU64,
        path::Path,
        time::{Duration, Instant},
    };
    use waterkit_video::{
        ColorPrimaries, ColorRange, ContentLightLevel, MatrixCoefficients,
        SubtitleTrackSelection as EngineSubtitleTrackSelection,
        TimedMetadata as EngineTimedMetadata, TransferFunction,
    };
    use waterui_graphics::{
        GpuContext, GpuFrame, GpuRuntime, GpuSurface, GpuView, OffscreenRenderConfig, OffscreenSize,
    };
    use waterui_video::{
        AudioTrackSelection, EquirectangularProjection, SphericalStereoLayout, SphericalViewport,
        SubtitleSelection, SubtitleTrack, VideoTrackSelection,
    };

    const VISUAL_WIDTH: u32 = 320;
    const VISUAL_HEIGHT: u32 = 180;
    const LIMITED_COLOR_BARS: [(u16, u16, u16); 8] = [
        (81, 90, 240),
        (145, 54, 34),
        (41, 240, 110),
        (210, 16, 146),
        (170, 166, 16),
        (106, 202, 222),
        (235, 128, 128),
        (16, 128, 128),
    ];

    #[test]
    fn playback_observability_keeps_source_lifetime_metrics_across_rebuffering() {
        let source_selected_at = Instant::now();
        let mut observability = PlaybackObservability::new(
            NonZeroU64::new(5_000_000).expect("initial bandwidth must be non-zero"),
            source_selected_at,
        );
        observability.record_first_frame(source_selected_at + Duration::from_millis(400));
        observability.record_dropped_video_frame();
        observability.record_network_throughput(
            NonZeroU64::new(12_000_000).expect("throughput must be non-zero"),
        );
        observability.record_buffering(true, true, source_selected_at + Duration::from_secs(1));

        let active = observability.snapshot(
            source_selected_at + Duration::from_millis(1_250),
            Duration::from_secs(8),
            Duration::from_secs(3),
            Some(-2.0),
        );
        assert_eq!(active.startup_time(), Duration::from_millis(400));
        assert_eq!(active.rebuffer_count(), 1);
        assert_eq!(active.rebuffer_duration(), Duration::from_millis(250));
        assert_eq!(active.dropped_video_frame_count(), 1);
        assert_eq!(active.observed_av_drift_ms(), Some(-2.0));

        observability.record_buffering(
            false,
            false,
            source_selected_at + Duration::from_millis(1_500),
        );
        let completed = observability.snapshot(
            source_selected_at + Duration::from_secs(2),
            Duration::from_secs(9),
            Duration::from_secs(2),
            None,
        );
        assert_eq!(completed.rebuffer_duration(), Duration::from_millis(500));
        assert_eq!(
            completed.observed_network_throughput_bps(),
            NonZeroU64::new(12_000_000)
        );
    }

    #[test]
    fn frame_history_rewinds_exactly_within_the_configured_window() {
        let mut history = PresentedFrameHistory::new(Duration::from_secs(2));
        history.record(Duration::from_secs(1));
        history.record(Duration::from_secs(2));
        history.record(Duration::from_secs(3));
        history.record(Duration::from_secs(4));

        assert_eq!(history.rewind(), Some(Duration::from_secs(3)));
        assert_eq!(history.rewind(), Some(Duration::from_secs(2)));
        assert_eq!(history.rewind(), None);
    }

    #[test]
    fn timed_metadata_queue_orders_deduplicates_and_drains_by_media_time() {
        let early = EngineTimedMetadata::new(
            "https://aomedia.org/emsg/ID3",
            "id3",
            1,
            Duration::from_secs(1),
            Duration::from_millis(250),
            b"early".to_vec(),
        );
        let late = EngineTimedMetadata::new(
            "urn:mpeg:dash:event:2012",
            "marker",
            2,
            Duration::from_secs(3),
            Duration::from_millis(500),
            b"late".to_vec(),
        );
        let mut queue = Vec::new();
        super::merge_timed_metadata(&mut queue, vec![late.clone(), early.clone(), early.clone()]);

        assert_eq!(queue, vec![early.clone(), late.clone()]);
        assert_eq!(
            take_due_timed_metadata(&mut queue, Duration::from_secs(2)),
            vec![early]
        );
        assert_eq!(queue, vec![late]);
    }

    struct VideoColorVisualRenderer {
        layout: DecodedPixelLayout,
        color: VideoColorInfo,
        pipeline: Option<wgpu::RenderPipeline>,
        bind_group: Option<wgpu::BindGroup>,
        vertex_buffer: Option<wgpu::Buffer>,
        spherical_projection: Option<SphericalProjectionUniform>,
    }

    impl VideoColorVisualRenderer {
        const fn new(layout: DecodedPixelLayout, color: VideoColorInfo) -> Self {
            Self {
                layout,
                color,
                pipeline: None,
                bind_group: None,
                vertex_buffer: None,
                spherical_projection: None,
            }
        }

        const fn spherical(
            layout: DecodedPixelLayout,
            color: VideoColorInfo,
            spherical_projection: SphericalProjectionUniform,
        ) -> Self {
            Self {
                layout,
                color,
                pipeline: None,
                bind_group: None,
                vertex_buffer: None,
                spherical_projection: Some(spherical_projection),
            }
        }
    }

    impl GpuView for VideoColorVisualRenderer {
        fn setup(
            &mut self,
            ctx: &GpuContext<'_>,
            _env: &mut waterui_core::Environment,
        ) -> impl core::future::Future<Output = ()> {
            let spherical = self.spherical_projection.is_some();
            let bind_group_layout = create_video_bind_group_layout(ctx.device, spherical);
            let pipeline = create_video_render_pipeline(
                ctx.device,
                &bind_group_layout,
                ctx.surface_format,
                spherical,
            );
            let sampler = create_video_sampler(ctx.device);
            let uniform = create_color_uniform_buffer(
                ctx.device,
                video_color_uniform(
                    self.color,
                    self.layout,
                    shader_target_mode(ctx.surface_format, self.color.is_hdr()),
                ),
            );
            let (y_texture, uv_texture) = create_visual_yuv_textures(ctx.device, self.layout);
            write_visual_color_bars(ctx.queue, &y_texture, &uv_texture, self.layout);
            let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let uv_view = uv_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let projection_uniform = self.spherical_projection.map(|projection| {
                create_spherical_projection_uniform_buffer(ctx.device, projection)
            });
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform.as_entire_binding(),
                },
            ];
            if let Some(projection_uniform) = projection_uniform.as_ref() {
                entries.push(wgpu::BindGroupEntry {
                    binding: 5,
                    resource: projection_uniform.as_entire_binding(),
                });
            }
            let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Video color visual bind group"),
                layout: &bind_group_layout,
                entries: &entries,
            });
            let vertices = build_vertices(
                AspectRatio::Stretch,
                VISUAL_WIDTH,
                VISUAL_HEIGHT,
                VISUAL_WIDTH,
                VISUAL_HEIGHT,
            );
            let mut vertex_bytes =
                Vec::with_capacity(vertices.len() * 4 * core::mem::size_of::<f32>());
            for vertex in vertices {
                for value in vertex {
                    vertex_bytes.extend_from_slice(&value.to_ne_bytes());
                }
            }
            let vertex_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Video color visual vertex buffer"),
                size: usize_to_u64(vertex_bytes.len(), "visual vertex buffer length"),
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: true,
            });
            {
                let mut mapped = vertex_buffer.slice(..).get_mapped_range_mut();
                mapped.copy_from_slice(&vertex_bytes);
            }
            vertex_buffer.unmap();

            self.pipeline = Some(pipeline);
            self.bind_group = Some(bind_group);
            self.vertex_buffer = Some(vertex_buffer);
            core::future::ready(())
        }

        fn render(&mut self, frame: &mut GpuFrame) {
            let pipeline = self
                .pipeline
                .as_ref()
                .expect("visual pipeline must be set up");
            let bind_group = self
                .bind_group
                .as_ref()
                .expect("visual bind group must be set up");
            let vertex_buffer = self
                .vertex_buffer
                .as_ref()
                .expect("visual vertex buffer must be set up");
            let mut encoder =
                frame
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Video color visual encoder"),
                    });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Video color visual pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, bind_group, &[]);
                pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                pass.draw(0..6, 0..1);
            }
            frame.queue.submit([encoder.finish()]);
        }
    }

    fn create_visual_yuv_textures(
        device: &wgpu::Device,
        layout: DecodedPixelLayout,
    ) -> (wgpu::Texture, wgpu::Texture) {
        let (y_format, uv_format) = match layout {
            DecodedPixelLayout::Nv12 => {
                (wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm)
            }
            DecodedPixelLayout::P010 => (
                wgpu::TextureFormat::R16Unorm,
                wgpu::TextureFormat::Rg16Unorm,
            ),
        };
        let create = |label, width, height, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        (
            create(
                "Video color visual Y",
                VISUAL_WIDTH,
                VISUAL_HEIGHT,
                y_format,
            ),
            create(
                "Video color visual UV",
                VISUAL_WIDTH / 2,
                VISUAL_HEIGHT / 2,
                uv_format,
            ),
        )
    }

    fn write_visual_color_bars(
        queue: &wgpu::Queue,
        y_texture: &wgpu::Texture,
        uv_texture: &wgpu::Texture,
        layout: DecodedPixelLayout,
    ) {
        let (y_plane, uv_plane, row_bytes) = visual_color_bar_planes(layout);
        queue.write_texture(
            y_texture.as_image_copy(),
            &y_plane,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(VISUAL_HEIGHT),
            },
            y_texture.size(),
        );
        queue.write_texture(
            uv_texture.as_image_copy(),
            &uv_plane,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(VISUAL_HEIGHT / 2),
            },
            uv_texture.size(),
        );
    }

    fn visual_color_bar_planes(layout: DecodedPixelLayout) -> (Vec<u8>, Vec<u8>, u32) {
        let bytes_per_component = match layout {
            DecodedPixelLayout::Nv12 => 1,
            DecodedPixelLayout::P010 => 2,
        };
        let row_bytes = VISUAL_WIDTH * bytes_per_component;
        let mut y_plane = Vec::with_capacity((row_bytes * VISUAL_HEIGHT) as usize);
        let mut uv_plane = Vec::with_capacity((row_bytes * VISUAL_HEIGHT / 2) as usize);
        for _ in 0..VISUAL_HEIGHT {
            for x in 0..VISUAL_WIDTH {
                let (y, _, _) = LIMITED_COLOR_BARS[color_bar_index(x)];
                extend_component(&mut y_plane, y, layout);
            }
        }
        for _ in 0..VISUAL_HEIGHT / 2 {
            for x in 0..VISUAL_WIDTH / 2 {
                let (_, u, v) = LIMITED_COLOR_BARS[color_bar_index(x * 2)];
                extend_component(&mut uv_plane, u, layout);
                extend_component(&mut uv_plane, v, layout);
            }
        }
        (y_plane, uv_plane, row_bytes)
    }

    fn color_bar_index(x: u32) -> usize {
        let color_bar_count = u32::try_from(LIMITED_COLOR_BARS.len())
            .expect("video visual color bar count must fit u32");
        usize::try_from((x * color_bar_count) / VISUAL_WIDTH)
            .expect("color bar index must fit usize")
    }

    fn extend_component(output: &mut Vec<u8>, component: u16, layout: DecodedPixelLayout) {
        match layout {
            DecodedPixelLayout::Nv12 => output.push(
                u8::try_from(component).expect("NV12 visual component must fit into eight bits"),
            ),
            DecodedPixelLayout::P010 => {
                output.extend_from_slice(&(component.saturating_mul(4) << 6).to_le_bytes());
            }
        }
    }

    fn export_video_color_visual(
        runtime: &GpuRuntime,
        output_dir: &Path,
        file_name: &str,
        layout: DecodedPixelLayout,
        color: VideoColorInfo,
    ) {
        let size = OffscreenSize::try_from_pixels(VISUAL_WIDTH, VISUAL_HEIGHT)
            .expect("video visual dimensions must be valid");
        let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8UnormSrgb);
        let mut env = waterui_core::Environment::new();
        let output = pollster::block_on(
            GpuSurface::new(VideoColorVisualRenderer::new(layout, color))
                .render_offscreen(runtime, config, &mut env),
        )
        .expect("video color visual must render through the production GPU shader");
        assert_eq!((output.width, output.height), (VISUAL_WIDTH, VISUAL_HEIGHT));
        output
            .save_png(output_dir.join(file_name))
            .expect("video color visual PNG must be saved");
    }

    fn export_spherical_video_visual(runtime: &GpuRuntime, output_dir: &Path) {
        let projection = EquirectangularProjection::new(SphericalViewport::new(25.0, 30.0, 100.0))
            .stereo_layout(SphericalStereoLayout::Mono);
        let uniform = SphericalProjectionUniform::read(&projection, VISUAL_WIDTH, VISUAL_HEIGHT);
        let size = OffscreenSize::try_from_pixels(VISUAL_WIDTH, VISUAL_HEIGHT)
            .expect("spherical video visual dimensions must be valid");
        let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8UnormSrgb);
        let mut env = waterui_core::Environment::new();
        let output = pollster::block_on(
            GpuSurface::new(VideoColorVisualRenderer::spherical(
                DecodedPixelLayout::Nv12,
                VideoColorInfo::default(),
                uniform,
            ))
            .render_offscreen(runtime, config, &mut env),
        )
        .expect("spherical visual must render through the production GPU shader");
        output
            .save_png(output_dir.join("nv12_equirectangular_mono.png"))
            .expect("spherical video visual PNG must be saved");
    }

    #[test]
    fn gpu_export_video_color_visuals() {
        let output_dir = Path::new("/tmp/waterui_video_visual");
        fs::create_dir_all(output_dir).expect("video visual output directory must be created");
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("video color visual requires a working GPU runtime");

        export_video_color_visual(
            &runtime,
            output_dir,
            "nv12_bt709_sdr.png",
            DecodedPixelLayout::Nv12,
            VideoColorInfo::default(),
        );
        export_video_color_visual(
            &runtime,
            output_dir,
            "p010_bt2020_pq_hdr10_to_sdr.png",
            DecodedPixelLayout::P010,
            VideoColorInfo {
                matrix: MatrixCoefficients::Bt2020NonConstantLuminance,
                primaries: ColorPrimaries::Bt2020,
                transfer: TransferFunction::Pq,
                range: ColorRange::Limited,
                content_light_level: Some(ContentLightLevel::new(1_000, 400)),
                dolby_vision: false,
            },
        );
        export_spherical_video_visual(&runtime, output_dir);
    }

    #[test]
    fn spherical_projection_uniform_encodes_viewport_layout_and_surface() {
        let projection = EquirectangularProjection::new(SphericalViewport::new(90.0, -45.0, 60.0))
            .stereo_layout(SphericalStereoLayout::TopBottom);
        let uniform = SphericalProjectionUniform::read(&projection, 3840, 2160);

        assert!((uniform.yaw_radians - core::f32::consts::FRAC_PI_2).abs() < f32::EPSILON);
        assert!((uniform.pitch_radians + core::f32::consts::FRAC_PI_4).abs() < f32::EPSILON);
        assert!(
            (uniform.vertical_field_of_view_radians - core::f32::consts::FRAC_PI_3).abs()
                < f32::EPSILON
        );
        assert_eq!(uniform.stereo_layout, 1);
        assert!((uniform.surface_aspect_ratio - 16.0 / 9.0).abs() < f32::EPSILON);
    }

    #[test]
    fn hdr_source_maps_to_sdr_on_srgb_surface() {
        let mode = shader_target_mode(wgpu::TextureFormat::Bgra8UnormSrgb, true);
        assert_eq!(mode, ColorOutputTarget::LinearSdr);
    }

    #[test]
    fn hdr_source_maps_to_hdr_on_float_surface() {
        let mode = shader_target_mode(wgpu::TextureFormat::Rgba16Float, true);
        assert_eq!(mode, ColorOutputTarget::LinearHdr);
    }

    #[test]
    fn sdr_source_stays_sdr_even_on_float_surface() {
        let mode = shader_target_mode(wgpu::TextureFormat::Rgba16Float, false);
        assert_eq!(mode, ColorOutputTarget::LinearSdr);
    }

    #[test]
    fn stalled_audio_does_not_freeze_video_clock_at_zero() {
        let position = playback_clock_position(
            Some(Duration::ZERO),
            Duration::ZERO,
            Some(Duration::from_millis(250)),
            1.0,
        );
        assert_eq!(position, Duration::from_millis(250));
    }

    #[test]
    fn progressing_audio_remains_the_authoritative_clock() {
        let position = playback_clock_position(
            Some(Duration::from_millis(80)),
            Duration::ZERO,
            Some(Duration::from_millis(250)),
            1.0,
        );
        assert_eq!(position, Duration::from_millis(80));
    }

    #[test]
    fn vod_buffer_wait_uses_start_then_resume_thresholds() {
        let policy = PlaybackPolicy::vod_default();
        assert!(should_wait_for_vod_buffering(
            policy, true, true, false, 900
        ));
        assert!(!should_wait_for_vod_buffering(
            policy, true, true, false, 1300
        ));

        assert!(should_wait_for_vod_buffering(policy, true, true, true, 700));
        assert!(!should_wait_for_vod_buffering(
            policy, true, true, true, 900
        ));
    }

    #[test]
    fn vod_stall_buffering_only_applies_after_first_frame() {
        let policy = PlaybackPolicy::vod_default();
        assert!(!should_enter_vod_stall_buffering(
            policy, true, true, false, 50
        ));
        assert!(should_enter_vod_stall_buffering(
            policy, true, true, true, 150
        ));
        assert!(!should_enter_vod_stall_buffering(
            policy, true, true, true, 500
        ));
    }

    #[test]
    fn realtime_policy_bypasses_vod_buffer_thresholds() {
        let policy = PlaybackPolicy::live_default();
        assert!(!should_wait_for_vod_buffering(policy, true, true, false, 0));
        assert!(!should_enter_vod_stall_buffering(
            policy, true, true, true, 0
        ));
    }

    #[test]
    fn default_subtitle_track_prefers_forced_track() {
        let tracks = vec![
            SubtitleTrack::new("https://example.com/subs/en.vtt").language("en"),
            SubtitleTrack::new("https://example.com/subs/forced.vtt")
                .language("en")
                .forced(true),
        ];
        let runtime_tracks = runtime_sidecar_subtitle_tracks(&tracks);

        let selected =
            select_default_subtitle_track_index(&runtime_tracks).expect("track must be selected");
        assert_eq!(selected, 1);
    }

    #[test]
    fn explicit_subtitle_track_selection_rejects_out_of_range_index() {
        let tracks = vec![SubtitleTrack::new("https://example.com/subs/en.vtt").language("en")];
        let runtime_tracks = runtime_sidecar_subtitle_tracks(&tracks);

        let error = resolve_selected_subtitle_index(&runtime_tracks, SubtitleSelection::Track(7))
            .expect_err("out-of-range selection must fail");
        assert!(error.contains("out of range"));
    }

    #[test]
    fn segmented_subtitle_selection_preserves_combined_track_indices() {
        assert_eq!(
            segmented_subtitle_track_selection(0, SubtitleSelection::Auto),
            EngineSubtitleTrackSelection::Auto
        );
        assert_eq!(
            segmented_subtitle_track_selection(1, SubtitleSelection::Auto),
            EngineSubtitleTrackSelection::Off
        );
        assert_eq!(
            segmented_subtitle_track_selection(1, SubtitleSelection::Track(0)),
            EngineSubtitleTrackSelection::Off
        );
        assert_eq!(
            segmented_subtitle_track_selection(1, SubtitleSelection::Track(2)),
            EngineSubtitleTrackSelection::Track(1)
        );
    }

    #[test]
    fn subtitle_selection_cycles_auto_off_tracks_then_auto() {
        let tracks = vec![
            SubtitleTrack::new("https://example.com/subs/en.vtt").language("en"),
            SubtitleTrack::new("https://example.com/subs/es.vtt").language("es"),
        ];
        let labels = subtitle_track_info_labels(&runtime_subtitle_track_info(
            &runtime_sidecar_subtitle_tracks(&tracks),
        ));

        assert_eq!(
            next_subtitle_selection(&labels, SubtitleSelection::Auto).expect("cycle must succeed"),
            SubtitleSelection::Off
        );
        assert_eq!(
            next_subtitle_selection(&labels, SubtitleSelection::Off).expect("cycle must succeed"),
            SubtitleSelection::Track(0)
        );
        assert_eq!(
            next_subtitle_selection(&labels, SubtitleSelection::Track(0))
                .expect("cycle must succeed"),
            SubtitleSelection::Track(1)
        );
        assert_eq!(
            next_subtitle_selection(&labels, SubtitleSelection::Track(1))
                .expect("cycle must succeed"),
            SubtitleSelection::Auto
        );
    }

    #[test]
    fn audio_selection_cycles_auto_tracks_then_auto() {
        let labels = vec![String::from("English"), String::from("Commentary")];

        assert_eq!(
            next_audio_selection(&labels, AudioTrackSelection::Auto).expect("cycle must succeed"),
            AudioTrackSelection::Track(0)
        );
        assert_eq!(
            next_audio_selection(&labels, AudioTrackSelection::Track(0))
                .expect("cycle must succeed"),
            AudioTrackSelection::Track(1)
        );
        assert_eq!(
            next_audio_selection(&labels, AudioTrackSelection::Track(1))
                .expect("cycle must succeed"),
            AudioTrackSelection::Auto
        );
    }

    #[test]
    fn audio_selection_rejects_out_of_range_track() {
        let error = next_audio_selection(&[String::from("English")], AudioTrackSelection::Track(4))
            .expect_err("out-of-range selection must fail");

        assert!(error.contains("out of range"));
    }

    #[test]
    fn video_selection_cycles_auto_tracks_then_auto() {
        let labels = vec![String::from("1280×720"), String::from("3840×2160 HDR")];

        assert_eq!(
            next_video_selection(&labels, VideoTrackSelection::Auto).expect("cycle must succeed"),
            VideoTrackSelection::Track(0)
        );
        assert_eq!(
            next_video_selection(&labels, VideoTrackSelection::Track(0))
                .expect("cycle must succeed"),
            VideoTrackSelection::Track(1)
        );
        assert_eq!(
            next_video_selection(&labels, VideoTrackSelection::Track(1))
                .expect("cycle must succeed"),
            VideoTrackSelection::Auto
        );
    }

    #[test]
    fn video_selection_rejects_out_of_range_track() {
        let error =
            next_video_selection(&[String::from("1280×720")], VideoTrackSelection::Track(4))
                .expect_err("out-of-range selection must fail");

        assert!(error.contains("out of range"));
    }

    #[test]
    fn progress_for_position_clamps_to_duration() {
        let duration = Duration::from_secs(10);

        assert!((progress_for_position(duration, Duration::ZERO) - 0.0).abs() <= f64::EPSILON);
        assert!(
            (progress_for_position(duration, Duration::from_secs(5)) - 0.5).abs() <= f64::EPSILON
        );
        assert!(
            (progress_for_position(duration, Duration::from_secs(15)) - 1.0).abs() <= f64::EPSILON
        );
    }

    #[test]
    fn effective_audio_volume_respects_muted_and_ducked_states() {
        let volume = Volume::new(0.8);
        assert!((effective_audio_volume(volume, false, false) - 0.8).abs() <= f32::EPSILON);
        assert!((effective_audio_volume(volume, false, true) - 0.16).abs() <= f32::EPSILON);
        assert!((effective_audio_volume(volume, true, false) - 0.0).abs() <= f32::EPSILON);
        assert!((effective_audio_volume(volume, true, true) - 0.0).abs() <= f32::EPSILON);
    }

    #[test]
    fn live_catch_up_uses_hysteresis_around_the_manifest_target() {
        let tolerance = Duration::from_millis(500);
        assert!(
            (select_live_catch_up_rate(
                1.0,
                Duration::from_secs(8),
                Duration::from_secs(10),
                tolerance,
                0.97,
                1.03,
            ) - 1.03)
                .abs()
                <= f32::EPSILON
        );
        assert!(
            (select_live_catch_up_rate(
                1.03,
                Duration::from_millis(9_800),
                Duration::from_secs(10),
                tolerance,
                0.97,
                1.03,
            ) - 1.0)
                .abs()
                <= f32::EPSILON
        );
        assert!(
            (select_live_catch_up_rate(
                1.0,
                Duration::from_secs(12),
                Duration::from_secs(10),
                tolerance,
                0.97,
                1.03,
            ) - 0.97)
                .abs()
                <= f32::EPSILON
        );
    }
}
