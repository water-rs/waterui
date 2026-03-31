use core::fmt;
use std::collections::hash_map::DefaultHasher;
use std::{
    collections::VecDeque,
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        OnceLock,
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

use executor_core::spawn_local;
use nami::Computed;
use uuid::Uuid;
use waterkit_audio::{
    AudioPlayer, MediaCommand, MediaMetadata, MediaSession, PlaybackState, QueueNavigationControls,
};
use waterkit_codec::{CodecType, DecodedFrame, Decoder};
use waterkit_video::{
    EmbeddedSubtitleTrack as EmbeddedSubtitleSourceTrack, PictureInPictureCommand,
    PictureInPictureControllerState, PictureInPictureHostId, VideoReader, embedded_subtitle_tracks,
    enter_picture_in_picture, is_picture_in_picture_active, poll_picture_in_picture_command,
    read_embedded_subtitle_cues, sync_picture_in_picture_controller,
};
use waterui_controls::{button, slider::slider};
use waterui_core::{
    AnyView, Binding, Environment, SignalExt as _, State, View, binding,
    dynamic::Dynamic,
    env::With,
    layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions},
};
use waterui_graphics::{Color, GpuContext, GpuFrame, GpuSurface, GpuView, RedrawHandle, wgpu};
use waterui_layout::{
    overlay,
    stack::{Alignment, hstack, vstack},
};
use waterui_text::{Text, text};

use crate::Url;
use crate::source::{MediaItem, SubtitleTrack};
use crate::subtitles::{SubtitleCue, active_subtitle_text, parse_subtitles_from_path};
use crate::video::{
    AspectRatio, Event, PlaybackPolicy, SubtitleSelection, VideoConfig, VideoPlayerConfig, Volume,
};

const SEEK_EPSILON: f64 = 0.005;
const SEEK_RESTART_THROTTLE: Duration = Duration::from_millis(40);
const PRESENT_TOLERANCE: Duration = Duration::from_millis(3);
const VOD_FRAME_DROP_THRESHOLD: Duration = Duration::from_millis(300);
const DECODE_FRAME_QUEUE_CAPACITY: usize = 4;
const STREAMING_PROBE_INTERVAL_BYTES: usize = 256 * 1024;
const STREAMING_MIN_READY_BYTES: usize = 512 * 1024;
const DOWNLOAD_PROGRESS_REPORT_INTERVAL_BYTES: usize = 64 * 1024;
const MIN_PLAYBACK_RATE: f32 = 0.25;
const MAX_PLAYBACK_RATE: f32 = 4.0;
const AUDIO_FOCUS_DUCK_FACTOR: f32 = 0.2;
const METRICS_REPORT_INTERVAL: Duration = Duration::from_millis(500);
const BUFFER_LEVEL_REPORT_STEP_MS: u32 = 50;
const MEDIA_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(250);

type OnEvent = Rc<dyn Fn(Event) + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum ColorMatrix {
    Bt709 = 0,
    Bt601 = 1,
    Bt2020 = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum ColorPrimaries {
    Bt709 = 0,
    Bt601 = 1,
    DisplayP3 = 2,
    Bt2020 = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum ColorRangeMode {
    Limited = 0,
    Full = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum TransferMode {
    Sdr = 0,
    Pq = 1,
    Hlg = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum ShaderTargetMode {
    GammaSdr = 0,
    LinearSdr = 1,
    LinearHdr = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VideoColorProfile {
    matrix: ColorMatrix,
    primaries: ColorPrimaries,
    range: ColorRangeMode,
    transfer: TransferMode,
    hdr: bool,
    wide_gamut: bool,
    dolby_vision: bool,
}

impl Default for VideoColorProfile {
    fn default() -> Self {
        Self {
            matrix: ColorMatrix::Bt709,
            primaries: ColorPrimaries::Bt709,
            range: ColorRangeMode::Limited,
            transfer: TransferMode::Sdr,
            hdr: false,
            wide_gamut: false,
            dolby_vision: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct VideoColorUniform {
    matrix_mode: u32,
    range_mode: u32,
    primaries_mode: u32,
    transfer_mode: u32,
    target_mode: u32,
    _padding0: u32,
    _padding1: u32,
    _padding2: u32,
}

fn shader_target_mode(format: wgpu::TextureFormat, source_is_hdr: bool) -> ShaderTargetMode {
    if matches!(
        format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    ) {
        if source_is_hdr {
            ShaderTargetMode::LinearHdr
        } else {
            ShaderTargetMode::LinearSdr
        }
    } else if format.is_srgb() {
        ShaderTargetMode::LinearSdr
    } else {
        ShaderTargetMode::GammaSdr
    }
}

fn color_uniform_bytes(uniform: VideoColorUniform) -> [u8; 32] {
    let mut bytes = [0_u8; 32];
    bytes[0..4].copy_from_slice(&uniform.matrix_mode.to_ne_bytes());
    bytes[4..8].copy_from_slice(&uniform.range_mode.to_ne_bytes());
    bytes[8..12].copy_from_slice(&uniform.primaries_mode.to_ne_bytes());
    bytes[12..16].copy_from_slice(&uniform.transfer_mode.to_ne_bytes());
    bytes[16..20].copy_from_slice(&uniform.target_mode.to_ne_bytes());
    bytes[20..24].copy_from_slice(&uniform._padding0.to_ne_bytes());
    bytes[24..28].copy_from_slice(&uniform._padding1.to_ne_bytes());
    bytes[28..32].copy_from_slice(&uniform._padding2.to_ne_bytes());
    bytes
}

fn clamp_playback_rate(rate: f32) -> f32 {
    if rate.is_finite() {
        rate.clamp(MIN_PLAYBACK_RATE, MAX_PLAYBACK_RATE)
    } else {
        1.0
    }
}

fn effective_audio_volume(requested: Volume, audio_focus_ducked: bool) -> f32 {
    let base_volume = if requested.is_sign_negative() {
        0.0
    } else {
        requested.clamp(0.0, 1.0)
    };
    if audio_focus_ducked {
        base_volume * AUDIO_FOCUS_DUCK_FACTOR
    } else {
        base_volume
    }
}

fn should_wait_for_vod_buffering(
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

fn should_enter_vod_stall_buffering(
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

#[derive(Debug, Clone, Copy)]
struct NclxColorInfo {
    primaries: u16,
    transfer: u16,
    matrix: u16,
    full_range: bool,
}

#[derive(Debug, Clone, Copy)]
struct DolbyVisionInfo {
    profile: u8,
    level: u8,
}

#[derive(Debug, Clone, Copy, Default)]
struct Mp4ColorMetadata {
    nclx: Option<NclxColorInfo>,
    dolby_vision: Option<DolbyVisionInfo>,
}

fn is_hdr_transfer(transfer: u16) -> bool {
    matches!(transfer, 16 | 18)
}

fn is_wide_gamut_primaries(primaries: u16) -> bool {
    matches!(primaries, 9 | 10 | 11 | 12)
}

fn map_color_matrix(matrix: u16, height_hint: Option<u32>) -> ColorMatrix {
    match matrix {
        1 => ColorMatrix::Bt709,
        5 | 6 => ColorMatrix::Bt601,
        9 | 10 => ColorMatrix::Bt2020,
        _ if height_hint.is_some_and(|h| h <= 576) => ColorMatrix::Bt601,
        _ => ColorMatrix::Bt709,
    }
}

fn map_color_primaries(primaries: u16, height_hint: Option<u32>) -> ColorPrimaries {
    match primaries {
        9 | 10 => ColorPrimaries::Bt2020,
        11 | 12 => ColorPrimaries::DisplayP3,
        5 | 6 | 7 => ColorPrimaries::Bt601,
        1 => ColorPrimaries::Bt709,
        _ if height_hint.is_some_and(|h| h <= 576) => ColorPrimaries::Bt601,
        _ => ColorPrimaries::Bt709,
    }
}

fn map_transfer_mode(transfer: u16) -> TransferMode {
    match transfer {
        16 => TransferMode::Pq,
        18 => TransferMode::Hlg,
        _ => TransferMode::Sdr,
    }
}

fn probe_video_color_profile(path: &Path, height_hint: Option<u32>) -> VideoColorProfile {
    let metadata = probe_mp4_color_metadata(path);
    let mut profile = if let Some(info) = metadata.nclx {
        let transfer = map_transfer_mode(info.transfer);
        VideoColorProfile {
            matrix: map_color_matrix(info.matrix, height_hint),
            primaries: map_color_primaries(info.primaries, height_hint),
            range: if info.full_range {
                ColorRangeMode::Full
            } else {
                ColorRangeMode::Limited
            },
            transfer,
            hdr: is_hdr_transfer(info.transfer),
            wide_gamut: is_wide_gamut_primaries(info.primaries),
            dolby_vision: false,
        }
    } else {
        VideoColorProfile {
            matrix: map_color_matrix(0, height_hint),
            primaries: map_color_primaries(0, height_hint),
            ..VideoColorProfile::default()
        }
    };

    if metadata.dolby_vision.is_some() {
        profile.dolby_vision = true;
        profile.hdr = true;
        if profile.transfer == TransferMode::Sdr {
            profile.transfer = TransferMode::Pq;
        }
        if profile.primaries == ColorPrimaries::Bt709 {
            profile.primaries = ColorPrimaries::Bt2020;
        }
        profile.wide_gamut = true;
    }

    profile
}

fn probe_mp4_color_metadata(path: &Path) -> Mp4ColorMetadata {
    let Ok(file) = File::open(path) else {
        return Mp4ColorMetadata::default();
    };
    let mut file = std::io::BufReader::new(file);
    let mut chunk = [0_u8; 64 * 1024];
    let mut carry = Vec::<u8>::new();
    let mut metadata = Mp4ColorMetadata::default();

    loop {
        let read = match file.read(&mut chunk) {
            Ok(read) => read,
            Err(_) => return metadata,
        };
        if read == 0 {
            break;
        }

        let mut buffer = Vec::with_capacity(carry.len() + read);
        buffer.extend_from_slice(&carry);
        buffer.extend_from_slice(&chunk[..read]);

        if metadata.nclx.is_none() {
            metadata.nclx = find_nclx_box(&buffer);
        }
        if metadata.dolby_vision.is_none() {
            metadata.dolby_vision = find_dolby_vision_box(&buffer);
        }
        if metadata.nclx.is_some() && metadata.dolby_vision.is_some() {
            break;
        }

        let keep = buffer.len().min(32);
        carry.clear();
        carry.extend_from_slice(&buffer[buffer.len().saturating_sub(keep)..]);
    }

    metadata
}

fn find_dolby_vision_box(bytes: &[u8]) -> Option<DolbyVisionInfo> {
    const MIN_DOLBY_VISION_BOX_SIZE: usize = 12;
    const MAX_DOLBY_VISION_BOX_SIZE: usize = 128;

    for index in 0..bytes.len().saturating_sub(4) {
        let box_type = &bytes[index..index + 4];
        if box_type != b"dvcC" && box_type != b"dvvC" {
            continue;
        }
        if index < 4 {
            continue;
        }

        let box_size = u32::from_be_bytes([
            bytes[index - 4],
            bytes[index - 3],
            bytes[index - 2],
            bytes[index - 1],
        ]) as usize;
        if !(MIN_DOLBY_VISION_BOX_SIZE..=MAX_DOLBY_VISION_BOX_SIZE).contains(&box_size) {
            continue;
        }

        let box_start = index - 4;
        let box_end = box_start.saturating_add(box_size);
        if box_end > bytes.len() || index + 8 > bytes.len() {
            continue;
        }

        let config0 = bytes[index + 6];
        let config1 = bytes[index + 7];
        let profile = (config0 & 0xfe) >> 1;
        let level = ((config0 & 0x01) << 5) | ((config1 & 0xf8) >> 3);
        if profile == 0 || level == 0 {
            continue;
        }
        return Some(DolbyVisionInfo { profile, level });
    }

    None
}

fn find_nclx_box(bytes: &[u8]) -> Option<NclxColorInfo> {
    const MIN_NCLX_SIZE: usize = 19;
    const MAX_REASONABLE_COLR_SIZE: usize = 96;

    for index in 0..bytes.len().saturating_sub(4) {
        if &bytes[index..index + 4] != b"nclx" {
            continue;
        }

        if index < 8 || index + 11 > bytes.len() {
            continue;
        }
        if &bytes[index - 4..index] != b"colr" {
            continue;
        }

        let box_size = u32::from_be_bytes([
            bytes[index - 8],
            bytes[index - 7],
            bytes[index - 6],
            bytes[index - 5],
        ]) as usize;
        if box_size < MIN_NCLX_SIZE || box_size > MAX_REASONABLE_COLR_SIZE {
            continue;
        }
        let box_start = index - 8;
        let box_end = box_start.saturating_add(box_size);
        if box_end > bytes.len() || box_end < index + 11 {
            continue;
        }

        let primaries = u16::from_be_bytes([bytes[index + 4], bytes[index + 5]]);
        let transfer = u16::from_be_bytes([bytes[index + 6], bytes[index + 7]]);
        let matrix = u16::from_be_bytes([bytes[index + 8], bytes[index + 9]]);
        let full_range_byte = bytes[index + 10];
        if (full_range_byte & 0x7f) != 0 {
            continue;
        }
        if primaries > 22 || transfer > 22 || matrix > 14 {
            continue;
        }
        let full_range = (full_range_byte & 0x80) != 0;
        return Some(NclxColorInfo {
            primaries,
            transfer,
            matrix,
            full_range,
        });
    }

    None
}

#[derive(Clone)]
struct PlayerBindings {
    is_playing: Binding<bool>,
    progress_display: Binding<f64>,
    seek_request: Binding<f64>,
    picture_in_picture_request: Binding<u64>,
    duration_seconds: Binding<f64>,
    position_seconds: Binding<f64>,
    is_buffering: Binding<bool>,
    playback_rate: Binding<f32>,
    preserve_pitch: Binding<bool>,
}

impl PlayerBindings {
    fn progress_control_binding(&self) -> Binding<f64> {
        let seek_request = self.seek_request.clone();
        Binding::mapping(
            &self.progress_display,
            |current: f64| current,
            move |display, requested: f64| {
                let clamped = requested.clamp(0.0, 1.0);
                display.set(clamped);
                seek_request.set(clamped);
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
    subtitle_tracks: Vec<RuntimeSubtitleTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeSubtitleSource {
    Sidecar(SubtitleTrack),
    Embedded(EmbeddedSubtitleSourceTrack),
}

#[derive(Debug)]
enum UiUpdate {
    Event(Event),
    Progress(f64),
    SeekRequest(f64),
    Duration(f64),
    Position(f64),
    Buffering(bool),
    Playing(bool),
    Subtitle(String),
    SubtitleTracks(Vec<String>),
}

fn apply_ui_update(
    on_event: &OnEvent,
    player: Option<&PlayerBindings>,
    subtitle: Option<&SubtitleBindings>,
    update: UiUpdate,
) {
    match update {
        UiUpdate::Event(event) => (on_event)(event),
        UiUpdate::Progress(value) => {
            if let Some(player) = player {
                player.progress_display.set(value);
            }
        }
        UiUpdate::SeekRequest(value) => {
            if let Some(player) = player {
                let clamped = value.clamp(0.0, 1.0);
                player.progress_display.set(clamped);
                player.seek_request.set(clamped);
                let duration = player.duration_seconds.get().max(0.0);
                player.position_seconds.set(duration * clamped);
            }
        }
        UiUpdate::Duration(value) => {
            if let Some(player) = player {
                player.duration_seconds.set(value);
            }
        }
        UiUpdate::Position(value) => {
            if let Some(player) = player {
                player.position_seconds.set(value);
            }
        }
        UiUpdate::Buffering(value) => {
            if let Some(player) = player {
                player.is_buffering.set(value);
            }
        }
        UiUpdate::Playing(value) => {
            if let Some(player) = player {
                player.is_playing.set(value);
            }
        }
        UiUpdate::Subtitle(value) => {
            if let Some(subtitle) = subtitle {
                subtitle.text.set(value);
            }
        }
        UiUpdate::SubtitleTracks(value) => {
            if let Some(subtitle) = subtitle {
                subtitle.track_labels.set(value);
            }
        }
    }
}

fn start_ui_update_pump(
    receiver: Receiver<UiUpdate>,
    on_event: OnEvent,
    player: Option<PlayerBindings>,
    subtitle: Option<SubtitleBindings>,
) {
    spawn_local(async move {
        loop {
            let mut disconnected = false;

            loop {
                match receiver.try_recv() {
                    Ok(update) => {
                        apply_ui_update(&on_event, player.as_ref(), subtitle.as_ref(), update)
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }

            if disconnected {
                break;
            }

            native_executor::sleep(Duration::from_millis(8)).await;
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
    Finished,
    Failed(String),
}

pub(crate) fn install_platform_hooks(env: &mut Environment) {
    env.insert_hook::<VideoConfig, AnyView>(|_env, config| {
        let VideoConfig {
            source,
            subtitle_selection,
            has_next,
            has_previous,
            volume,
            playback_rate,
            preserve_pitch,
            aspect_ratio,
            loops,
            playback_policy,
            on_event,
        } = config;

        let on_event: OnEvent = Rc::from(on_event);
        let source = runtime_media_item_signal(source);
        AnyView::new(Dynamic::watch(source, move |item: RuntimeMediaItem| {
            let subtitle_tracks = item.subtitle_tracks;
            let subtitle = SubtitleBindings {
                text: binding(String::new()),
                track_labels: binding(runtime_subtitle_track_labels(&subtitle_tracks)),
            };
            let (ui_updates, ui_receiver) = mpsc::channel();
            start_ui_update_pump(ui_receiver, on_event.clone(), None, Some(subtitle.clone()));
            let surface = VideoSurface::new(
                item.source,
                subtitle_tracks,
                subtitle_selection.clone(),
                has_next.clone(),
                has_previous.clone(),
                volume.clone(),
                playback_rate.clone(),
                preserve_pitch.clone(),
                aspect_ratio,
                loops,
                playback_policy,
                ui_updates,
                None,
            );
            overlay(surface, subtitle_banner(subtitle.text)).alignment(Alignment::Bottom)
        }))
    });

    env.insert_hook::<VideoPlayerConfig, AnyView>(|_env, config| {
        let VideoPlayerConfig {
            source,
            subtitle_selection,
            has_next,
            has_previous,
            volume,
            playback_rate,
            preserve_pitch,
            aspect_ratio,
            show_controls,
            playback_policy,
            on_event,
        } = config;

        let on_event: OnEvent = Rc::from(on_event);
        let source = runtime_media_item_signal(source);
        AnyView::new(Dynamic::watch(source, move |item: RuntimeMediaItem| {
            let subtitle_tracks = item.subtitle_tracks;
            let player = PlayerBindings {
                is_playing: Binding::bool(true),
                progress_display: Binding::f64(0.0),
                seek_request: Binding::f64(0.0),
                picture_in_picture_request: binding(0_u64),
                duration_seconds: Binding::f64(0.0),
                position_seconds: Binding::f64(0.0),
                is_buffering: Binding::bool(false),
                playback_rate: playback_rate.clone(),
                preserve_pitch: preserve_pitch.clone(),
            };
            let subtitle = SubtitleBindings {
                text: binding(String::new()),
                track_labels: binding(runtime_subtitle_track_labels(&subtitle_tracks)),
            };
            let (ui_updates, ui_receiver) = mpsc::channel();
            start_ui_update_pump(
                ui_receiver,
                on_event.clone(),
                Some(player.clone()),
                Some(subtitle.clone()),
            );

            let source = item.source;
            let show_controls = show_controls.clone();
            let subtitle_selection = subtitle_selection.clone();
            let has_previous = has_previous.clone();
            let has_next = has_next.clone();
            let volume = volume.clone();
            let playback_rate = playback_rate.clone();
            let preserve_pitch = preserve_pitch.clone();
            let on_event = on_event.clone();
            let player = player.clone();
            let subtitle = subtitle.clone();
            let subtitle_tracks = subtitle_tracks.clone();
            let ui_updates = ui_updates.clone();

            Dynamic::watch(show_controls, move |show_controls| {
                let surface = VideoSurface::new(
                    source.clone(),
                    subtitle_tracks.clone(),
                    subtitle_selection.clone(),
                    has_next.clone(),
                    has_previous.clone(),
                    volume.clone(),
                    playback_rate.clone(),
                    preserve_pitch.clone(),
                    aspect_ratio,
                    true,
                    playback_policy,
                    ui_updates.clone(),
                    Some(player.clone()),
                );

                if show_controls {
                    let progress = player.progress_control_binding();
                    let controls = player_controls(
                        subtitle.track_labels.clone(),
                        subtitle_selection.clone(),
                        has_previous.clone(),
                        has_next.clone(),
                        player.is_playing.clone(),
                        progress,
                        player.picture_in_picture_request.clone(),
                        player.duration_seconds.clone(),
                        player.position_seconds.clone(),
                        player.is_buffering.clone(),
                        player.playback_rate.clone(),
                        player.preserve_pitch.clone(),
                        volume.clone(),
                        on_event.clone(),
                    );
                    let bottom_cluster =
                        vstack((subtitle_banner(subtitle.text.clone()), controls)).spacing(12.0);
                    AnyView::new(overlay(surface, bottom_cluster).alignment(Alignment::Bottom))
                } else {
                    AnyView::new(
                        overlay(surface, subtitle_banner(subtitle.text.clone()))
                            .alignment(Alignment::Bottom),
                    )
                }
            })
        }))
    });
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
        format!("Embedded {}", language)
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

fn runtime_subtitle_track_labels(tracks: &[RuntimeSubtitleTrack]) -> Vec<String> {
    tracks.iter().map(|track| track.label.clone()).collect()
}

fn runtime_media_item(item: MediaItem) -> RuntimeMediaItem {
    RuntimeMediaItem {
        source: item.source,
        subtitle_tracks: runtime_sidecar_subtitle_tracks(&item.subtitle_tracks),
    }
}

fn runtime_media_item_signal(source: Computed<MediaItem>) -> Computed<RuntimeMediaItem> {
    source.map(runtime_media_item).distinct().computed()
}

fn select_default_subtitle_track_index(tracks: &[RuntimeSubtitleTrack]) -> Option<usize> {
    tracks
        .iter()
        .position(|track| matches!(&track.source, RuntimeSubtitleSource::Sidecar(sidecar) if sidecar.forced))
        .or_else(|| (!tracks.is_empty()).then_some(0))
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
        SubtitleSelection::Auto => Ok(String::from("Subs Auto")),
        SubtitleSelection::Off => Ok(String::from("Subs Off")),
        SubtitleSelection::Track(index) => track_labels
            .get(index)
            .map(|label| format!("Subs {label}"))
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

fn subtitle_banner(subtitle_text: Binding<String>) -> impl View {
    Dynamic::watch(subtitle_text, |current| {
        if current.trim().is_empty() {
            AnyView::new(())
        } else {
            AnyView::new(
                text(current)
                    .footnote()
                    .color(Color::srgb(255, 255, 255))
                    .background_color(Color::srgb(0, 0, 0)),
            )
        }
    })
}

fn picture_in_picture_button(request: Binding<u64>) -> impl View {
    #[cfg(any(target_os = "android", target_os = "ios", target_os = "macos"))]
    {
        With::new(
            button("PiP").action(|State(request): State<Binding<u64>>| {
                request.set(request.get().wrapping_add(1));
            }),
            State(request),
        )
    }

    #[cfg(not(any(target_os = "android", target_os = "ios", target_os = "macos")))]
    {
        let _ = request;
        ()
    }
}

fn player_controls(
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
    on_event: OnEvent,
) -> impl View {
    let volume_level = Binding::mapping(
        &volume,
        |current| f64::from(current.abs().clamp(0.0, 1.0)),
        |volume_binding, level| {
            let magnitude = (level as f32).clamp(0.0, 1.0).max(0.01);
            let muted = volume_binding.get().is_sign_negative();
            volume_binding.set(if muted { -magnitude } else { magnitude });
        },
    );

    let muted = Binding::mapping(
        &volume,
        |current| current.is_sign_negative(),
        |volume_binding, is_muted| {
            let magnitude = volume_binding.get().abs().max(0.1);
            volume_binding.set(if is_muted { -magnitude } else { magnitude });
        },
    );

    let playback_rate_level = Binding::mapping(
        &playback_rate,
        |current| f64::from(clamp_playback_rate(current)),
        |rate_binding, requested| {
            rate_binding.set(clamp_playback_rate(requested as f32));
        },
    );

    let subtitle_toggle = With::new(
        With::new(
            button(Text::display(
                subtitle_track_labels
                    .zip(&subtitle_selection)
                    .map(|(track_labels, selection)| {
                        subtitle_selection_label(&track_labels, selection)
                            .unwrap_or_else(|message| message)
                    }),
            ))
            .action(
                move |State(selection): State<Binding<SubtitleSelection>>,
                      State(track_labels): State<Binding<Vec<String>>>| {
                    let next = next_subtitle_selection(&track_labels.get(), selection.get())
                        .expect("subtitle selection state must resolve");
                    selection.set(next);
                },
            ),
            State(subtitle_selection),
        ),
        State(subtitle_track_labels),
    );

    let previous_button = Dynamic::watch(has_previous.clone(), {
        let on_event = on_event.clone();
        move |enabled| {
            enabled.then({
                let on_event = on_event.clone();
                move || button("Previous").action(move || (on_event)(Event::PreviousRequested))
            })
        }
    });

    let next_button = Dynamic::watch(has_next.clone(), move |enabled| {
        enabled.then({
            let on_event = on_event.clone();
            move || button("Next").action(move || (on_event)(Event::NextRequested))
        })
    });

    let transport = hstack((
        previous_button,
        With::new(
            With::new(
                button("Back 10s").action(
                    |State(value): State<Binding<f64>>, State(duration): State<Binding<f64>>| {
                        let duration = duration.get();
                        if duration <= f64::EPSILON {
                            return;
                        }

                        let delta = (10.0 / duration).min(1.0);
                        value.set((value.get() - delta).max(0.0));
                    },
                ),
                State(progress.clone()),
            ),
            State(duration_seconds.clone()),
        ),
        With::new(
            button(Text::display(
                is_playing
                    .clone()
                    .map(|playing| if playing { "Pause" } else { "Play" }),
            ))
            .action(|State(playing): State<Binding<bool>>| playing.set(!playing.get())),
            State(is_playing.clone()),
        ),
        With::new(
            button(Text::display(
                muted
                    .clone()
                    .map(|is_muted| if is_muted { "Unmute" } else { "Mute" }),
            ))
            .action(|State(is_muted): State<Binding<bool>>| is_muted.set(!is_muted.get())),
            State(muted.clone()),
        ),
        slider(0.0..=1.0, &volume_level),
        picture_in_picture_button(picture_in_picture_request),
        subtitle_toggle,
        With::new(
            With::new(
                button("Forward 10s").action(
                    |State(value): State<Binding<f64>>, State(duration): State<Binding<f64>>| {
                        let duration = duration.get();
                        if duration <= f64::EPSILON {
                            return;
                        }

                        let delta = (10.0 / duration).min(1.0);
                        value.set((value.get() + delta).min(1.0));
                    },
                ),
                State(progress.clone()),
            ),
            State(duration_seconds.clone()),
        ),
        next_button,
    ))
    .spacing(8.0);

    let playback_rate_label = Text::display(
        playback_rate
            .clone()
            .map(|rate| format!("Speed {:.2}x", clamp_playback_rate(rate))),
    )
    .footnote();

    let speed_controls = hstack((
        playback_rate_label,
        With::new(
            button("0.5x").action(|State(rate): State<Binding<f32>>| rate.set(0.5)),
            State(playback_rate.clone()),
        ),
        With::new(
            button("1.0x").action(|State(rate): State<Binding<f32>>| rate.set(1.0)),
            State(playback_rate.clone()),
        ),
        With::new(
            button("1.5x").action(|State(rate): State<Binding<f32>>| rate.set(1.5)),
            State(playback_rate.clone()),
        ),
        With::new(
            button("2.0x").action(|State(rate): State<Binding<f32>>| rate.set(2.0)),
            State(playback_rate),
        ),
        slider(0.25..=2.0, &playback_rate_level),
        With::new(
            button(Text::display(preserve_pitch.clone().map(|enabled| {
                if enabled {
                    "Pitch Lock On"
                } else {
                    "Pitch Lock Off"
                }
            })))
            .action(|State(enabled): State<Binding<bool>>| enabled.set(!enabled.get())),
            State(preserve_pitch),
        ),
    ))
    .spacing(8.0);

    let timeline = Text::display(
        position_seconds
            .zip(&duration_seconds)
            .zip(&is_buffering)
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
    .footnote();

    vstack((
        timeline,
        slider(0.0..=1.0, &progress),
        transport,
        speed_controls,
    ))
    .spacing(8.0)
}

fn format_timestamp(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
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

struct VideoSurface {
    picture_in_picture_host_id: PictureInPictureHostId,
    renderer: VideoRenderer,
}

impl VideoSurface {
    fn new(
        source: Url,
        subtitle_tracks: Vec<RuntimeSubtitleTrack>,
        subtitle_selection: Binding<SubtitleSelection>,
        has_next: Binding<bool>,
        has_previous: Binding<bool>,
        volume: Binding<Volume>,
        playback_rate: Binding<f32>,
        preserve_pitch: Binding<bool>,
        aspect_ratio: AspectRatio,
        loops: bool,
        playback_policy: PlaybackPolicy,
        ui_updates: Sender<UiUpdate>,
        player: Option<PlayerBindings>,
    ) -> Self {
        let picture_in_picture_host_id = new_picture_in_picture_host_id();
        Self {
            picture_in_picture_host_id,
            renderer: VideoRenderer::new(
                picture_in_picture_host_id,
                source,
                subtitle_tracks,
                subtitle_selection,
                has_next,
                has_previous,
                volume,
                playback_rate,
                preserve_pitch,
                aspect_ratio,
                loops,
                playback_policy,
                ui_updates,
                player,
            ),
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
        GpuSurface::new(self.renderer)
            .picture_in_picture_host_id(self.picture_in_picture_host_id.get())
    }
}

#[derive(Debug)]
enum DecoderControl {
    Stop,
    Seek { progress: f64 },
}

#[derive(Debug)]
enum DecoderOutput {
    Opened { duration: Duration },
    Seeked { progress: f64, pts: Duration },
    Frame(DecodedVideoFrame),
    Ended,
    Error(String),
}

struct DecoderWorker {
    updates: Receiver<DecoderOutput>,
    control: Sender<DecoderControl>,
    handle: Option<thread::JoinHandle<()>>,
}

impl DecoderWorker {
    fn spawn(source_path: PathBuf, start_progress: f64) -> Self {
        let (updates_tx, updates_rx) = mpsc::sync_channel(DECODE_FRAME_QUEUE_CAPACITY);
        let (control_tx, control_rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let mut decode = match DecodeState::open(&source_path) {
                Ok(state) => state,
                Err(message) => {
                    let _ = updates_tx.send(DecoderOutput::Error(message));
                    return;
                }
            };

            if start_progress > 0.0
                && let Err(message) = decode.seek_to_progress(start_progress)
            {
                let _ = updates_tx.send(DecoderOutput::Error(message));
                return;
            }

            if updates_tx
                .send(DecoderOutput::Opened {
                    duration: decode.duration(),
                })
                .is_err()
            {
                return;
            }

            let mut pending_seek = None::<f64>;

            'decode_loop: loop {
                loop {
                    match control_rx.try_recv() {
                        Ok(DecoderControl::Stop) | Err(TryRecvError::Disconnected) => return,
                        Ok(DecoderControl::Seek { progress }) => {
                            pending_seek = Some(progress.clamp(0.0, 1.0));
                        }
                        Err(TryRecvError::Empty) => break,
                    }
                }

                if let Some(progress) = pending_seek.take() {
                    match decode.seek_to_progress(progress) {
                        Ok(pts) => {
                            if updates_tx
                                .send(DecoderOutput::Seeked { progress, pts })
                                .is_err()
                            {
                                return;
                            }
                            continue;
                        }
                        Err(message) => {
                            let _ = updates_tx.send(DecoderOutput::Error(message));
                            return;
                        }
                    }
                }

                match decode.next_frame() {
                    Ok(Some(mut frame)) => 'send_frame: loop {
                        match updates_tx.try_send(DecoderOutput::Frame(frame)) {
                            Ok(()) => break 'send_frame,
                            Err(TrySendError::Full(DecoderOutput::Frame(pending))) => {
                                frame = pending;
                                match control_rx.try_recv() {
                                    Ok(DecoderControl::Stop) | Err(TryRecvError::Disconnected) => {
                                        return;
                                    }
                                    Ok(DecoderControl::Seek { progress }) => {
                                        pending_seek = Some(progress.clamp(0.0, 1.0));
                                        continue 'decode_loop;
                                    }
                                    Err(TryRecvError::Empty) => {
                                        thread::sleep(Duration::from_millis(2));
                                    }
                                }
                            }
                            Err(TrySendError::Disconnected(_)) => return,
                            Err(TrySendError::Full(_)) => return,
                        }
                    },
                    Ok(None) => {
                        let _ = updates_tx.send(DecoderOutput::Ended);
                        return;
                    }
                    Err(message) => {
                        let _ = updates_tx.send(DecoderOutput::Error(message));
                        return;
                    }
                }
            }
        });

        Self {
            updates: updates_rx,
            control: control_tx,
            handle: Some(handle),
        }
    }

    fn try_recv(&mut self) -> Result<DecoderOutput, TryRecvError> {
        self.updates.try_recv()
    }

    fn request_seek(&self, progress: f64) {
        let _ = self.control.send(DecoderControl::Seek {
            progress: progress.clamp(0.0, 1.0),
        });
    }

    fn stop(&mut self) {
        let _ = self.control.send(DecoderControl::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for DecoderWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

struct VideoRenderer {
    picture_in_picture_host_id: PictureInPictureHostId,
    source: Url,
    subtitle_tracks: Vec<RuntimeSubtitleTrack>,
    subtitle_selection: Binding<SubtitleSelection>,
    active_subtitle_track: Option<usize>,
    embedded_subtitle_tracks_loaded: bool,
    has_next: Binding<bool>,
    has_previous: Binding<bool>,
    volume: Binding<Volume>,
    playback_rate: Binding<f32>,
    preserve_pitch: Binding<bool>,
    aspect_ratio: AspectRatio,
    loops: bool,
    playback_policy: PlaybackPolicy,
    ui_updates: Sender<UiUpdate>,
    player: Option<PlayerBindings>,
    decode_worker: Option<DecoderWorker>,
    render_pipeline: Option<wgpu::RenderPipeline>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    sampler: Option<wgpu::Sampler>,
    surface_format: Option<wgpu::TextureFormat>,
    color_profile: VideoColorProfile,
    color_profile_initialized: bool,
    color_profile_source_path: Option<PathBuf>,
    color_uniform_buffer: Option<wgpu::Buffer>,
    color_uniform_dirty: bool,
    y_texture: Option<wgpu::Texture>,
    uv_texture: Option<wgpu::Texture>,
    bind_group: Option<wgpu::BindGroup>,
    vertex_buffer: Option<wgpu::Buffer>,
    vertex_layout_key: Option<VertexLayoutKey>,
    pending_frame: Option<DecodedVideoFrame>,
    duration: Duration,
    source_path: Option<PathBuf>,
    audio_player: Option<AudioPlayer>,
    pending_audio_open: Option<PendingAudioOpen>,
    audio_open_generation: u64,
    audio_failed: bool,
    last_audio_retry_at: Option<Instant>,
    last_applied_volume: Option<f32>,
    last_applied_playback_rate: Option<f32>,
    last_applied_preserve_pitch: Option<bool>,
    media_session: Option<MediaSessionState>,
    media_command_poller: Option<MediaCommandPoller>,
    redraw_handle: Option<RedrawHandle>,
    first_frame_presented: bool,
    ready_sent: bool,
    ended_sent: bool,
    is_buffering: bool,
    playback_anchor_pts: Duration,
    playback_anchor_instant: Option<Instant>,
    play_requested: bool,
    pending_play_request_sync: Option<bool>,
    resume_after_audio_focus_gain: bool,
    audio_focus_ducked: bool,
    last_playing_state: bool,
    last_playback_rate: f32,
    last_reported_progress: f64,
    last_handled_picture_in_picture_request: Option<u64>,
    last_picture_in_picture_controller_state: Option<PictureInPictureControllerState>,
    last_picture_in_picture_active: Option<bool>,
    last_handled_seek_request: Option<f64>,
    pending_seek_request_sync: Option<f64>,
    pending_seek_request: Option<f64>,
    seek_inflight: bool,
    last_seek_restart_at: Option<Instant>,
    source_asset: FileAssetState,
    source_error_reported: bool,
    subtitle_asset: Option<FileAssetState>,
    subtitle_error_reported: bool,
    subtitle_cues: Vec<SubtitleCue>,
    last_subtitle_text: Option<String>,
    download_retry_at: Option<Instant>,
    downloaded_bytes: usize,
    download_total_bytes: Option<usize>,
    last_reported_buffer_level_ms: Option<u32>,
    dropped_video_frames: u64,
    last_metrics_report_at: Option<Instant>,
    surface_prefers_hdr_cache: OnceLock<Option<bool>>,
}

struct PendingAudioOpen {
    receiver: Receiver<(u64, Result<AudioPlayer, String>)>,
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

impl VideoRenderer {
    fn new(
        picture_in_picture_host_id: PictureInPictureHostId,
        source: Url,
        subtitle_tracks: Vec<RuntimeSubtitleTrack>,
        subtitle_selection: Binding<SubtitleSelection>,
        has_next: Binding<bool>,
        has_previous: Binding<bool>,
        volume: Binding<Volume>,
        playback_rate: Binding<f32>,
        preserve_pitch: Binding<bool>,
        aspect_ratio: AspectRatio,
        loops: bool,
        playback_policy: PlaybackPolicy,
        ui_updates: Sender<UiUpdate>,
        player: Option<PlayerBindings>,
    ) -> Self {
        let play_requested = player.as_ref().is_none_or(|p| p.is_playing.get());
        let last_playback_rate = clamp_playback_rate(playback_rate.get());

        Self {
            picture_in_picture_host_id,
            source,
            subtitle_tracks,
            subtitle_selection,
            active_subtitle_track: None,
            embedded_subtitle_tracks_loaded: false,
            has_next,
            has_previous,
            volume,
            playback_rate,
            preserve_pitch,
            aspect_ratio,
            loops,
            playback_policy,
            ui_updates,
            player,
            decode_worker: None,
            render_pipeline: None,
            bind_group_layout: None,
            sampler: None,
            surface_format: None,
            color_profile: VideoColorProfile::default(),
            color_profile_initialized: false,
            color_profile_source_path: None,
            color_uniform_buffer: None,
            color_uniform_dirty: true,
            y_texture: None,
            uv_texture: None,
            bind_group: None,
            vertex_buffer: None,
            vertex_layout_key: None,
            pending_frame: None,
            duration: Duration::ZERO,
            source_path: None,
            audio_player: None,
            pending_audio_open: None,
            audio_open_generation: 0,
            audio_failed: false,
            last_audio_retry_at: None,
            last_applied_volume: None,
            last_applied_playback_rate: None,
            last_applied_preserve_pitch: None,
            media_session: None,
            media_command_poller: None,
            redraw_handle: None,
            first_frame_presented: false,
            ready_sent: false,
            ended_sent: false,
            is_buffering: false,
            playback_anchor_pts: Duration::ZERO,
            playback_anchor_instant: None,
            play_requested,
            pending_play_request_sync: None,
            resume_after_audio_focus_gain: false,
            audio_focus_ducked: false,
            last_playing_state: play_requested,
            last_playback_rate,
            last_reported_progress: 0.0,
            last_handled_picture_in_picture_request: Some(0),
            last_picture_in_picture_controller_state: None,
            last_picture_in_picture_active: None,
            last_handled_seek_request: Some(0.0),
            pending_seek_request_sync: None,
            pending_seek_request: None,
            seek_inflight: false,
            last_seek_restart_at: None,
            source_asset: FileAssetState::Unresolved,
            source_error_reported: false,
            subtitle_asset: None,
            subtitle_error_reported: false,
            subtitle_cues: Vec::new(),
            last_subtitle_text: None,
            download_retry_at: None,
            downloaded_bytes: 0,
            download_total_bytes: None,
            last_reported_buffer_level_ms: None,
            dropped_video_frames: 0,
            last_metrics_report_at: None,
            surface_prefers_hdr_cache: OnceLock::new(),
        }
    }

    fn push_ui_update(&self, update: UiUpdate) {
        let _ = self.ui_updates.send(update);
    }

    fn ensure_media_command_poller(&mut self) {
        if self.media_command_poller.is_some() {
            return;
        }
        let Some(redraw_handle) = self.redraw_handle.clone() else {
            return;
        };
        self.media_command_poller = Some(MediaCommandPoller::spawn(redraw_handle));
    }

    fn current_color_uniform(&self) -> VideoColorUniform {
        let target_mode = self
            .surface_format
            .map(|format| shader_target_mode(format, self.color_profile.hdr))
            .unwrap_or(ShaderTargetMode::LinearSdr);

        VideoColorUniform {
            matrix_mode: self.color_profile.matrix as u32,
            range_mode: self.color_profile.range as u32,
            primaries_mode: self.color_profile.primaries as u32,
            transfer_mode: self.color_profile.transfer as u32,
            target_mode: target_mode as u32,
            _padding0: 0,
            _padding1: 0,
            _padding2: 0,
        }
    }

    fn update_color_profile(&mut self, profile: VideoColorProfile, source_path: &Path) {
        if self.color_profile_initialized && self.color_profile == profile {
            return;
        }

        tracing::info!(
            "[VideoFallback] color profile source={} matrix={:?} primaries={:?} range={:?} transfer={:?} hdr={} wide_gamut={}",
            source_path.display(),
            profile.matrix,
            profile.primaries,
            profile.range,
            profile.transfer,
            profile.hdr,
            profile.wide_gamut,
        );
        self.color_profile = profile;
        self.color_profile_initialized = true;
        self.color_uniform_dirty = true;
    }

    fn upload_color_uniform_if_needed(&mut self, queue: &wgpu::Queue) {
        if !self.color_uniform_dirty {
            return;
        }

        let Some(buffer) = self.color_uniform_buffer.as_ref() else {
            return;
        };

        let bytes = color_uniform_bytes(self.current_color_uniform());
        queue.write_buffer(buffer, 0, &bytes);
        self.color_uniform_dirty = false;
    }

    fn ensure_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if self.render_pipeline.is_some() {
            return;
        }
        self.surface_format = Some(format);
        self.color_uniform_dirty = true;
        tracing::info!("[VideoFallback] create render pipeline surface_format={format:?}");

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Video render shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/video_render.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Video bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            std::num::NonZeroU64::new(32)
                                .expect("color uniform min binding size is non-zero"),
                        ),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Video pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Video render pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: (4 * core::mem::size_of::<f32>()) as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: (2 * core::mem::size_of::<f32>()) as u64,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                    ],
                }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
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
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Video sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let color_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Video color uniform"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        {
            let bytes = color_uniform_bytes(self.current_color_uniform());
            let mut mapped = color_uniform_buffer.slice(..).get_mapped_range_mut();
            mapped.copy_from_slice(&bytes);
        }
        color_uniform_buffer.unmap();

        self.bind_group_layout = Some(bind_group_layout);
        self.render_pipeline = Some(render_pipeline);
        self.sampler = Some(sampler);
        self.color_uniform_buffer = Some(color_uniform_buffer);
    }

    fn should_poll_source(&self) -> bool {
        matches!(
            self.source_asset,
            FileAssetState::Unresolved | FileAssetState::Downloading { .. }
        )
    }

    fn is_source_downloading(&self) -> bool {
        matches!(self.source_asset, FileAssetState::Downloading { .. })
    }

    fn poll_source_download_updates(&mut self) {
        if !self.is_source_downloading() {
            return;
        }

        if let Err(message) = self.resolve_source_path() {
            if !self.source_error_reported {
                self.emit_event(Event::Error {
                    message: message.clone(),
                });
                self.source_error_reported = true;
            }
            self.set_buffering(false);
        }
    }

    fn resolve_source_path(&mut self) -> Result<Option<PathBuf>, String> {
        loop {
            match &mut self.source_asset {
                FileAssetState::Unresolved => {
                    if is_remote_url(&self.source) {
                        let cache_path = cached_video_path(&self.source);
                        if cache_path.exists() {
                            tracing::info!(
                                "[VideoFallback] using cached source: {}",
                                cache_path.display()
                            );
                            self.source_asset = FileAssetState::Ready(cache_path.clone());
                            self.downloaded_bytes = fs::metadata(&cache_path)
                                .ok()
                                .map_or(0, |meta| meta.len() as usize);
                            self.download_total_bytes = Some(self.downloaded_bytes);
                            return Ok(Some(cache_path));
                        }

                        tracing::info!(
                            "[VideoFallback] starting download: {}",
                            self.source.as_str()
                        );
                        self.downloaded_bytes = 0;
                        self.download_total_bytes = None;
                        self.source_asset = FileAssetState::Downloading {
                            path: cache_path.clone(),
                            receiver: start_asset_download(
                                self.source.as_str().to_owned(),
                                cache_path,
                            ),
                            ready: false,
                        };
                        return Ok(None);
                    }

                    let local_path = local_source_path(&self.source);
                    self.source_asset = FileAssetState::Ready(local_path.clone());
                    self.downloaded_bytes = fs::metadata(&local_path)
                        .ok()
                        .map_or(0, |meta| meta.len() as usize);
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
                        self.downloaded_bytes = bytes_written;
                        self.download_total_bytes = total_bytes;
                        continue;
                    }
                    Ok(DownloadUpdate::Ready) => {
                        tracing::info!(
                            "[VideoFallback] download became playable: {}",
                            path.display()
                        );
                        *ready = true;
                        return Ok(Some(path.clone()));
                    }
                    Ok(DownloadUpdate::Finished) => {
                        tracing::info!("[VideoFallback] download finished: {}", path.display());
                        // Retry audio open once the file is complete. Early attempts on
                        // partial downloads may fail when container metadata isn't ready.
                        self.audio_failed = false;
                        self.downloaded_bytes = fs::metadata(path.as_path())
                            .ok()
                            .map_or(0, |meta| meta.len() as usize);
                        self.download_total_bytes = Some(self.downloaded_bytes);
                        let resolved = path.clone();
                        self.source_asset = FileAssetState::Ready(resolved.clone());
                        return Ok(Some(resolved));
                    }
                    Ok(DownloadUpdate::Failed(message)) => {
                        tracing::warn!("[VideoFallback] download failed: {message}");
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
                        return if *ready {
                            self.downloaded_bytes = fs::metadata(path.as_path())
                                .ok()
                                .map_or(0, |meta| meta.len() as usize);
                            self.download_total_bytes = Some(self.downloaded_bytes);
                            let resolved = path.clone();
                            self.source_asset = FileAssetState::Ready(resolved.clone());
                            Ok(Some(resolved))
                        } else {
                            let message = String::from("Video download channel disconnected");
                            self.source_asset = FileAssetState::Failed(message.clone());
                            Err(message)
                        };
                    }
                },
                FileAssetState::Ready(path) => return Ok(Some(path.clone())),
                FileAssetState::Failed(message) => return Err(message.clone()),
            }
        }
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
        if self.embedded_subtitle_tracks_loaded {
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
            self.push_ui_update(UiUpdate::SubtitleTracks(runtime_subtitle_track_labels(
                &self.subtitle_tracks,
            )));
        }
        self.embedded_subtitle_tracks_loaded = true;
        Ok(())
    }

    fn sync_selected_subtitle_track(&mut self) -> Result<(), String> {
        self.ensure_embedded_subtitle_tracks()?;
        let selection = self.subtitle_selection.get();
        let next = match resolve_selected_subtitle_index(&self.subtitle_tracks, selection) {
            Ok(next) => next,
            Err(message)
                if !self.embedded_subtitle_tracks_loaded
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
        self.subtitle_error_reported = false;
        self.subtitle_cues.clear();
        self.set_subtitle_text(None);
        Ok(())
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
            RuntimeSubtitleSource::Embedded(embedded_track) => {
                let source_path = if let Some(path) = self.source_path.clone() {
                    path
                } else {
                    match self.resolve_source_path()? {
                        Some(path) => path,
                        None => return Ok(()),
                    }
                };
                self.subtitle_cues =
                    read_embedded_subtitle_cues(&source_path, embedded_track.track_id)
                        .map_err(|error| error.to_string())?
                        .into_iter()
                        .map(|cue| SubtitleCue {
                            start: cue.start,
                            end: cue.end,
                            text: cue.text,
                        })
                        .collect();
                return Ok(());
            }
            RuntimeSubtitleSource::Sidecar(sidecar_track) => {
                let subtitle_source = sidecar_track.source.clone();
                let Some(asset) = self.subtitle_asset.as_mut() else {
                    return Ok(());
                };

                loop {
                    match asset {
                        FileAssetState::Unresolved => {
                            if is_remote_url(&subtitle_source) {
                                let cache_path = cached_subtitle_path(&subtitle_source);
                                if cache_path.exists() {
                                    self.subtitle_cues = parse_subtitles_from_path(&cache_path)?;
                                    *asset = FileAssetState::Ready(cache_path);
                                    return Ok(());
                                }

                                *asset = FileAssetState::Downloading {
                                    path: cache_path.clone(),
                                    receiver: start_asset_download(
                                        subtitle_source.as_str().to_owned(),
                                        cache_path,
                                    ),
                                    ready: false,
                                };
                                return Ok(());
                            }

                            let local_path = local_source_path(&subtitle_source);
                            self.subtitle_cues = parse_subtitles_from_path(&local_path)?;
                            *asset = FileAssetState::Ready(local_path);
                            return Ok(());
                        }
                        FileAssetState::Downloading { path, receiver, .. } => match receiver
                            .try_recv()
                        {
                            Ok(DownloadUpdate::Progress { .. } | DownloadUpdate::Ready) => continue,
                            Ok(DownloadUpdate::Finished) => {
                                self.subtitle_cues = parse_subtitles_from_path(path)?;
                                *asset = FileAssetState::Ready(path.clone());
                                return Ok(());
                            }
                            Ok(DownloadUpdate::Failed(message)) => {
                                let message = format!("subtitle download failed: {message}");
                                *asset = FileAssetState::Failed(message.clone());
                                return Err(message);
                            }
                            Err(TryRecvError::Empty) => return Ok(()),
                            Err(TryRecvError::Disconnected) => {
                                self.subtitle_cues = parse_subtitles_from_path(path)?;
                                *asset = FileAssetState::Ready(path.clone());
                                return Ok(());
                            }
                        },
                        FileAssetState::Ready(_) => return Ok(()),
                        FileAssetState::Failed(message) => return Err(message.clone()),
                    }
                }
            }
        }
    }

    fn sync_subtitle_text(&mut self, now: Instant) {
        let next = active_subtitle_text(&self.subtitle_cues, self.playback_position(now))
            .map(ToOwned::to_owned)
            .filter(|text| !text.trim().is_empty());
        self.set_subtitle_text(next);
    }

    fn stop_decode_worker(&mut self) {
        if let Some(mut worker) = self.decode_worker.take() {
            worker.stop();
        }
    }

    fn start_audio_player_open(&mut self, source_path: &Path) {
        if let Some(player) = self.audio_player.take() {
            player.stop();
        }
        self.pending_audio_open = None;
        self.audio_open_generation = self.audio_open_generation.wrapping_add(1);
        let generation = self.audio_open_generation;

        let now = Instant::now();
        self.audio_failed = false;
        self.last_audio_retry_at = Some(now);
        self.last_applied_volume = None;
        self.last_applied_playback_rate = None;
        self.last_applied_preserve_pitch = None;

        let source_path = source_path.to_path_buf();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let open_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                AudioPlayer::open(&source_path).map_err(|error| error.to_string())
            }))
            .unwrap_or_else(|_| Err(String::from("Audio player initialization panicked")));
            let _ = sender.send((generation, open_result));
        });
        self.pending_audio_open = Some(PendingAudioOpen { receiver });
    }

    fn poll_pending_audio_open(&mut self) {
        let Some(pending) = self.pending_audio_open.as_ref() else {
            return;
        };
        let open_result = match pending.receiver.try_recv() {
            Ok((generation, result)) => {
                self.pending_audio_open = None;
                if generation != self.audio_open_generation {
                    return;
                }
                result
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.pending_audio_open = None;
                self.on_audio_player_open_failed(String::from(
                    "Audio player initialization channel disconnected",
                ));
                return;
            }
        };

        match open_result {
            Ok(player) => self.on_audio_player_ready(player),
            Err(error) => self.on_audio_player_open_failed(error),
        }
    }

    fn on_audio_player_ready(&mut self, player: AudioPlayer) {
        tracing::info!("[VideoFallback] audio player ready");
        self.audio_player = Some(player);
        self.pending_audio_open = None;
        if let Some(audio) = self.audio_player.as_ref() {
            tracing::info!(
                metadata = ?audio.metadata(),
                duration = ?audio.duration(),
                "[VideoFallback] audio player metadata ready"
            );
        }
        self.audio_failed = false;
        self.last_applied_volume = None;
        self.last_applied_playback_rate = None;
        self.last_applied_preserve_pitch = None;
        self.sync_audio_volume();
        self.sync_audio_playback_params();
        let should_play = self.should_play();
        let target_position = self.playback_position(Instant::now());
        self.seek_audio_to(target_position, should_play);
    }

    fn on_audio_player_open_failed(&mut self, error: String) {
        tracing::warn!("[VideoFallback] audio player open failed: {error}");
        self.audio_player = None;
        self.audio_failed = true;
        self.last_applied_volume = None;
        self.last_applied_playback_rate = None;
        self.last_applied_preserve_pitch = None;
        self.emit_event(Event::Error {
            message: format!("Audio playback unavailable: {error}"),
        });
    }

    fn ensure_audio_player(&mut self, source_path: &Path) {
        let source_changed = self
            .source_path
            .as_ref()
            .is_some_and(|existing| existing.as_path() != source_path);

        if source_changed {
            self.audio_failed = false;
            self.start_audio_player_open(source_path);
            return;
        }

        if self.audio_player.is_none() && self.pending_audio_open.is_none() && !self.audio_failed {
            self.start_audio_player_open(source_path);
        } else {
            self.sync_audio_volume();
            self.sync_audio_playback_params();
            self.sync_audio_playback(self.should_play());
        }
    }

    fn retry_audio_player_if_needed(&mut self) {
        if !self.audio_failed || self.audio_player.is_some() || self.pending_audio_open.is_some() {
            return;
        }

        let source_ready = match &self.source_asset {
            FileAssetState::Ready(_) => true,
            FileAssetState::Downloading { ready, .. } => *ready,
            FileAssetState::Unresolved | FileAssetState::Failed(_) => false,
        };
        if !source_ready {
            return;
        }

        let Some(source_path) = self.source_path.clone() else {
            return;
        };

        let now = Instant::now();
        if self
            .last_audio_retry_at
            .is_some_and(|last| now.saturating_duration_since(last) < Duration::from_millis(900))
        {
            return;
        }

        self.audio_failed = false;
        self.start_audio_player_open(&source_path);
    }

    fn start_decode_worker(&mut self, source_path: PathBuf, start_progress: f64) {
        let playback_source_changed = self
            .source_path
            .as_ref()
            .is_none_or(|path| path != &source_path);
        let source_changed = self
            .color_profile_source_path
            .as_ref()
            .is_none_or(|path| path != &source_path);
        if source_changed {
            let height_hint = self.y_texture.as_ref().map(wgpu::Texture::height);
            let color_profile = probe_video_color_profile(&source_path, height_hint);
            self.update_color_profile(color_profile, &source_path);
            self.color_profile_source_path = Some(source_path.clone());
        }

        tracing::info!(
            "[VideoFallback] start decoder worker source={} progress={start_progress:.3}",
            source_path.display()
        );
        if playback_source_changed {
            self.dropped_video_frames = 0;
        }
        self.stop_decode_worker();
        self.decode_worker = Some(DecoderWorker::spawn(source_path.clone(), start_progress));
        self.ensure_audio_player(&source_path);
        self.source_path = Some(source_path.clone());
        self.pending_frame = None;
        self.first_frame_presented = false;
        self.ended_sent = false;
        self.last_handled_seek_request = self.player.as_ref().map(|p| p.seek_request.get());
        self.pending_seek_request = None;
        self.seek_inflight = false;
        self.last_seek_restart_at = None;
        self.download_retry_at = None;
        self.last_audio_retry_at = None;
        self.last_reported_buffer_level_ms = None;
        self.last_metrics_report_at = None;
        self.set_buffering(true);
    }

    fn restart_decoder_from_progress(&mut self, progress: f64) -> Result<(), String> {
        let source_path = if let Some(path) = self.source_path.clone() {
            path
        } else {
            match self.resolve_source_path()? {
                Some(path) => path,
                None => return Ok(()),
            }
        };

        self.start_decode_worker(source_path, progress.clamp(0.0, 1.0));
        Ok(())
    }

    fn open_decode_state(&mut self) {
        let source_path = match self.resolve_source_path() {
            Ok(Some(path)) => path,
            Ok(None) => {
                self.set_buffering(true);
                return;
            }
            Err(message) => {
                if !self.source_error_reported {
                    self.emit_event(Event::Error {
                        message: message.clone(),
                    });
                    self.source_error_reported = true;
                }
                self.set_buffering(false);
                return;
            }
        };

        let restart_from_beginning = self.ended_sent && self.last_reported_progress >= 0.999;
        let start_progress = if restart_from_beginning {
            0.0
        } else {
            self.last_reported_progress.clamp(0.0, 1.0)
        };
        if restart_from_beginning {
            self.last_reported_progress = 0.0;
            self.playback_anchor_pts = Duration::ZERO;
            self.playback_anchor_instant = None;
            self.ended_sent = false;
        }

        self.start_decode_worker(source_path, start_progress);
        self.source_error_reported = false;
    }

    fn reconcile_play_request_from_ui(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };

        let ui_playing = player.is_playing.get();
        let user_initiated_change =
            self.pending_play_request_sync.is_none() && ui_playing != self.play_requested;
        if let Some(pending) = self.pending_play_request_sync {
            if ui_playing == pending {
                self.pending_play_request_sync = None;
            } else {
                return;
            }
        }

        if user_initiated_change {
            self.resume_after_audio_focus_gain = false;
            self.set_audio_focus_ducked(false);
        }

        self.play_requested = ui_playing;
    }

    fn set_play_requested(&mut self, playing: bool) {
        self.play_requested = playing;
        if self.player.is_some() {
            self.pending_play_request_sync = Some(playing);
            self.push_ui_update(UiUpdate::Playing(playing));
        }
    }

    fn set_audio_focus_ducked(&mut self, ducked: bool) {
        if self.audio_focus_ducked == ducked {
            return;
        }

        self.audio_focus_ducked = ducked;
        self.last_applied_volume = None;
        self.sync_audio_volume();
    }

    fn queue_seek_request(&mut self, progress: f64, sync_player: bool) {
        let requested = progress.clamp(0.0, 1.0);
        self.last_handled_seek_request = Some(requested);
        self.pending_seek_request = Some(requested);
        if sync_player && self.player.is_some() {
            self.pending_seek_request_sync = Some(requested);
            self.push_ui_update(UiUpdate::SeekRequest(requested));
        }
    }

    fn queue_navigation_controls(&self) -> QueueNavigationControls {
        QueueNavigationControls::disabled()
            .with_next_enabled(self.has_next.get())
            .with_previous_enabled(self.has_previous.get())
    }

    fn handle_media_command(&mut self, command: MediaCommand) {
        let now = Instant::now();
        match command {
            MediaCommand::Play => {
                self.resume_after_audio_focus_gain = false;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(true);
            }
            MediaCommand::Pause => {
                self.resume_after_audio_focus_gain = false;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(false);
            }
            MediaCommand::PlayPause => {
                self.resume_after_audio_focus_gain = false;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(!self.play_requested);
            }
            MediaCommand::Stop => {
                self.resume_after_audio_focus_gain = false;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(false);
                self.queue_seek_request(0.0, true);
            }
            MediaCommand::Seek(position) => {
                self.queue_seek_request(progress_for_position(self.duration, position), true);
            }
            MediaCommand::SeekForward(delta) => {
                let target = self.playback_position(now).saturating_add(delta);
                self.queue_seek_request(progress_for_position(self.duration, target), true);
            }
            MediaCommand::SeekBackward(delta) => {
                let target = self.playback_position(now).saturating_sub(delta);
                self.queue_seek_request(progress_for_position(self.duration, target), true);
            }
            MediaCommand::AudioFocusGained => {
                self.set_audio_focus_ducked(false);
                if self.resume_after_audio_focus_gain {
                    self.resume_after_audio_focus_gain = false;
                    self.set_play_requested(true);
                }
            }
            MediaCommand::AudioFocusLost => {
                self.resume_after_audio_focus_gain = false;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(false);
            }
            MediaCommand::AudioFocusLostTransient => {
                self.resume_after_audio_focus_gain = self.play_requested;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(false);
            }
            MediaCommand::AudioFocusLostDuck => {
                self.resume_after_audio_focus_gain = false;
                self.set_audio_focus_ducked(true);
            }
            MediaCommand::AudioBecomingNoisy => {
                self.resume_after_audio_focus_gain = false;
                self.set_audio_focus_ducked(false);
                self.set_play_requested(false);
            }
            MediaCommand::Next => {
                self.emit_event(Event::NextRequested);
            }
            MediaCommand::Previous => {
                self.emit_event(Event::PreviousRequested);
            }
            _ => {
                tracing::warn!("[VideoFallback] ignoring unsupported media command {command:?}");
            }
        }
    }

    fn poll_media_commands(&mut self) {
        loop {
            let command = self
                .media_session
                .as_ref()
                .and_then(MediaSessionState::poll_command);
            let Some(command) = command else {
                break;
            };
            self.handle_media_command(command);
        }
    }

    fn poll_picture_in_picture_commands(&mut self) {
        loop {
            let command = match poll_picture_in_picture_command(self.picture_in_picture_host_id) {
                Ok(command) => command,
                Err(error) => {
                    self.emit_event(Event::Error {
                        message: error.to_string(),
                    });
                    break;
                }
            };
            let Some(command) = command else {
                break;
            };

            match command {
                PictureInPictureCommand::Play => self.handle_media_command(MediaCommand::Play),
                PictureInPictureCommand::Pause => self.handle_media_command(MediaCommand::Pause),
                PictureInPictureCommand::SeekForward(delta) => {
                    self.handle_media_command(MediaCommand::SeekForward(delta));
                }
                PictureInPictureCommand::SeekBackward(delta) => {
                    self.handle_media_command(MediaCommand::SeekBackward(delta));
                }
            }
        }
    }

    fn should_open_decode_worker(&self) -> bool {
        self.source_path.is_none() || self.play_requested || self.pending_seek_request.is_some()
    }

    fn should_play(&self) -> bool {
        self.play_requested
    }

    fn requested_playback_rate(&self) -> f32 {
        clamp_playback_rate(self.playback_rate.get())
    }

    fn requested_preserve_pitch(&self) -> bool {
        self.preserve_pitch.get()
    }

    fn is_realtime_policy(&self) -> bool {
        self.playback_policy.realtime
    }

    fn live_drop_threshold(&self) -> Duration {
        Duration::from_millis(u64::from(
            self.playback_policy.live_max_video_late_ms.max(1),
        ))
    }

    fn estimated_buffered_ahead(&self, now: Instant) -> Duration {
        let playback = self.playback_position(now);
        if self.duration.is_zero() {
            return Duration::ZERO;
        }

        if self.is_source_downloading()
            && let Some(total) = self.download_total_bytes
        {
            if total > 0 {
                let ratio = (self.downloaded_bytes as f64 / total as f64).clamp(0.0, 1.0);
                let downloaded_duration = self.duration.mul_f64(ratio);
                return downloaded_duration.saturating_sub(playback);
            }
        }

        if let Some(frame) = self.pending_frame.as_ref() {
            return frame.pts.saturating_sub(playback);
        }

        if matches!(self.source_asset, FileAssetState::Ready(_)) {
            return self.duration.saturating_sub(playback);
        }

        Duration::ZERO
    }

    fn estimated_buffered_ahead_ms(&self, now: Instant) -> u32 {
        self.estimated_buffered_ahead(now)
            .as_millis()
            .min(u128::from(u32::MAX)) as u32
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
        match self.playback_anchor_instant {
            Some(anchor) => self.playback_anchor_pts.saturating_add(
                now.saturating_duration_since(anchor)
                    .mul_f64(f64::from(self.last_playback_rate)),
            ),
            None => self.playback_anchor_pts,
        }
    }

    fn av_drift_ms(&self, now: Instant) -> f32 {
        let Some(audio) = self.audio_player.as_ref() else {
            return 0.0;
        };
        let audio_position = audio.position();
        let video_position = self.video_timeline_position(now);
        let delta = audio_position.as_secs_f64() - video_position.as_secs_f64();
        (delta * 1000.0) as f32
    }

    fn maybe_emit_playback_metrics(&mut self, now: Instant) {
        if !self.should_play() || !self.first_frame_presented {
            return;
        }
        if self
            .last_metrics_report_at
            .is_some_and(|last| now.saturating_duration_since(last) < METRICS_REPORT_INTERVAL)
        {
            return;
        }

        self.last_metrics_report_at = Some(now);
        self.emit_event(Event::PlaybackMetrics {
            av_drift_ms: self.av_drift_ms(now),
            dropped_video_frames: self.dropped_video_frames,
        });
    }

    fn should_wait_for_vod_buffer(&self, now: Instant) -> bool {
        should_wait_for_vod_buffering(
            self.playback_policy,
            self.is_source_downloading(),
            self.download_total_bytes.is_some(),
            self.first_frame_presented,
            self.estimated_buffered_ahead_ms(now),
        )
    }

    fn should_enter_vod_stall_buffering(&self, now: Instant) -> bool {
        should_enter_vod_stall_buffering(
            self.playback_policy,
            self.is_source_downloading(),
            self.download_total_bytes.is_some(),
            self.first_frame_presented,
            self.estimated_buffered_ahead_ms(now),
        )
    }

    fn playback_position(&self, now: Instant) -> Duration {
        if self.last_playing_state
            && let Some(audio) = self.audio_player.as_ref()
        {
            let audio_position = audio.position();
            if audio_position > Duration::ZERO || self.playback_anchor_pts == Duration::ZERO {
                return audio_position;
            }
        }

        match self.playback_anchor_instant {
            Some(anchor) => self.playback_anchor_pts.saturating_add(
                now.saturating_duration_since(anchor)
                    .mul_f64(f64::from(self.last_playback_rate)),
            ),
            None => self.playback_anchor_pts,
        }
    }

    fn set_playback_position(&mut self, pts: Duration, should_play: bool) {
        self.playback_anchor_pts = pts;
        self.playback_anchor_instant = should_play.then(Instant::now);
        self.last_playing_state = should_play;
        if self.player.is_some() {
            self.push_ui_update(UiUpdate::Position(pts.as_secs_f64()));
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
        let hold_audio_focus = should_play || self.resume_after_audio_focus_gain;
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
        if self.is_buffering == buffering {
            return;
        }

        self.is_buffering = buffering;
        if self.player.is_some() {
            self.push_ui_update(UiUpdate::Buffering(buffering));
        }

        self.emit_event(if buffering {
            Event::Buffering
        } else {
            Event::BufferingEnded
        });
        self.maybe_emit_buffer_level(Instant::now());
    }

    fn sync_audio_volume(&mut self) {
        let Some(audio) = self.audio_player.as_ref() else {
            return;
        };

        let volume = effective_audio_volume(self.volume.get(), self.audio_focus_ducked);

        if self
            .last_applied_volume
            .is_some_and(|last| (last - volume).abs() <= 0.01)
        {
            return;
        }

        audio.set_volume(volume);
        self.last_applied_volume = Some(volume);
    }

    fn sync_audio_playback_params(&mut self) {
        if self.audio_player.is_none() {
            return;
        }

        let rate = self.requested_playback_rate();
        let preserve_pitch = self.requested_preserve_pitch();
        let rate_changed = self
            .last_applied_playback_rate
            .is_none_or(|last| (last - rate).abs() > 0.001);
        let preserve_pitch_changed = self
            .last_applied_preserve_pitch
            .is_none_or(|last| last != preserve_pitch);
        if !rate_changed && !preserve_pitch_changed {
            return;
        }

        if (rate - 1.0).abs() > 0.001 || !preserve_pitch {
            tracing::warn!(
                requested_rate = rate,
                preserve_pitch,
                "[VideoFallback] waterkit-audio no longer exposes playback-rate or pitch controls; audio remains at native speed"
            );
        }

        self.last_applied_playback_rate = Some(rate);
        self.last_applied_preserve_pitch = Some(preserve_pitch);
    }

    fn sync_audio_playback(&self, should_play: bool) {
        let Some(audio) = self.audio_player.as_ref() else {
            return;
        };

        if should_play {
            audio.play();
        } else {
            audio.pause();
        }
    }

    fn seek_audio_to(&self, pts: Duration, should_play: bool) {
        let Some(audio) = self.audio_player.as_ref() else {
            return;
        };

        audio.seek(pts);
        if should_play {
            audio.play();
        } else {
            audio.pause();
        }
    }

    fn drain_decoder_outputs(&mut self, should_play: bool) {
        loop {
            if self.pending_frame.is_some() {
                // If render is lagging behind real playback time, drop stale pending frame
                // and pull a newer one from the decode queue.
                let late_by = self.pending_frame.as_ref().map_or(Duration::ZERO, |frame| {
                    self.playback_position(Instant::now())
                        .saturating_sub(frame.pts)
                });
                let late_frame_threshold = if self.is_realtime_policy() {
                    self.live_drop_threshold()
                } else {
                    VOD_FRAME_DROP_THRESHOLD
                };
                if should_play && late_by > late_frame_threshold {
                    tracing::warn!(
                        "[VideoFallback] dropping stale frame late_by_ms={} progress={:.3}",
                        late_by.as_millis(),
                        self.last_reported_progress
                    );
                    self.dropped_video_frames = self.dropped_video_frames.saturating_add(1);
                    self.pending_frame = None;
                } else {
                    // Keep decoder backpressured while waiting to present the current frame.
                    // This prevents drifting far ahead of the playback clock.
                    break;
                }
            }

            let output = {
                let Some(worker) = self.decode_worker.as_mut() else {
                    return;
                };

                match worker.try_recv() {
                    Ok(output) => Some(output),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => Some(DecoderOutput::Error(String::from(
                        "Decoder worker disconnected",
                    ))),
                }
            };

            let Some(output) = output else {
                break;
            };

            match output {
                DecoderOutput::Opened { duration } => {
                    tracing::info!(
                        "[VideoFallback] decoder opened duration={:.3}s",
                        duration.as_secs_f64()
                    );
                    self.duration = duration;
                    self.update_ui_progress();
                    if self.media_session.is_none() {
                        self.media_session = MediaSessionState::new(&self.source, duration);
                        if self.media_session.is_some() {
                            self.ensure_media_command_poller();
                        }
                    }
                    if !self.ready_sent {
                        self.emit_event(Event::ReadyToPlay);
                        self.ready_sent = true;
                    }
                    self.sync_picture_in_picture_controller(should_play);
                }
                DecoderOutput::Seeked { progress, pts } => {
                    self.seek_inflight = false;
                    self.pending_frame = None;
                    self.ended_sent = false;
                    self.last_reported_progress = progress;
                    self.set_playback_position(pts, should_play);
                    self.seek_audio_to(pts, should_play);
                    self.sync_media_session(should_play);
                    self.update_ui_progress();
                }
                DecoderOutput::Frame(decoded) => {
                    if self.seek_inflight {
                        continue;
                    }
                    self.pending_frame = Some(decoded);
                    self.ended_sent = false;
                    self.download_retry_at = None;
                    break;
                }
                DecoderOutput::Ended => {
                    if self.seek_inflight {
                        continue;
                    }
                    tracing::info!("[VideoFallback] decoder reached end");
                    if self.is_source_downloading() {
                        let now = Instant::now();
                        if !self.is_realtime_policy() {
                            self.set_buffering(true);
                            self.maybe_emit_buffer_level(now);
                            let buffered_ms = self.estimated_buffered_ahead_ms(now);
                            if buffered_ms < self.playback_policy.vod_resume_buffer_ms {
                                return;
                            }
                        } else {
                            self.set_buffering(false);
                        }

                        if self.download_retry_at.is_none_or(|next| now >= next) {
                            self.download_retry_at = Some(now + Duration::from_millis(150));
                            if let Err(message) =
                                self.restart_decoder_from_progress(self.last_reported_progress)
                            {
                                self.emit_event(Event::Error { message });
                                self.stop_decode_worker();
                                self.set_buffering(false);
                            }
                        }
                        return;
                    }

                    self.set_buffering(false);
                    if !self.ended_sent {
                        self.emit_event(Event::Ended);
                        self.ended_sent = true;
                    }

                    if self.loops {
                        if let Err(message) = self.restart_decoder_from_progress(0.0) {
                            self.emit_event(Event::Error { message });
                            self.stop_decode_worker();
                            self.set_buffering(false);
                            return;
                        }
                        self.last_reported_progress = 0.0;
                        self.set_playback_position(Duration::ZERO, should_play);
                        self.seek_audio_to(Duration::ZERO, should_play);
                        self.update_ui_progress();
                    } else {
                        self.set_play_requested(false);
                        self.stop_decode_worker();
                        self.set_playback_position(self.duration, false);
                        self.sync_audio_playback(false);
                        self.sync_media_session(false);
                    }
                }
                DecoderOutput::Error(message) => {
                    tracing::warn!("[VideoFallback] decoder error: {message}");
                    self.seek_inflight = false;
                    self.emit_event(Event::Error { message });
                    self.stop_decode_worker();
                    self.pending_frame = None;
                    self.set_buffering(false);
                    return;
                }
            }
        }
    }

    fn maybe_seek_from_ui(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };

        let requested = player.seek_request.get().clamp(0.0, 1.0);
        if let Some(pending) = self.pending_seek_request_sync {
            if (requested - pending).abs() <= SEEK_EPSILON {
                self.pending_seek_request_sync = None;
            } else {
                return;
            }
        }

        if self
            .last_handled_seek_request
            .is_some_and(|last| (requested - last).abs() <= SEEK_EPSILON)
        {
            return;
        }
        if (requested - self.last_reported_progress).abs() <= SEEK_EPSILON {
            self.last_handled_seek_request = Some(requested);
            self.pending_seek_request = None;
            return;
        }

        self.queue_seek_request(requested, false);
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
        if let Err(error) = enter_picture_in_picture(self.picture_in_picture_host_id, aspect_ratio)
        {
            self.emit_event(Event::Error {
                message: error.to_string(),
            });
        }
    }

    fn picture_in_picture_controller_state(
        &self,
        should_play: bool,
    ) -> PictureInPictureControllerState {
        let active = self.player.is_some() && (self.ready_sent || self.first_frame_presented);
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

        if let Err(error) = sync_picture_in_picture_controller(state) {
            self.emit_event(Event::Error {
                message: error.to_string(),
            });
        }
    }

    fn clear_picture_in_picture_controller(&mut self) {
        self.last_picture_in_picture_controller_state = None;
        let _ = sync_picture_in_picture_controller(PictureInPictureControllerState::new(
            self.picture_in_picture_host_id,
            false,
            false,
            None,
        ));
    }

    fn emit_picture_in_picture_event_if_needed(&mut self) {
        let active = match is_picture_in_picture_active(self.picture_in_picture_host_id) {
            Ok(active) => active,
            Err(error) => {
                self.emit_event(Event::Error {
                    message: error.to_string(),
                });
                return;
            }
        };
        if self.last_picture_in_picture_active == Some(active) {
            return;
        }

        self.last_picture_in_picture_active = Some(active);
        self.emit_event(Event::PictureInPictureChanged { active });
    }

    fn apply_pending_seek_if_due(&mut self, should_play: bool) {
        let Some(mut requested) = self.pending_seek_request else {
            return;
        };
        requested = requested.clamp(0.0, 1.0);

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
        self.ended_sent = false;
        self.seek_inflight = true;

        let target_pts = self.duration.mul_f64(requested);
        self.set_playback_position(target_pts, should_play);
        self.last_reported_progress = requested;
        self.seek_audio_to(target_pts, should_play);

        if let Some(worker) = self.decode_worker.as_ref() {
            worker.request_seek(requested);
        } else if let Err(message) = self.restart_decoder_from_progress(requested) {
            self.seek_inflight = false;
            self.emit_event(Event::Error { message });
            self.set_buffering(false);
            return;
        }

        self.set_buffering(true);
        self.sync_media_session(should_play);
        self.update_ui_progress();
    }

    fn update_ui_progress(&mut self) {
        if self.player.is_none() {
            return;
        }

        let progress = self.last_reported_progress.clamp(0.0, 1.0);
        self.push_ui_update(UiUpdate::Progress(progress));
        self.push_ui_update(UiUpdate::Duration(self.duration.as_secs_f64()));
        self.push_ui_update(UiUpdate::Position(
            self.duration.mul_f64(progress).as_secs_f64(),
        ));
    }

    fn ensure_texture_resources(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let uv_width = (width / 2).max(1);
        let uv_height = (height / 2).max(1);

        let need_texture =
            self.y_texture
                .as_ref()
                .is_none_or(|texture| texture.width() != width || texture.height() != height)
                || self.uv_texture.as_ref().is_none_or(|texture| {
                    texture.width() != uv_width || texture.height() != uv_height
                });
        if !need_texture {
            return;
        }

        let y_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Video Y texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let uv_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Video UV texture"),
            size: wgpu::Extent3d {
                width: uv_width,
                height: uv_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let Some(bind_group_layout) = self.bind_group_layout.as_ref() else {
            return;
        };
        let Some(sampler) = self.sampler.as_ref() else {
            return;
        };
        let Some(color_uniform_buffer) = self.color_uniform_buffer.as_ref() else {
            return;
        };

        let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let uv_view = uv_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Video bind group"),
            layout: bind_group_layout,
            entries: &[
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
            ],
        });

        self.y_texture = Some(y_texture);
        self.uv_texture = Some(uv_texture);
        self.bind_group = Some(bind_group);
        self.vertex_buffer = None;
        self.vertex_layout_key = None;
    }

    fn upload_frame_texture(&mut self, frame: &GpuFrame, decoded: &DecodedVideoFrame) {
        self.ensure_texture_resources(frame.device, decoded.width, decoded.height);
        let Some(y_texture) = self.y_texture.as_ref() else {
            return;
        };
        let Some(uv_texture) = self.uv_texture.as_ref() else {
            return;
        };

        let y_plane_len = (decoded.width * decoded.height) as usize;
        if decoded.nv12.len() < y_plane_len {
            return;
        }
        let (y_plane, uv_plane) = decoded.nv12.split_at(y_plane_len);

        frame.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: y_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            y_plane,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(decoded.width),
                rows_per_image: Some(decoded.height),
            },
            wgpu::Extent3d {
                width: decoded.width,
                height: decoded.height,
                depth_or_array_layers: 1,
            },
        );

        frame.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: uv_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            uv_plane,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(decoded.width),
                rows_per_image: Some((decoded.height / 2).max(1)),
            },
            wgpu::Extent3d {
                width: (decoded.width / 2).max(1),
                height: (decoded.height / 2).max(1),
                depth_or_array_layers: 1,
            },
        );
    }

    fn ensure_vertex_buffer(
        &mut self,
        device: &wgpu::Device,
        surface_width: u32,
        surface_height: u32,
    ) {
        let Some(texture) = self.y_texture.as_ref() else {
            self.vertex_buffer = None;
            self.vertex_layout_key = None;
            return;
        };

        let key = VertexLayoutKey {
            surface_width: surface_width.max(1),
            surface_height: surface_height.max(1),
            video_width: texture.width().max(1),
            video_height: texture.height().max(1),
            aspect_ratio: self.aspect_ratio,
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
            size: bytes.len() as u64,
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
        self.poll_source_download_updates();
        if let Err(message) = self.ensure_subtitle_cues() {
            self.set_subtitle_text(None);
            if !self.subtitle_error_reported {
                self.emit_event(Event::Error {
                    message: message.clone(),
                });
                self.subtitle_error_reported = true;
            }
        }

        self.reconcile_play_request_from_ui();
        self.poll_media_commands();
        self.poll_picture_in_picture_commands();

        if self.decode_worker.is_none() && self.should_open_decode_worker() {
            self.open_decode_state();
        }

        let should_play = self.should_play();
        self.update_playing_state(should_play);
        self.sync_playback_rate(should_play);
        self.poll_pending_audio_open();
        self.retry_audio_player_if_needed();
        self.sync_audio_volume();
        self.sync_audio_playback_params();
        self.maybe_seek_from_ui();
        self.sync_picture_in_picture_controller(should_play);
        self.maybe_enter_picture_in_picture();
        self.emit_picture_in_picture_event_if_needed();
        self.apply_pending_seek_if_due(should_play);
        self.drain_decoder_outputs(should_play);
        let now = Instant::now();
        self.sync_subtitle_text(now);
        self.maybe_emit_buffer_level(now);
        self.maybe_emit_playback_metrics(now);

        if self.decode_worker.is_none() {
            return;
        }

        if !should_play && self.pending_frame.is_none() && self.y_texture.is_some() {
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

        let Some(pending) = self.pending_frame.as_ref() else {
            return;
        };
        let pending_pts = pending.pts;

        let present_immediately = self.y_texture.is_none();
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

        self.upload_frame_texture(frame, &decoded);
        if !self.first_frame_presented {
            tracing::info!(
                "[VideoFallback] first frame presented pts={:.3}s progress={:.3}",
                decoded.pts.as_secs_f64(),
                decoded.progress
            );
            self.first_frame_presented = true;
        }
        self.set_playback_position(decoded.pts, should_play);
        self.sync_subtitle_text(Instant::now());
        self.set_buffering(false);
        self.sync_picture_in_picture_controller(should_play);

        if self.player.is_some() {
            let progress = decoded.progress;
            self.push_ui_update(UiUpdate::Progress(progress));
            self.push_ui_update(UiUpdate::Position(decoded.pts.as_secs_f64()));
            self.last_reported_progress = progress;
        }

        self.sync_media_session(should_play);
    }

    fn render_surface(&mut self, frame: &GpuFrame) {
        self.ensure_vertex_buffer(frame.device, frame.width, frame.height);
        self.upload_color_uniform_if_needed(frame.queue);

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
            return Some((frame.width.max(1), frame.height.max(1)));
        }

        self.y_texture
            .as_ref()
            .map(|texture| (texture.width().max(1), texture.height().max(1)))
    }

    fn current_video_aspect_ratio(&self) -> Option<f32> {
        self.current_video_dimensions()
            .map(|(width, height)| width as f32 / height as f32)
    }
}

impl GpuView for VideoRenderer {
    fn preferred_surface_hdr(&self) -> Option<bool> {
        *self.surface_prefers_hdr_cache.get_or_init(|| {
            let preference = preferred_surface_hdr_for_source(&self.source);
            tracing::info!(
                "[VideoFallback] initial surface hdr preference source={} preference={preference:?}",
                self.source.as_str()
            );
            preference
        })
    }

    async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        self.redraw_handle = Some(ctx.redraw_handle.clone());
        self.ensure_pipeline(ctx.device, ctx.surface_format);
        self.open_decode_state();
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        self.step_decoder_if_needed(frame);
        self.render_surface(frame);

        // Request continuous redraw while playback/buffering is active
        let needs_redraw = if self.decode_worker.is_none() {
            self.should_poll_source() || self.should_play() || self.is_buffering
        } else {
            self.pending_frame.is_some() || self.should_play() || self.is_buffering
        };
        if needs_redraw {
            frame.request_redraw();
        }
    }
}

impl SubView for VideoRenderer {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        if self.aspect_ratio != AspectRatio::Fit {
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
                    ViewDimensions::new(Size::new(video_width as f32, video_height as f32))
                } else {
                    ViewDimensions::new(Size::zero())
                }
            }
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        match self.aspect_ratio {
            AspectRatio::Fit => StretchAxis::Horizontal,
            AspectRatio::Fill | AspectRatio::Stretch => StretchAxis::Both,
        }
    }

    fn priority(&self) -> i32 {
        0
    }
}

impl Drop for VideoRenderer {
    fn drop(&mut self) {
        self.clear_picture_in_picture_controller();
        self.stop_decode_worker();
        if let Some(mut poller) = self.media_command_poller.take() {
            poller.stop();
        }
        if let Some(player) = self.audio_player.take() {
            player.stop();
        }
    }
}

struct MediaCommandPoller {
    stop: Sender<()>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MediaCommandPoller {
    fn spawn(redraw_handle: RedrawHandle) -> Self {
        let (stop, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            loop {
                match receiver.recv_timeout(MEDIA_COMMAND_POLL_INTERVAL) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => redraw_handle.request_redraw(),
                }
            }
        });

        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn stop(&mut self) {
        let _ = self.stop.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for MediaCommandPoller {
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
        let session = MediaSession::new().ok()?;
        let _ = session.set_metadata(
            &MediaMetadata::new()
                .with_title(source.as_str())
                .with_duration(duration),
        );
        Some(Self {
            session,
            queue_navigation_controls: QueueNavigationControls::disabled(),
            has_audio_focus: false,
        })
    }

    fn poll_command(&self) -> Option<MediaCommand> {
        self.session.poll_command()
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
            if self.session.request_audio_focus().is_ok() {
                self.has_audio_focus = true;
            }
        } else if !hold_audio_focus && self.has_audio_focus {
            let _ = self.session.abandon_audio_focus();
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
        let _ = self.session.set_playback_state(&playback);
    }
}

impl Drop for MediaSessionState {
    fn drop(&mut self) {
        let _ = self.session.set_playback_state(
            &PlaybackState::stopped()
                .with_queue_navigation_controls(self.queue_navigation_controls),
        );
        if self.has_audio_focus {
            let _ = self.session.abandon_audio_focus();
        }
        let _ = self.session.clear();
    }
}

#[derive(Debug)]
struct DecodedVideoFrame {
    nv12: Vec<u8>,
    pts: Duration,
    progress: f64,
    width: u32,
    height: u32,
}

#[derive(Debug)]
struct PendingDecodedFrame {
    frame: DecodedFrame,
    pts: Duration,
    progress: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VideoSampleMetadata {
    pts: u64,
    is_keyframe: bool,
}

fn estimated_video_duration(sample_metadata: &[VideoSampleMetadata], timescale: u32) -> Duration {
    match sample_metadata {
        [] => Duration::ZERO,
        [sample] => pts_to_duration(sample.pts, timescale),
        _ => {
            let last = sample_metadata
                .last()
                .expect("sample metadata length > 1 must have a last sample");
            let previous = sample_metadata[sample_metadata.len() - 2];
            let frame_span = last.pts.saturating_sub(previous.pts);
            pts_to_duration(last.pts.saturating_add(frame_span), timescale)
        }
    }
}

fn nearest_keyframe_index_at_or_before(
    sample_metadata: &[VideoSampleMetadata],
    target_index: usize,
) -> usize {
    sample_metadata
        .iter()
        .take(target_index.saturating_add(1))
        .enumerate()
        .rev()
        .find_map(|(index, sample)| sample.is_keyframe.then_some(index))
        .unwrap_or(0)
}

fn sample_pts_duration(
    sample_metadata: &[VideoSampleMetadata],
    timescale: u32,
    sample_index: usize,
) -> Duration {
    sample_metadata
        .get(sample_index)
        .map_or(Duration::ZERO, |sample| {
            pts_to_duration(sample.pts, timescale)
        })
}

struct DecodeState {
    reader: VideoReader,
    decoder: Decoder,
    pending_decoded: VecDeque<PendingDecodedFrame>,
    width: u32,
    height: u32,
    timescale: u32,
    total_samples: u32,
    duration: Duration,
    codec_type: CodecType,
    codec_config: Option<Vec<u8>>,
    sample_metadata: Vec<VideoSampleMetadata>,
}

impl DecodeState {
    fn open(source_path: &Path) -> Result<Self, String> {
        let reader = VideoReader::open(source_path).map_err(|error| error.to_string())?;
        let (width, height) = reader.dimensions();
        let codec_config = reader.codec_config().map(|config| config.to_vec());
        let codec_type = detect_codec_type(codec_config.as_deref())?;
        let decoder = Decoder::new(codec_type, codec_config.as_deref(), width, height)
            .map_err(|error| error.to_string())?;
        let timescale = reader.timescale();
        let sample_count = reader.sample_count();
        let sample_capacity =
            usize::try_from(sample_count).expect("video sample count must fit into usize");
        let mut sample_metadata = Vec::with_capacity(sample_capacity);
        for index in 0..sample_capacity {
            let (pts, is_keyframe) = reader
                .sample_info(index)
                .ok_or_else(|| format!("missing video sample metadata for sample {index}"))?;
            sample_metadata.push(VideoSampleMetadata { pts, is_keyframe });
        }

        let first_pts = sample_metadata.first().map_or(0, |sample| sample.pts);
        let second_pts = sample_metadata.get(1).map_or(0, |sample| sample.pts);
        let third_pts = sample_metadata.get(2).map_or(0, |sample| sample.pts);
        let duration = estimated_video_duration(&sample_metadata, timescale);
        tracing::info!(
            "[VideoFallback] decode source stats size={}x{} samples={} timescale={} pts=[{first_pts},{second_pts},{third_pts}] duration={:.3}s",
            width,
            height,
            sample_count,
            timescale,
            duration.as_secs_f64()
        );

        Ok(Self {
            reader,
            decoder,
            pending_decoded: VecDeque::new(),
            width,
            height,
            timescale,
            total_samples: sample_count,
            duration,
            codec_type,
            codec_config,
            sample_metadata,
        })
    }

    fn duration(&self) -> Duration {
        self.duration
    }

    fn rebuild_decoder(&mut self) -> Result<(), String> {
        self.decoder = Decoder::new(
            self.codec_type,
            self.codec_config.as_deref(),
            self.width,
            self.height,
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn seek_to_progress(&mut self, progress: f64) -> Result<Duration, String> {
        if self.total_samples == 0 {
            return Ok(Duration::ZERO);
        }

        let clamped = progress.clamp(0.0, 1.0);
        let target_index =
            (clamped * f64::from(self.total_samples.saturating_sub(1))).round() as usize;
        let keyframe_index =
            nearest_keyframe_index_at_or_before(&self.sample_metadata, target_index);

        self.rebuild_decoder()?;
        self.reader.seek_to_sample(keyframe_index);
        self.pending_decoded.clear();

        while self.reader.current_index() < target_index {
            let Some((sample_data, _, _)) = self
                .reader
                .read_sample()
                .map_err(|error| error.to_string())?
            else {
                return Ok(sample_pts_duration(
                    &self.sample_metadata,
                    self.timescale,
                    target_index,
                ));
            };
            let mut stream = self.decoder.decode(&sample_data);
            while let Some(result) = stream.next() {
                result.map_err(|error| error.to_string())?;
            }
        }

        Ok(sample_pts_duration(
            &self.sample_metadata,
            self.timescale,
            target_index,
        ))
    }

    fn next_frame(&mut self) -> Result<Option<DecodedVideoFrame>, String> {
        if let Some(frame) = self.pending_decoded.pop_front() {
            return Ok(Some(self.materialize_frame(frame)));
        }

        loop {
            let sample_index = self.reader.current_index();
            let Some((sample_data, pts, _)) = self
                .reader
                .read_sample()
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            let pts = pts_to_duration(pts, self.timescale);
            let progress = if self.total_samples <= 1 {
                0.0
            } else {
                let bounded_index = u32::try_from(sample_index)
                    .expect("video sample index must fit into u32")
                    .min(self.total_samples - 1);
                f64::from(bounded_index) / f64::from(self.total_samples - 1)
            };

            let mut stream = self.decoder.decode(&sample_data);
            while let Some(result) = stream.next() {
                let decoded = result.map_err(|error| error.to_string())?;
                let decoded_pts = if decoded.timestamp_ns() > 0 {
                    Duration::from_nanos(decoded.timestamp_ns())
                } else {
                    pts
                };
                self.pending_decoded.push_back(PendingDecodedFrame {
                    frame: decoded,
                    pts: decoded_pts,
                    progress,
                });
            }

            if let Some(frame) = self.pending_decoded.pop_front() {
                if sample_index % 240 == 0 {
                    tracing::info!(
                        "[VideoFallback] decoded sample_index={} pts={:.3}s progress={:.3}",
                        sample_index,
                        frame.pts.as_secs_f64(),
                        progress
                    );
                }
                return Ok(Some(self.materialize_frame(frame)));
            }
        }
    }

    fn materialize_frame(&self, pending: PendingDecodedFrame) -> DecodedVideoFrame {
        let mut nv12 = vec![0_u8; (self.width * self.height * 3 / 2) as usize];
        pending.frame.copy_to_buffer(&mut nv12);
        DecodedVideoFrame {
            nv12,
            pts: pending.pts,
            progress: pending.progress,
            width: self.width,
            height: self.height,
        }
    }
}

fn is_remote_url(url: &Url) -> bool {
    matches!(url.scheme(), Some("http" | "https"))
}

fn local_source_path(url: &Url) -> PathBuf {
    PathBuf::from(url.as_str())
}

fn cached_remote_asset_path(url: &Url, default_extension: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    url.as_str().hash(&mut hasher);
    let hash = hasher.finish();

    let extension = {
        let path = url.path();
        let candidate = path.rsplit_once('.').map_or("", |(_, ext)| ext);
        let clean = candidate
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>();
        if clean.is_empty() {
            String::from(default_extension)
        } else {
            clean
        }
    };

    let cache_dir = std::env::temp_dir().join("waterui_video_cache");
    cache_dir.join(format!("{hash:016x}.{extension}"))
}

fn cached_video_path(url: &Url) -> PathBuf {
    cached_remote_asset_path(url, "mp4")
}

fn cached_subtitle_path(url: &Url) -> PathBuf {
    cached_remote_asset_path(url, "vtt")
}

fn preferred_surface_hdr_for_source(source: &Url) -> Option<bool> {
    let probe_path = if is_remote_url(source) {
        let cached = cached_video_path(source);
        if !cached.exists() {
            return None;
        }
        cached
    } else {
        let local = local_source_path(source);
        if !local.exists() {
            return None;
        }
        local
    };

    Some(probe_video_color_profile(&probe_path, None).hdr)
}

fn start_asset_download(url: String, destination: PathBuf) -> Receiver<DownloadUpdate> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        if let Err(message) = download_asset_to_path(&url, &destination, &sender) {
            let _ = sender.send(DownloadUpdate::Failed(message));
        }
    });

    receiver
}

fn download_asset_to_path(
    url: &str,
    destination: &Path,
    updates: &Sender<DownloadUpdate>,
) -> Result<(), String> {
    let Some(parent) = destination.parent() else {
        return Err(String::from("Invalid download destination"));
    };

    fs::create_dir_all(parent).map_err(|error| error.to_string())?;

    let download_result = futures::executor::block_on(async {
        use futures::StreamExt;
        use zenwave::{Client, Method, redirect::FollowRedirect};

        let mut client = FollowRedirect::new(zenwave::client());
        let response = client
            .method(Method::GET, url)
            .await
            .map_err(|error| error.to_string())?;

        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()));
        }

        let total_bytes = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());

        let mut body = response.into_body();
        let mut file = File::create(destination).map_err(|error| error.to_string())?;
        let mut bytes_written = 0usize;
        let mut last_probe = 0usize;
        let mut last_progress_report = 0usize;
        let mut ready_sent = false;
        let _ = updates.send(DownloadUpdate::Progress {
            bytes_written,
            total_bytes,
        });

        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|error| error.to_string())?;
            file.write_all(&chunk).map_err(|error| error.to_string())?;
            bytes_written = bytes_written.saturating_add(chunk.len());

            if bytes_written.saturating_sub(last_progress_report)
                >= DOWNLOAD_PROGRESS_REPORT_INTERVAL_BYTES
            {
                last_progress_report = bytes_written;
                let _ = updates.send(DownloadUpdate::Progress {
                    bytes_written,
                    total_bytes,
                });
            }

            if !ready_sent && bytes_written >= STREAMING_MIN_READY_BYTES {
                let should_probe =
                    bytes_written.saturating_sub(last_probe) >= STREAMING_PROBE_INTERVAL_BYTES;
                if should_probe {
                    file.flush().map_err(|error| error.to_string())?;
                    last_probe = bytes_written;
                    if VideoReader::open(destination).is_ok() {
                        ready_sent = true;
                        let _ = updates.send(DownloadUpdate::Ready);
                    }
                }
            }
        }

        file.flush().map_err(|error| error.to_string())?;
        let _ = updates.send(DownloadUpdate::Progress {
            bytes_written,
            total_bytes,
        });
        if !ready_sent && VideoReader::open(destination).is_ok() {
            let _ = updates.send(DownloadUpdate::Ready);
        }
        let _ = updates.send(DownloadUpdate::Finished);
        Ok::<(), String>(())
    });

    if let Err(message) = download_result {
        let _ = fs::remove_file(destination);
        return Err(message);
    }
    Ok(())
}

fn build_vertices(
    aspect_ratio: AspectRatio,
    video_width: u32,
    video_height: u32,
    surface_width: u32,
    surface_height: u32,
) -> [[f32; 4]; 6] {
    let video_ratio = (video_width.max(1) as f32) / (video_height.max(1) as f32);
    let surface_ratio = (surface_width.max(1) as f32) / (surface_height.max(1) as f32);

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

fn pts_to_duration(pts: u64, timescale: u32) -> Duration {
    if timescale == 0 {
        return Duration::ZERO;
    }

    Duration::from_nanos((pts.saturating_mul(1_000_000_000)) / u64::from(timescale))
}

fn detect_codec_type(config: Option<&[u8]>) -> Result<CodecType, String> {
    let Some(config) = config else {
        return Err(String::from("No codec config found"));
    };

    if config.len() >= 8 && &config[4..8] == b"avcC" {
        return Ok(CodecType::H264);
    }

    if config.len() >= 7 && config[0] == 0x01 {
        let profile = config[1];
        if matches!(profile, 66 | 77 | 88 | 100 | 110 | 122 | 144) {
            return Ok(CodecType::H264);
        }
    }

    if config.len() >= 8 && &config[4..8] == b"hvcC" {
        return Ok(CodecType::H265);
    }

    if config.len() >= 23 && config[0] == 0x01 {
        let level = config[12];
        if level > 0 && level <= 186 {
            return Ok(CodecType::H265);
        }
    }

    Err(String::from(
        "Unknown codec (only H.264 and H.265 are supported)",
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        PlaybackPolicy, ShaderTargetMode, VideoSampleMetadata, effective_audio_volume,
        estimated_video_duration, nearest_keyframe_index_at_or_before, next_subtitle_selection,
        preferred_surface_hdr_for_source, progress_for_position, resolve_selected_subtitle_index,
        runtime_sidecar_subtitle_tracks, runtime_subtitle_track_labels,
        select_default_subtitle_track_index, shader_target_mode, should_enter_vod_stall_buffering,
        should_wait_for_vod_buffering,
    };
    use crate::{SubtitleSelection, SubtitleTrack};
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, SystemTime},
    };
    use waterui_graphics::wgpu;
    use waterui_url::Url;

    #[test]
    fn hdr_source_maps_to_sdr_on_srgb_surface() {
        let mode = shader_target_mode(wgpu::TextureFormat::Bgra8UnormSrgb, true);
        assert_eq!(mode, ShaderTargetMode::LinearSdr);
    }

    #[test]
    fn hdr_source_maps_to_hdr_on_float_surface() {
        let mode = shader_target_mode(wgpu::TextureFormat::Rgba16Float, true);
        assert_eq!(mode, ShaderTargetMode::LinearHdr);
    }

    #[test]
    fn sdr_source_stays_sdr_even_on_float_surface() {
        let mode = shader_target_mode(wgpu::TextureFormat::Rgba16Float, false);
        assert_eq!(mode, ShaderTargetMode::LinearSdr);
    }

    fn unique_temp_file(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system clock must be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("waterui_video_{name}_{nanos}.mp4"))
    }

    fn write_nclx_file(path: &PathBuf, transfer: u16) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0, 0, 0, 4, b'f', b't', b'y', b'p']);
        bytes.extend_from_slice(&19_u32.to_be_bytes());
        bytes.extend_from_slice(b"colr");
        bytes.extend_from_slice(b"nclx");
        bytes.extend_from_slice(&1_u16.to_be_bytes());
        bytes.extend_from_slice(&transfer.to_be_bytes());
        bytes.extend_from_slice(&1_u16.to_be_bytes());
        bytes.push(0);
        fs::write(path, bytes).expect("test nclx file write must succeed");
    }

    #[test]
    fn preferred_surface_uses_sdr_for_sdr_source() {
        let path = unique_temp_file("sdr_pref");
        write_nclx_file(&path, 1);
        let url = Url::from_file_path_str(path.to_string_lossy().to_string());
        let preference = preferred_surface_hdr_for_source(&url);
        assert_eq!(preference, Some(false));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn preferred_surface_uses_hdr_for_hdr_source() {
        let path = unique_temp_file("hdr_pref");
        write_nclx_file(&path, 16);
        let url = Url::from_file_path_str(path.to_string_lossy().to_string());
        let preference = preferred_surface_hdr_for_source(&url);
        assert_eq!(preference, Some(true));
        let _ = fs::remove_file(path);
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
    fn estimated_video_duration_uses_last_sample_delta() {
        let sample_metadata = [
            VideoSampleMetadata {
                pts: 0,
                is_keyframe: true,
            },
            VideoSampleMetadata {
                pts: 40,
                is_keyframe: false,
            },
            VideoSampleMetadata {
                pts: 80,
                is_keyframe: false,
            },
        ];
        assert_eq!(
            estimated_video_duration(&sample_metadata, 1_000),
            Duration::from_millis(120)
        );
    }

    #[test]
    fn nearest_keyframe_helper_returns_latest_sync_sample() {
        let sample_metadata = [
            VideoSampleMetadata {
                pts: 0,
                is_keyframe: true,
            },
            VideoSampleMetadata {
                pts: 40,
                is_keyframe: false,
            },
            VideoSampleMetadata {
                pts: 80,
                is_keyframe: true,
            },
            VideoSampleMetadata {
                pts: 120,
                is_keyframe: false,
            },
        ];
        assert_eq!(nearest_keyframe_index_at_or_before(&sample_metadata, 0), 0);
        assert_eq!(nearest_keyframe_index_at_or_before(&sample_metadata, 1), 0);
        assert_eq!(nearest_keyframe_index_at_or_before(&sample_metadata, 2), 2);
        assert_eq!(nearest_keyframe_index_at_or_before(&sample_metadata, 3), 2);
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
    fn subtitle_selection_cycles_auto_off_tracks_then_auto() {
        let tracks = vec![
            SubtitleTrack::new("https://example.com/subs/en.vtt").language("en"),
            SubtitleTrack::new("https://example.com/subs/es.vtt").language("es"),
        ];
        let labels = runtime_subtitle_track_labels(&runtime_sidecar_subtitle_tracks(&tracks));

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
    fn progress_for_position_clamps_to_duration() {
        let duration = Duration::from_secs(10);

        assert_eq!(progress_for_position(duration, Duration::ZERO), 0.0);
        assert_eq!(progress_for_position(duration, Duration::from_secs(5)), 0.5);
        assert_eq!(
            progress_for_position(duration, Duration::from_secs(15)),
            1.0
        );
    }

    #[test]
    fn effective_audio_volume_respects_muted_and_ducked_states() {
        assert_eq!(effective_audio_volume(0.8, false), 0.8);
        assert!((effective_audio_volume(0.8, true) - 0.16).abs() <= f32::EPSILON);
        assert_eq!(effective_audio_volume(-0.8, false), 0.0);
        assert_eq!(effective_audio_volume(-0.8, true), 0.0);
    }
}
