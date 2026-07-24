//! Background demux and decode worker for the self-drawn player.

use std::future::Future;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

use async_channel::{Receiver, Sender, TryRecvError};
use futures::future::{Either, select};
#[cfg(target_os = "android")]
use waterkit_audio::DecodedAudioFrame;
use waterkit_audio::{AudioOutput, StreamingAudioPlayer, StreamingAudioProducer};
use waterkit_video::container::{
    EncodedSample, ProgressiveTrackReader, SubtitleCue, TimedMetadata, TrackInfo, TrackKind,
};
use waterkit_video::streaming::{
    AdaptiveSelectionPolicy, DashRepresentation, MediaRequest, StreamVariant, Url,
};
#[cfg(target_os = "android")]
use waterkit_video::{
    AndroidAudioDrmBootstrap, AndroidDrmBootstrap, AndroidLicenseChallenge, AndroidOfflineKeySet,
    AndroidOffloadAudioController, AndroidOffloadAudioPlayback, AndroidPendingDecoder,
    AndroidProtectedAudioDecoder, AndroidProtectedVideoDecoder, AndroidProtectedVideoOutput,
    AndroidReadyDecoder, AndroidTunneledPlayback, AndroidVideoDecoderTarget, AnyLicenseServer,
    LicenseRequest, LicenseResponse, LicenseServer, ProtectionInitData,
};
use waterkit_video::{
    AudioTrackDecoder, AudioTrackSelection as EngineAudioTrackSelection, DashPlaybackSession,
    DashSegmentPoll, DecodedVideoFrame, HlsPlaybackSession, HlsSegmentPoll,
    LivePlaybackRateRange as EngineLivePlaybackRateRange, LiveWindow as EngineLiveWindow,
    SegmentedPlaybackOptions, SelectableAudioTrack, SelectableSubtitleTrack, SelectableVideoTrack,
    SubtitleTrackSelection as EngineSubtitleTrackSelection, VideoColorInfo, VideoError,
    VideoPlayer, VideoTrackDecoder, VideoTrackSelection as EngineVideoTrackSelection,
};
use waterui_video::{AudioTrackSelection, NetworkPlaybackPolicy, VideoTrackSelection};
#[cfg(target_os = "android")]
use waterui_video::{DrmConfiguration, OfflineDrmKeySet, PlaybackOutputPath, PlaybackPowerPolicy};

#[cfg(target_os = "android")]
use crate::android_video_surface::{
    AndroidVideoSurfaceLifecycle, AndroidVideoSurfacePort, AndroidVideoSurfaceReceiver,
};
use crate::latest_channel::{LatestSender, latest_channel};

const PROGRESSIVE_FRAME_QUEUE_CAPACITY: usize = 4;
const SEGMENTED_FRAME_QUEUE_CAPACITY: usize = 1;
const DECODE_EVENT_QUEUE_CAPACITY: usize = 8;

#[derive(Debug)]
enum DecoderControl {
    Stop,
    Seek { position: Duration },
}

#[cfg(target_os = "android")]
enum ProtectedOutputControl {
    Present { sequence: u64, delay: Duration },
    Discard { sequence: u64 },
}

#[derive(Clone, Copy)]
struct SegmentedDecoderChannels<'a> {
    updates: &'a Sender<DecoderOutput>,
    frames: &'a Sender<DecodedVideoFrame>,
    stale_frames: &'a Receiver<DecodedVideoFrame>,
    control: &'a Receiver<DecoderControl>,
    #[cfg(target_os = "android")]
    protected_control: &'a Receiver<ProtectedOutputControl>,
}

/// Segmented transport selected by the semantic media item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentedProtocol {
    /// HTTP Live Streaming.
    Hls,
    /// MPEG-DASH.
    Dash,
}

/// Validated resource and viewport inputs for segmented playback.
#[derive(Debug, Clone)]
pub struct SegmentedDecoderConfig {
    network: NetworkPlaybackPolicy,
    viewport: Option<(u32, u32)>,
    audio_output: AudioOutput,
    audio_track_selection: AudioTrackSelection,
    video_track_selection: VideoTrackSelection,
    subtitle_track_selection: EngineSubtitleTrackSelection,
    #[cfg(target_os = "android")]
    android_playback: Option<AndroidPlaybackConfig>,
}

impl SegmentedDecoderConfig {
    /// Creates worker configuration from semantic playback policy.
    pub const fn new(
        network: NetworkPlaybackPolicy,
        viewport: Option<(u32, u32)>,
        audio_output: AudioOutput,
        audio_track_selection: AudioTrackSelection,
        video_track_selection: VideoTrackSelection,
        subtitle_track_selection: EngineSubtitleTrackSelection,
    ) -> Self {
        Self {
            network,
            viewport,
            audio_output,
            audio_track_selection,
            video_track_selection,
            subtitle_track_selection,
            #[cfg(target_os = "android")]
            android_playback: None,
        }
    }

    #[cfg(target_os = "android")]
    pub(crate) fn android_playback(mut self, config: AndroidPlaybackConfig) -> Self {
        self.android_playback = Some(config);
        self
    }

    fn playback_options(&self) -> Result<SegmentedPlaybackOptions, VideoError> {
        let adaptive = AdaptiveSelectionPolicy::new(
            self.network.bandwidth_fraction_per_mille(),
            self.network.minimum_buffer_for_upgrade(),
            self.network.emergency_buffer(),
        )?;
        let audio_track_selection = match self.audio_track_selection {
            AudioTrackSelection::Auto => EngineAudioTrackSelection::Auto,
            AudioTrackSelection::Track(index) => EngineAudioTrackSelection::Track(index),
        };
        let video_track_selection = match self.video_track_selection {
            VideoTrackSelection::Auto => EngineVideoTrackSelection::Auto,
            VideoTrackSelection::Track(index) => EngineVideoTrackSelection::Track(index),
        };
        Ok(SegmentedPlaybackOptions::new(
            self.network.maximum_segment_bytes(),
            self.network.initial_bandwidth(),
            adaptive,
            self.viewport,
        )
        .audio_track_selection(audio_track_selection)
        .video_track_selection(video_track_selection)
        .subtitle_track_selection(self.subtitle_track_selection))
    }
}

/// Per-player Android protected playback services.
#[cfg(target_os = "android")]
#[derive(Debug, Clone)]
pub struct AndroidPlaybackConfig {
    drm: Option<AndroidDrmNetworkConfig>,
    license_server: AnyLicenseServer,
    surfaces: AndroidVideoSurfaceReceiver,
    power: PlaybackPowerPolicy,
    compatibility: AndroidPowerCompatibility,
}

#[cfg(target_os = "android")]
#[derive(Debug, Clone, Copy)]
pub struct AndroidPowerCompatibility {
    clock: AndroidPlaybackClock,
    video_access: AndroidVideoAccess,
    audio_processing: AndroidAudioProcessing,
    audio_route: AndroidAudioRoute,
}

#[cfg(target_os = "android")]
#[derive(Debug, Clone, Copy)]
pub enum AndroidPlaybackClock {
    Fixed(f32),
    Realtime,
}

#[cfg(target_os = "android")]
#[derive(Debug, Clone, Copy)]
pub enum AndroidVideoAccess {
    DirectSurface,
    GpuSamplingRequired,
}

#[cfg(target_os = "android")]
#[derive(Debug, Clone, Copy)]
pub enum AndroidAudioProcessing {
    DirectCompressedEligible,
    SkipSilence,
}

#[cfg(target_os = "android")]
#[derive(Debug, Clone, Copy)]
pub enum AndroidAudioRoute {
    PlatformSelected,
    ExplicitDevice,
}

#[cfg(target_os = "android")]
impl AndroidPowerCompatibility {
    pub const fn new(
        clock: AndroidPlaybackClock,
        video_access: AndroidVideoAccess,
        audio_processing: AndroidAudioProcessing,
        audio_route: AndroidAudioRoute,
    ) -> Self {
        Self {
            clock,
            video_access,
            audio_processing,
            audio_route,
        }
    }
}

#[cfg(target_os = "android")]
impl AndroidPlaybackConfig {
    pub(crate) fn new(
        drm: Option<DrmConfiguration>,
        license_server: AnyLicenseServer,
        surfaces: AndroidVideoSurfaceReceiver,
        power: PlaybackPowerPolicy,
        compatibility: AndroidPowerCompatibility,
    ) -> Self {
        Self {
            drm: drm.map(AndroidDrmNetworkConfig::from),
            license_server,
            surfaces,
            power,
            compatibility,
        }
    }

    fn validate_power_requirements(&self) -> Result<(), VideoError> {
        if self.power == PlaybackPowerPolicy::PlatformManaged {
            return Ok(());
        }
        if self.drm.is_some() {
            return Err(VideoError::Unsupported(String::from(
                "required Android offload/tunneling does not accept DRM until a CDM-aware compressed-audio path is selected",
            )));
        }
        if matches!(
            self.compatibility.audio_route,
            AndroidAudioRoute::ExplicitDevice
        ) {
            return Err(VideoError::Unsupported(String::from(
                "required Android offload/tunneling cannot preserve a cpal-selected output device",
            )));
        }
        if matches!(
            self.compatibility.audio_processing,
            AndroidAudioProcessing::SkipSilence
        ) {
            return Err(VideoError::Unsupported(String::from(
                "skip-silence requires decoded PCM and is incompatible with required Android offload/tunneling",
            )));
        }
        if !matches!(
            self.compatibility.clock,
            AndroidPlaybackClock::Fixed(rate) if (rate - 1.0).abs() <= 0.001
        ) {
            return Err(VideoError::Unsupported(String::from(
                "required Android offload/tunneling needs a fixed 1.0 playback clock without live catch-up",
            )));
        }
        if self.power == PlaybackPowerPolicy::RequireAudioVideoTunneling
            && matches!(
                self.compatibility.video_access,
                AndroidVideoAccess::GpuSamplingRequired
            )
        {
            return Err(VideoError::Unsupported(String::from(
                "spherical projection requires GPU frame access and is incompatible with Android tunneling",
            )));
        }
        Ok(())
    }
}

#[cfg(target_os = "android")]
#[derive(Debug, Clone)]
struct AndroidDrmNetworkConfig {
    license_url: Option<String>,
    provisioning_url: Option<String>,
    request_headers: Vec<(String, String)>,
    maximum_response_bytes: std::num::NonZeroUsize,
    renewal_threshold: Duration,
    offline_key_set: Option<Vec<u8>>,
}

#[cfg(target_os = "android")]
impl From<DrmConfiguration> for AndroidDrmNetworkConfig {
    fn from(value: DrmConfiguration) -> Self {
        Self {
            license_url: value
                .resolved_license_url()
                .map(|url| url.as_str().to_owned()),
            provisioning_url: value
                .resolved_provisioning_url()
                .map(|url| url.as_str().to_owned()),
            request_headers: value.request_headers().to_vec(),
            maximum_response_bytes: value.resolved_maximum_response_bytes(),
            renewal_threshold: value.resolved_renewal_threshold(),
            offline_key_set: value
                .resolved_offline_key_set()
                .map(|key_set| key_set.as_bytes().to_vec()),
        }
    }
}

#[cfg(target_os = "android")]
impl Default for AndroidDrmNetworkConfig {
    fn default() -> Self {
        Self::from(DrmConfiguration::default())
    }
}

/// Audio and resource inputs for progressive-file decoding.
#[derive(Debug, Clone)]
pub struct ProgressiveDecoderConfig {
    audio_output: AudioOutput,
    audio_track_selection: AudioTrackSelection,
    maximum_audio_buffer: Duration,
    #[cfg(target_os = "android")]
    android_playback: Option<AndroidPlaybackConfig>,
}

impl ProgressiveDecoderConfig {
    /// Creates progressive decoder configuration.
    #[must_use]
    pub const fn new(
        audio_output: AudioOutput,
        audio_track_selection: AudioTrackSelection,
        maximum_audio_buffer: Duration,
    ) -> Self {
        assert!(
            !maximum_audio_buffer.is_zero(),
            "progressive audio buffer bound must be non-zero"
        );
        Self {
            audio_output,
            audio_track_selection,
            maximum_audio_buffer,
            #[cfg(target_os = "android")]
            android_playback: None,
        }
    }

    #[cfg(target_os = "android")]
    pub(crate) fn android_playback(mut self, config: AndroidPlaybackConfig) -> Self {
        self.android_playback = Some(config);
        self
    }
}

/// Output emitted by the background decoder.
#[derive(Debug)]
pub enum DecoderOutput {
    /// Container and codec initialization completed.
    Opened {
        /// Presentation duration.
        duration: Duration,
        /// Whether an audio track is present.
        has_audio: bool,
        /// Coded video dimensions.
        video_dimensions: (u32, u32),
        /// Initial video color description.
        color_info: VideoColorInfo,
    },
    /// A requested normalized seek completed.
    SeekCompleted {
        /// Actual decode restart timestamp.
        pts: Duration,
    },
    /// Current live seek window, or `None` for finite media.
    LiveWindow {
        /// Current live seek window, or `None` for finite media.
        window: Option<EngineLiveWindow>,
        /// Manifest-authorized live-latency correction rates.
        playback_rate_range: Option<EngineLivePlaybackRateRange>,
    },
    /// One decoded presentation frame.
    Frame(DecodedVideoFrame),
    /// One decoded secure-surface output awaiting presentation authorization.
    #[cfg(target_os = "android")]
    ProtectedFrame {
        /// Decoder-local output generation.
        sequence: u64,
        /// Media presentation timestamp.
        presentation_time: Duration,
    },
    /// Selectable presentation audio tracks in selection-index order.
    AudioTracks(Vec<SelectableAudioTrack>),
    /// Selectable video representations in ascending-bandwidth order.
    VideoTracks(Vec<SelectableVideoTrack>),
    /// Selectable presentation subtitle tracks in selection-index order.
    SubtitleTracks(Vec<SelectableSubtitleTrack>),
    /// Decoded subtitle cues mapped to the presentation timeline.
    SubtitleCues(Vec<SubtitleCue>),
    /// Timed event messages mapped to the presentation timeline.
    TimedMetadata(Vec<TimedMetadata>),
    /// Latest conservative transport-throughput estimate.
    NetworkThroughput(NonZeroU64),
    /// Updated persistent-license identity after an offline renewal.
    #[cfg(target_os = "android")]
    OfflineDrmKeySetChanged(OfflineDrmKeySet),
    /// Platform PCM output initialized for a segmented audio track.
    StreamingAudioOpened(StreamingAudioPlayer),
    /// Android compressed-audio offload output and authoritative playback clock.
    #[cfg(target_os = "android")]
    OffloadedAudioOpened {
        /// Renderer-facing compressed-audio control and clock.
        controller: AndroidOffloadAudioController,
        /// Concrete offload-only or A/V-tunneled architecture.
        output_path: PlaybackOutputPath,
    },
    /// One tunneled video output released to the platform Surface.
    #[cfg(target_os = "android")]
    TunneledVideoOutput {
        /// Media presentation timestamp reported by `MediaCodec`.
        presentation_time: Duration,
    },
    /// The finite presentation ended.
    Ended,
    /// A fatal container, network, or codec error.
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SendResult {
    Sent,
    Seek(Duration),
    Stop,
}

/// Owns one bounded decode thread and its control channel.
pub struct DecoderWorker {
    updates: Receiver<DecoderOutput>,
    frames: Receiver<DecodedVideoFrame>,
    pending_terminal: Option<DecoderOutput>,
    control: LatestSender<DecoderControl>,
    handle: Option<thread::JoinHandle<()>>,
    buffered_nanos: Option<Arc<AtomicU64>>,
    #[cfg(target_os = "android")]
    protected_control: Option<Sender<ProtectedOutputControl>>,
}

impl DecoderWorker {
    /// Starts progressive file decoding at normalized progress.
    pub fn spawn_progressive(
        source_path: PathBuf,
        config: ProgressiveDecoderConfig,
        start_progress: f64,
    ) -> Self {
        let (updates_tx, updates_rx) = async_channel::bounded(DECODE_EVENT_QUEUE_CAPACITY);
        let (frames_tx, frames_rx) = async_channel::bounded(PROGRESSIVE_FRAME_QUEUE_CAPACITY);
        let (control_tx, control_rx) = latest_channel();

        let handle = thread::spawn(move || {
            run_progressive_decoder(
                &source_path,
                &config,
                start_progress,
                &updates_tx,
                &frames_tx,
                &control_rx,
            );
        });

        Self {
            updates: updates_rx,
            frames: frames_rx,
            pending_terminal: None,
            control: control_tx,
            handle: Some(handle),
            buffered_nanos: None,
            #[cfg(target_os = "android")]
            protected_control: None,
        }
    }

    /// Starts HLS or DASH network, demux, and decode processing.
    pub fn spawn_segmented(
        source_url: String,
        protocol: SegmentedProtocol,
        config: SegmentedDecoderConfig,
        start_progress: f64,
    ) -> Self {
        let (updates_tx, updates_rx) = async_channel::bounded(DECODE_EVENT_QUEUE_CAPACITY);
        let (frames_tx, frames_rx) = async_channel::bounded(SEGMENTED_FRAME_QUEUE_CAPACITY);
        let worker_frames_rx = frames_rx.clone();
        let (control_tx, control_rx) = latest_channel();
        #[cfg(target_os = "android")]
        let (protected_control_tx, protected_control_rx) = async_channel::unbounded();
        let buffered_nanos = Arc::new(AtomicU64::new(0));
        let worker_buffered_nanos = Arc::clone(&buffered_nanos);

        let handle = thread::spawn(move || {
            let channels = SegmentedDecoderChannels {
                updates: &updates_tx,
                frames: &frames_tx,
                stale_frames: &worker_frames_rx,
                control: &control_rx,
                #[cfg(target_os = "android")]
                protected_control: &protected_control_rx,
            };
            run_segmented_decoder(
                &source_url,
                protocol,
                config,
                start_progress,
                channels,
                worker_buffered_nanos,
            );
        });

        Self {
            updates: updates_rx,
            frames: frames_rx,
            pending_terminal: None,
            control: control_tx,
            handle: Some(handle),
            buffered_nanos: Some(buffered_nanos),
            #[cfg(target_os = "android")]
            protected_control: Some(protected_control_tx),
        }
    }

    /// Polls the next worker output without blocking the render thread.
    pub fn try_recv(&mut self) -> Result<DecoderOutput, TryRecvError> {
        if let Some(terminal) = self.pending_terminal.take() {
            return Ok(terminal);
        }

        match self.updates.try_recv() {
            Ok(terminal @ DecoderOutput::Ended) => match self.frames.try_recv() {
                Ok(frame) => {
                    self.pending_terminal = Some(terminal);
                    Ok(DecoderOutput::Frame(frame))
                }
                Err(TryRecvError::Empty | TryRecvError::Closed) => Ok(terminal),
            },
            Ok(output) => Ok(output),
            Err(TryRecvError::Empty) => self.frames.try_recv().map(DecoderOutput::Frame),
            Err(TryRecvError::Closed) => match self.frames.try_recv() {
                Ok(frame) => Ok(DecoderOutput::Frame(frame)),
                Err(TryRecvError::Empty | TryRecvError::Closed) => Err(TryRecvError::Closed),
            },
        }
    }

    /// Requests an absolute presentation-time seek.
    pub fn request_seek(&self, position: Duration) {
        let _ = self.control.send(DecoderControl::Seek { position });
    }

    #[cfg(target_os = "android")]
    pub(crate) fn present_protected(&self, sequence: u64, delay: Duration) {
        if let Some(control) = self.protected_control.as_ref() {
            let _ = control.try_send(ProtectedOutputControl::Present { sequence, delay });
        }
    }

    #[cfg(target_os = "android")]
    pub(crate) fn discard_protected(&self, sequence: u64) {
        if let Some(control) = self.protected_control.as_ref() {
            let _ = control.try_send(ProtectedOutputControl::Discard { sequence });
        }
    }

    /// Updates the buffered duration used by adaptive selection.
    pub fn update_buffered_duration(&self, buffered: Duration) {
        let Some(buffered_nanos) = self.buffered_nanos.as_ref() else {
            return;
        };
        let nanos = u64::try_from(buffered.as_nanos()).unwrap_or(u64::MAX);
        buffered_nanos.store(nanos, Ordering::Release);
    }

    /// Stops and joins the worker.
    pub fn stop(&mut self) {
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

fn run_progressive_decoder(
    source_path: &Path,
    config: &ProgressiveDecoderConfig,
    start_progress: f64,
    updates: &Sender<DecoderOutput>,
    frames: &Sender<DecodedVideoFrame>,
    control: &Receiver<DecoderControl>,
) {
    let state = match ProgressiveDecoderState::open(
        source_path,
        config,
        start_progress,
        updates,
        control,
    ) {
        Ok(Some(state)) => state,
        Ok(None) => return,
        Err(error) => {
            report_progressive_error(updates, control, &error);
            return;
        }
    };
    state.run(config.maximum_audio_buffer, updates, frames, control);
}

fn report_progressive_error(
    updates: &Sender<DecoderOutput>,
    control: &Receiver<DecoderControl>,
    error: &VideoError,
) {
    let _ = send_responsive(updates, control, DecoderOutput::Error(error.to_string()));
}

struct ProgressiveDecoderState {
    video: Option<VideoPlayer>,
    audio: Option<ProgressiveAudioPipeline>,
    #[cfg(target_os = "android")]
    tunnel: Option<ProgressiveTunneledPipeline>,
}

impl ProgressiveDecoderState {
    fn open(
        source_path: &Path,
        config: &ProgressiveDecoderConfig,
        start_progress: f64,
        updates: &Sender<DecoderOutput>,
        control: &Receiver<DecoderControl>,
    ) -> Result<Option<Self>, VideoError> {
        #[cfg(target_os = "android")]
        if config
            .android_playback
            .as_ref()
            .is_some_and(|android| android.power == PlaybackPowerPolicy::RequireAudioVideoTunneling)
        {
            return ProgressiveTunneledPipeline::open(
                source_path,
                config,
                start_progress,
                updates,
                control,
            )
            .map(|tunnel| {
                tunnel.map(|tunnel| Self {
                    video: None,
                    audio: None,
                    tunnel: Some(tunnel),
                })
            });
        }

        let mut video = VideoPlayer::open(source_path)?;
        let Some(ProgressiveAudioOpen {
            tracks: audio_tracks,
            pipeline: mut audio,
        }) = ProgressiveAudioPipeline::open(source_path, config, control)?
        else {
            return Ok(None);
        };

        if send_responsive(updates, control, DecoderOutput::AudioTracks(audio_tracks))
            != SendResult::Sent
        {
            return Ok(None);
        }

        if let Some(audio) = audio.as_mut() {
            let output = audio
                .take_output()
                .expect("new progressive audio pipeline must retain its output owner");
            if send_responsive(updates, control, output) != SendResult::Sent {
                return Ok(None);
            }
        }

        let opening_seek = audio
            .as_mut()
            .and_then(ProgressiveAudioPipeline::take_opening_seek);
        let resolved_seek = if opening_seek.is_some() || start_progress > 0.0 {
            let requested =
                opening_seek.unwrap_or_else(|| video.duration().mul_f64(start_progress));
            let progress = normalized_time_progress(requested, video.duration());
            let position = video.seek_to_progress(progress)?;
            if let Some(audio) = audio.as_mut() {
                audio.seek_to(position)?;
            }
            Some(position)
        } else {
            None
        };

        if updates
            .send_blocking(DecoderOutput::Opened {
                duration: video.duration(),
                has_audio: audio.is_some(),
                video_dimensions: video.dimensions(),
                color_info: video.color_info(),
            })
            .is_err()
        {
            return Ok(None);
        }
        if let Some(requested) = opening_seek {
            let pts = resolved_seek.expect("opening seek must resolve a progressive timestamp");
            if send_responsive(updates, control, DecoderOutput::SeekCompleted { pts })
                != SendResult::Sent
            {
                return Ok(None);
            }
            tracing::debug!(
                requested = requested.as_secs_f64(),
                resolved = pts.as_secs_f64(),
                "applied seek received while waiting for Android video surface"
            );
        }

        Ok(Some(Self {
            video: Some(video),
            audio,
            #[cfg(target_os = "android")]
            tunnel: None,
        }))
    }

    fn seek(
        &mut self,
        position: Duration,
        updates: &Sender<DecoderOutput>,
        control: &Receiver<DecoderControl>,
    ) -> Result<SendResult, VideoError> {
        #[cfg(target_os = "android")]
        if let Some(tunnel) = self.tunnel.as_mut() {
            let pts = tunnel.seek_to(position)?;
            return Ok(send_responsive(
                updates,
                control,
                DecoderOutput::SeekCompleted { pts },
            ));
        }
        let video = self
            .video
            .as_mut()
            .expect("decoded progressive playback must retain its video decoder");
        let progress = normalized_time_progress(position, video.duration());
        let pts = video.seek_to_progress(progress)?;
        if let Some(audio) = self.audio.as_mut() {
            audio.seek_to(pts)?;
        }
        Ok(send_responsive(
            updates,
            control,
            DecoderOutput::SeekCompleted { pts },
        ))
    }

    fn run(
        mut self,
        maximum_audio_buffer: Duration,
        updates: &Sender<DecoderOutput>,
        frames: &Sender<DecodedVideoFrame>,
        control: &Receiver<DecoderControl>,
    ) {
        #[cfg(target_os = "android")]
        if let Some(tunnel) = self.tunnel.take() {
            tunnel.run(updates, control);
            return;
        }
        let mut pending_seek = None::<Duration>;
        loop {
            if !collect_progressive_control(control, &mut pending_seek) {
                return;
            }

            if let Some(position) = pending_seek.take() {
                match self.seek(position, updates, control) {
                    Ok(SendResult::Sent) => continue,
                    Ok(SendResult::Seek(next_position)) => {
                        pending_seek = Some(next_position);
                        continue;
                    }
                    Ok(SendResult::Stop) => return,
                    Err(error) => {
                        report_progressive_error(updates, control, &error);
                        return;
                    }
                }
            }

            if let Some(audio) = self.audio.as_mut() {
                match audio.fill_to_buffer_bound(maximum_audio_buffer, control) {
                    Ok(SendResult::Sent) => {}
                    Ok(SendResult::Seek(position)) => {
                        pending_seek = Some(position);
                        continue;
                    }
                    Ok(SendResult::Stop) => return,
                    Err(error) => {
                        report_progressive_error(updates, control, &error);
                        return;
                    }
                }
            }

            let video = self
                .video
                .as_mut()
                .expect("decoded progressive playback must retain its video decoder");
            match video.next_frame() {
                Ok(Some(frame)) => match send_frame_responsive(frames, control, frame) {
                    SendResult::Sent => {}
                    SendResult::Seek(position) => pending_seek = Some(position),
                    SendResult::Stop => return,
                },
                Ok(None) => {
                    if !self.finish_audio(maximum_audio_buffer, control, &mut pending_seek, updates)
                    {
                        return;
                    }
                    if pending_seek.is_none() {
                        let _ = send_responsive(updates, control, DecoderOutput::Ended);
                        return;
                    }
                }
                Err(error) => {
                    report_progressive_error(updates, control, &error);
                    return;
                }
            }
        }
    }

    fn finish_audio(
        &mut self,
        maximum_audio_buffer: Duration,
        control: &Receiver<DecoderControl>,
        pending_seek: &mut Option<Duration>,
        updates: &Sender<DecoderOutput>,
    ) -> bool {
        let Some(audio) = self.audio.as_mut() else {
            return true;
        };
        match audio.finish_input(maximum_audio_buffer, control) {
            Ok(SendResult::Sent) => true,
            Ok(SendResult::Seek(position)) => {
                *pending_seek = Some(position);
                true
            }
            Ok(SendResult::Stop) => false,
            Err(error) => {
                report_progressive_error(updates, control, &error);
                false
            }
        }
    }
}

fn collect_progressive_control(
    control: &Receiver<DecoderControl>,
    pending_seek: &mut Option<Duration>,
) -> bool {
    loop {
        match control.try_recv() {
            Ok(DecoderControl::Stop) | Err(TryRecvError::Closed) => return false,
            Ok(DecoderControl::Seek { position }) => {
                *pending_seek = Some(position);
            }
            Err(TryRecvError::Empty) => return true,
        }
    }
}

enum ProgressiveAudioPipeline {
    Pcm(ProgressivePcmAudioPipeline),
    #[cfg(target_os = "android")]
    Offloaded(ProgressiveOffloadedAudioPipeline),
}

struct ProgressiveAudioOpen {
    tracks: Vec<SelectableAudioTrack>,
    pipeline: Option<ProgressiveAudioPipeline>,
}

struct SelectedProgressiveAudio {
    tracks: Vec<SelectableAudioTrack>,
    selection: Option<ProgressiveAudioSelection>,
}

struct ProgressiveAudioSelection {
    reader: ProgressiveTrackReader,
    track: TrackInfo,
}

impl ProgressiveAudioPipeline {
    fn open(
        path: &Path,
        config: &ProgressiveDecoderConfig,
        control: &Receiver<DecoderControl>,
    ) -> Result<Option<ProgressiveAudioOpen>, VideoError> {
        #[cfg(target_os = "android")]
        if let Some(android) = config.android_playback.as_ref()
            && android.power == PlaybackPowerPolicy::RequireAudioOffload
        {
            android.validate_power_requirements()?;
            let Some((port, opening_seek)) =
                wait_for_progressive_video_surface(&android.surfaces, control)
            else {
                return Ok(None);
            };
            return ProgressiveOffloadedAudioPipeline::open(path, config, &port, opening_seek).map(
                |(tracks, pipeline)| {
                    Some(ProgressiveAudioOpen {
                        tracks,
                        pipeline: pipeline.map(Self::Offloaded),
                    })
                },
            );
        }

        #[cfg(not(target_os = "android"))]
        let _ = control;

        ProgressivePcmAudioPipeline::open(path, config).map(|(tracks, pipeline)| {
            Some(ProgressiveAudioOpen {
                tracks,
                pipeline: pipeline.map(Self::Pcm),
            })
        })
    }

    fn take_output(&mut self) -> Option<DecoderOutput> {
        match self {
            Self::Pcm(pipeline) => pipeline
                .take_owner()
                .map(DecoderOutput::StreamingAudioOpened),
            #[cfg(target_os = "android")]
            Self::Offloaded(pipeline) => {
                pipeline
                    .take_controller()
                    .map(|controller| DecoderOutput::OffloadedAudioOpened {
                        controller,
                        output_path: PlaybackOutputPath::AudioOffload,
                    })
            }
        }
    }

    const fn take_opening_seek(&mut self) -> Option<Duration> {
        match self {
            Self::Pcm(_) => None,
            #[cfg(target_os = "android")]
            Self::Offloaded(pipeline) => pipeline.opening_seek.take(),
        }
    }

    fn seek_to(&mut self, position: Duration) -> Result<(), VideoError> {
        match self {
            Self::Pcm(pipeline) => pipeline.seek_to(position),
            #[cfg(target_os = "android")]
            Self::Offloaded(pipeline) => pipeline.seek_to(position),
        }
    }

    fn fill_to_buffer_bound(
        &mut self,
        maximum_buffer: Duration,
        control: &Receiver<DecoderControl>,
    ) -> Result<SendResult, VideoError> {
        match self {
            Self::Pcm(pipeline) => pipeline.fill_to_buffer_bound(maximum_buffer, control),
            #[cfg(target_os = "android")]
            Self::Offloaded(pipeline) => pipeline.fill_to_buffer_bound(maximum_buffer, control),
        }
    }

    fn finish_input(
        &mut self,
        maximum_buffer: Duration,
        control: &Receiver<DecoderControl>,
    ) -> Result<SendResult, VideoError> {
        match self {
            Self::Pcm(pipeline) => pipeline.finish_input(maximum_buffer, control),
            #[cfg(target_os = "android")]
            Self::Offloaded(pipeline) => pipeline.finish_input(control),
        }
    }
}

fn open_selected_progressive_audio(
    path: &Path,
    selection: AudioTrackSelection,
) -> Result<SelectedProgressiveAudio, VideoError> {
    let mut reader = ProgressiveTrackReader::open(path, TrackKind::Audio)?;
    let tracks = reader
        .tracks()
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let language = track.language().trim();
            let language = (!language.is_empty() && language != "und").then(|| language.to_owned());
            let label = language
                .clone()
                .unwrap_or_else(|| format!("Audio {}", index + 1));
            SelectableAudioTrack::new(label, language, Vec::new())
        })
        .collect::<Vec<_>>();
    let selected_index = match selection {
        AudioTrackSelection::Auto => {
            if reader.tracks().is_empty() {
                return Ok(SelectedProgressiveAudio {
                    tracks,
                    selection: None,
                });
            }
            0
        }
        AudioTrackSelection::Track(index) => index,
    };
    reader.select_track(selected_index)?;
    let track = reader
        .selected_track()
        .expect("successfully selected progressive audio track must exist")
        .info()
        .clone();
    Ok(SelectedProgressiveAudio {
        tracks,
        selection: Some(ProgressiveAudioSelection { reader, track }),
    })
}

struct ProgressivePcmAudioPipeline {
    reader: ProgressiveTrackReader,
    decoder: AudioTrackDecoder,
    producer: StreamingAudioProducer,
    owner: Option<StreamingAudioPlayer>,
    input_finished: bool,
}

impl ProgressivePcmAudioPipeline {
    fn open(
        path: &Path,
        config: &ProgressiveDecoderConfig,
    ) -> Result<(Vec<SelectableAudioTrack>, Option<Self>), VideoError> {
        let selected = open_selected_progressive_audio(path, config.audio_track_selection)?;
        let Some(ProgressiveAudioSelection { reader, track }) = selected.selection else {
            return Ok((selected.tracks, None));
        };
        let decoder = AudioTrackDecoder::new(track)?;
        let owner = StreamingAudioPlayer::new_with_output(&config.audio_output)
            .map_err(|error| VideoError::Codec(error.to_string()))?;
        let producer = owner.producer();
        Ok((
            selected.tracks,
            Some(Self {
                reader,
                decoder,
                producer,
                owner: Some(owner),
                input_finished: false,
            }),
        ))
    }

    const fn take_owner(&mut self) -> Option<StreamingAudioPlayer> {
        self.owner.take()
    }

    fn seek_to(&mut self, position: Duration) -> Result<(), VideoError> {
        self.reader.seek_to(position)?;
        self.decoder = AudioTrackDecoder::new(
            self.reader
                .selected_track()
                .expect("active progressive audio pipeline must retain its selected track")
                .info()
                .clone(),
        )?;
        self.producer
            .reset_to(position)
            .map_err(|error| VideoError::Codec(error.to_string()))?;
        self.input_finished = false;
        Ok(())
    }

    fn fill_to_buffer_bound(
        &mut self,
        maximum_buffer: Duration,
        control: &Receiver<DecoderControl>,
    ) -> Result<SendResult, VideoError> {
        while !self.input_finished && self.producer.buffered_duration() <= maximum_buffer {
            match control.try_recv() {
                Ok(DecoderControl::Stop) | Err(TryRecvError::Closed) => {
                    return Ok(SendResult::Stop);
                }
                Ok(DecoderControl::Seek { position }) => {
                    return Ok(SendResult::Seek(position));
                }
                Err(TryRecvError::Empty) => {}
            }
            let Some(sample) = self.reader.read_sample()? else {
                self.finish_decoder()?;
                break;
            };
            for frame in self.decoder.decode(&sample)? {
                self.producer
                    .enqueue(frame)
                    .map_err(|error| VideoError::Codec(error.to_string()))?;
            }
        }
        Ok(SendResult::Sent)
    }

    fn finish_input(
        &mut self,
        maximum_buffer: Duration,
        control: &Receiver<DecoderControl>,
    ) -> Result<SendResult, VideoError> {
        while !self.input_finished {
            match self.fill_to_buffer_bound(maximum_buffer, control)? {
                SendResult::Sent => {}
                result => return Ok(result),
            }
            if self.input_finished {
                break;
            }
            match self.wait_for_buffer_progress(control) {
                SendResult::Sent => {}
                result => return Ok(result),
            }
        }
        Ok(SendResult::Sent)
    }

    fn wait_for_buffer_progress(&self, control: &Receiver<DecoderControl>) -> SendResult {
        let buffer_progress = self.producer.buffer_progress_receiver();
        let progressed = buffer_progress.recv();
        let command = control.recv();
        futures::pin_mut!(progressed, command);
        match futures::executor::block_on(select(progressed, command)) {
            Either::Left((Ok(()), _)) => SendResult::Sent,
            Either::Right((Ok(DecoderControl::Seek { position }), _)) => SendResult::Seek(position),
            Either::Left((Err(_), _)) | Either::Right((Err(_) | Ok(DecoderControl::Stop), _)) => {
                SendResult::Stop
            }
        }
    }

    fn finish_decoder(&mut self) -> Result<(), VideoError> {
        for frame in self.decoder.finish()? {
            self.producer
                .enqueue(frame)
                .map_err(|error| VideoError::Codec(error.to_string()))?;
        }
        self.producer
            .finish()
            .map_err(|error| VideoError::Codec(error.to_string()))?;
        self.input_finished = true;
        Ok(())
    }
}

#[cfg(target_os = "android")]
struct ProgressiveOffloadedAudioPipeline {
    reader: ProgressiveTrackReader,
    playback: AndroidOffloadAudioPlayback,
    controller: Option<AndroidOffloadAudioController>,
    input_finished: bool,
    opening_seek: Option<Duration>,
}

#[cfg(target_os = "android")]
impl ProgressiveOffloadedAudioPipeline {
    fn open(
        path: &Path,
        config: &ProgressiveDecoderConfig,
        port: &AndroidVideoSurfacePort,
        opening_seek: Option<Duration>,
    ) -> Result<(Vec<SelectableAudioTrack>, Option<Self>), VideoError> {
        let selected = open_selected_progressive_audio(path, config.audio_track_selection)?;
        let Some(ProgressiveAudioSelection { reader, track }) = selected.selection else {
            return Err(VideoError::Unsupported(String::from(
                "required Android audio offload needs a selected compressed audio track",
            )));
        };
        let context = port.playback_context()?;
        let playback = AndroidOffloadAudioPlayback::new(&context, track, false)?;
        let controller = playback.controller();
        Ok((
            selected.tracks,
            Some(Self {
                reader,
                playback,
                controller: Some(controller),
                input_finished: false,
                opening_seek,
            }),
        ))
    }

    const fn take_controller(&mut self) -> Option<AndroidOffloadAudioController> {
        self.controller.take()
    }

    fn seek_to(&mut self, position: Duration) -> Result<(), VideoError> {
        let pts = self.reader.seek_to(position)?;
        self.playback.flush(pts)?;
        self.input_finished = false;
        Ok(())
    }

    fn fill_to_buffer_bound(
        &mut self,
        maximum_buffer: Duration,
        control: &Receiver<DecoderControl>,
    ) -> Result<SendResult, VideoError> {
        while !self.input_finished
            && self.playback.controller().buffered_duration()? <= maximum_buffer
        {
            match collect_one_progressive_control(control) {
                SendResult::Sent => {}
                result => return Ok(result),
            }
            let Some(sample) = self.reader.read_sample()? else {
                self.playback.controller().finish()?;
                self.input_finished = true;
                break;
            };
            self.playback.queue(&sample)?;
        }
        Ok(SendResult::Sent)
    }

    fn finish_input(
        &mut self,
        control: &Receiver<DecoderControl>,
    ) -> Result<SendResult, VideoError> {
        while !self.input_finished {
            match collect_one_progressive_control(control) {
                SendResult::Sent => {}
                result => return Ok(result),
            }
            let Some(sample) = self.reader.read_sample()? else {
                self.playback.controller().finish()?;
                self.input_finished = true;
                break;
            };
            self.playback.queue(&sample)?;
        }
        Ok(SendResult::Sent)
    }
}

#[cfg(target_os = "android")]
struct ProgressiveTunneledPipeline {
    audio_reader: ProgressiveTrackReader,
    video_reader: ProgressiveTrackReader,
    pending_audio: Option<EncodedSample>,
    pending_video: Option<EncodedSample>,
    playback: AndroidTunneledPlayback,
    surface_port: AndroidVideoSurfacePort,
    restart_position: Duration,
}

#[cfg(target_os = "android")]
impl ProgressiveTunneledPipeline {
    fn open(
        path: &Path,
        config: &ProgressiveDecoderConfig,
        start_progress: f64,
        updates: &Sender<DecoderOutput>,
        control: &Receiver<DecoderControl>,
    ) -> Result<Option<Self>, VideoError> {
        let android = config
            .android_playback
            .as_ref()
            .expect("required progressive tunnel must retain Android configuration");
        android.validate_power_requirements()?;
        let selected_audio = open_selected_progressive_audio(path, config.audio_track_selection)?;
        let Some(ProgressiveAudioSelection {
            reader: audio_reader,
            track: audio_track,
        }) = selected_audio.selection
        else {
            return Err(VideoError::Unsupported(String::from(
                "required Android A/V tunneling needs a selected compressed audio track",
            )));
        };
        let mut video_reader = ProgressiveTrackReader::open(path, TrackKind::Video)?;
        if video_reader.tracks().is_empty() {
            return Err(VideoError::Container(String::from(
                "progressive presentation has no supported video track",
            )));
        }
        video_reader.select_track(0)?;
        let video = video_reader
            .selected_track()
            .expect("selected progressive video track must exist");
        let video_track = video.info().clone();
        let duration = video.duration();
        let dimensions = video_track
            .video_dimensions()
            .expect("validated progressive video track must declare dimensions");
        let color_info = video_track.video_color_info().unwrap_or_default();
        let Some((surface_port, opening_seek)) =
            wait_for_progressive_video_surface(&android.surfaces, control)
        else {
            return Ok(None);
        };
        let surface = surface_port.acquire_clear()?;
        let playback = AndroidTunneledPlayback::new(surface, video_track, audio_track)?;
        let mut pipeline = Self {
            audio_reader,
            video_reader,
            pending_audio: None,
            pending_video: None,
            playback,
            surface_port,
            restart_position: Duration::ZERO,
        };
        let requested = opening_seek.unwrap_or_else(|| duration.mul_f64(start_progress));
        let resolved_seek = (opening_seek.is_some() || start_progress > 0.0)
            .then(|| pipeline.seek_to(requested))
            .transpose()?;

        if send_responsive(
            updates,
            control,
            DecoderOutput::AudioTracks(selected_audio.tracks),
        ) != SendResult::Sent
        {
            return Ok(None);
        }
        if send_responsive(
            updates,
            control,
            DecoderOutput::OffloadedAudioOpened {
                controller: pipeline.playback.audio_controller(),
                output_path: PlaybackOutputPath::AudioVideoTunneling,
            },
        ) != SendResult::Sent
        {
            return Ok(None);
        }
        if send_responsive(
            updates,
            control,
            DecoderOutput::Opened {
                duration,
                has_audio: true,
                video_dimensions: (dimensions.width.get(), dimensions.height.get()),
                color_info,
            },
        ) != SendResult::Sent
        {
            return Ok(None);
        }
        if opening_seek.is_some()
            && send_responsive(
                updates,
                control,
                DecoderOutput::SeekCompleted {
                    pts: resolved_seek.expect("opening tunnel seek must resolve a timestamp"),
                },
            ) != SendResult::Sent
        {
            return Ok(None);
        }
        Ok(Some(pipeline))
    }

    fn seek_to(&mut self, position: Duration) -> Result<Duration, VideoError> {
        let pts = self.video_reader.seek_to_keyframe(position)?;
        self.audio_reader.seek_to(pts)?;
        self.pending_audio = None;
        self.pending_video = None;
        self.playback.flush(pts)?;
        self.restart_position = pts;
        Ok(pts)
    }

    fn run(mut self, updates: &Sender<DecoderOutput>, control: &Receiver<DecoderControl>) {
        loop {
            match collect_one_progressive_control(control) {
                SendResult::Sent => {}
                SendResult::Seek(position) => match self.seek_to(position) {
                    Ok(pts) => {
                        if send_responsive(updates, control, DecoderOutput::SeekCompleted { pts })
                            != SendResult::Sent
                        {
                            return;
                        }
                        continue;
                    }
                    Err(error) => {
                        report_progressive_error(updates, control, &error);
                        return;
                    }
                },
                SendResult::Stop => return,
            }
            match self.queue_next(updates, control) {
                Ok(SendResult::Sent) => {}
                Ok(SendResult::Seek(position)) => match self.seek_to(position) {
                    Ok(pts) => {
                        if send_responsive(updates, control, DecoderOutput::SeekCompleted { pts })
                            != SendResult::Sent
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        report_progressive_error(updates, control, &error);
                        return;
                    }
                },
                Ok(SendResult::Stop) => return,
                Err(error) => {
                    report_progressive_error(updates, control, &error);
                    return;
                }
            }
        }
    }

    fn queue_next(
        &mut self,
        updates: &Sender<DecoderOutput>,
        control: &Receiver<DecoderControl>,
    ) -> Result<SendResult, VideoError> {
        if let Some(result) = self.poll_surface_lifecycle(updates, control)? {
            return Ok(result);
        }
        if self.pending_audio.is_none() {
            self.pending_audio = self.audio_reader.read_sample()?;
        }
        if self.pending_video.is_none() {
            self.pending_video = self.video_reader.read_sample()?;
        }
        if self.pending_audio.is_none() && self.pending_video.is_none() {
            let outputs = match self.playback.finish_input() {
                Ok(outputs) => outputs,
                Err(error) => {
                    return self.poll_surface_lifecycle(updates, control)?.ok_or(error);
                }
            };
            for presentation_time in outputs {
                self.restart_position = presentation_time;
                let result = send_responsive(
                    updates,
                    control,
                    DecoderOutput::TunneledVideoOutput { presentation_time },
                );
                if result != SendResult::Sent {
                    return Ok(result);
                }
            }
            let _ = send_responsive(updates, control, DecoderOutput::Ended);
            return Ok(SendResult::Stop);
        }
        let take_audio = match (&self.pending_audio, &self.pending_video) {
            (Some(audio), Some(video)) => {
                audio.decode_time().to_duration()? <= video.decode_time().to_duration()?
            }
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => unreachable!("tunnel exhaustion returns before sample selection"),
        };
        let sample = if take_audio {
            self.pending_audio
                .take()
                .expect("selected pending audio sample must exist")
        } else {
            self.pending_video
                .take()
                .expect("selected pending video sample must exist")
        };
        if !take_audio {
            self.restart_position = sample.presentation_time().to_duration()?;
        }
        let outputs = match self.playback.queue(&sample) {
            Ok(outputs) => outputs,
            Err(error) => {
                return self.poll_surface_lifecycle(updates, control)?.ok_or(error);
            }
        };
        for presentation_time in outputs {
            self.restart_position = presentation_time;
            let result = send_responsive(
                updates,
                control,
                DecoderOutput::TunneledVideoOutput { presentation_time },
            );
            if result != SendResult::Sent {
                return Ok(result);
            }
        }
        Ok(SendResult::Sent)
    }

    fn poll_surface_lifecycle(
        &mut self,
        updates: &Sender<DecoderOutput>,
        control: &Receiver<DecoderControl>,
    ) -> Result<Option<SendResult>, VideoError> {
        match self.surface_port.poll_lifecycle() {
            Some(AndroidVideoSurfaceLifecycle::Destroyed) => {
                self.recover_destroyed_surface(updates, control).map(Some)
            }
            Some(AndroidVideoSurfaceLifecycle::HostDisposed) => Err(video_surface_disposed()),
            None => Ok(None),
        }
    }

    fn recover_destroyed_surface(
        &mut self,
        updates: &Sender<DecoderOutput>,
        control: &Receiver<DecoderControl>,
    ) -> Result<SendResult, VideoError> {
        let video_track = self.playback.video_track_info().clone();
        let audio_track = self.playback.audio_track_info().clone();
        let surface = self.surface_port.acquire_clear()?;
        self.playback = AndroidTunneledPlayback::new(surface, video_track, audio_track)?;
        let restart_position = self.restart_position;
        self.seek_to(restart_position)?;
        Ok(send_responsive(
            updates,
            control,
            DecoderOutput::OffloadedAudioOpened {
                controller: self.playback.audio_controller(),
                output_path: PlaybackOutputPath::AudioVideoTunneling,
            },
        ))
    }
}

#[cfg(target_os = "android")]
impl Drop for ProgressiveTunneledPipeline {
    fn drop(&mut self) {
        if let Err(error) = self.surface_port.deactivate() {
            tracing::error!(%error, "failed to deactivate progressive tunnel Surface");
        }
    }
}

#[cfg(target_os = "android")]
fn collect_one_progressive_control(control: &Receiver<DecoderControl>) -> SendResult {
    match control.try_recv() {
        Ok(DecoderControl::Seek { position }) => SendResult::Seek(position),
        Ok(DecoderControl::Stop) | Err(TryRecvError::Closed) => SendResult::Stop,
        Err(TryRecvError::Empty) => SendResult::Sent,
    }
}

#[cfg(target_os = "android")]
fn wait_for_progressive_video_surface(
    surfaces: &AndroidVideoSurfaceReceiver,
    control: &Receiver<DecoderControl>,
) -> Option<(AndroidVideoSurfacePort, Option<Duration>)> {
    let mut pending_seek = None;
    loop {
        match wait_for_video_surface_host(surfaces, control) {
            VideoSurfaceHostWait::Ready(port) => return Some((port, pending_seek)),
            VideoSurfaceHostWait::Command(SendResult::Seek(position)) => {
                pending_seek = Some(position);
            }
            VideoSurfaceHostWait::Command(SendResult::Stop) => return None,
            VideoSurfaceHostWait::Command(SendResult::Sent) => {
                unreachable!("surface wait cannot complete with a sent decoder update")
            }
        }
    }
}

struct SegmentBatch {
    duration: Duration,
    network_throughput: Option<NonZeroU64>,
    tracks: Vec<TrackInfo>,
    samples: Vec<EncodedSample>,
    subtitle_cues: Vec<SubtitleCue>,
    timed_metadata: Vec<TimedMetadata>,
    #[cfg(target_os = "android")]
    protection_init_data: Vec<ProtectionInitData>,
}

enum SessionPoll {
    Ready(SegmentBatch),
    Awaiting(Duration),
    Ended,
}

enum PrefetchWait {
    Poll(Result<SessionPoll, VideoError>),
    Command(PrefetchCommand),
    Closed,
}

#[derive(Debug, Clone, Copy)]
enum PrefetchCommand {
    Stop,
    Seek { position: Duration },
}

#[derive(Debug, Clone, Copy)]
struct PrefetchConsumed {
    generation: u64,
    duration: Duration,
}

enum PrefetchOutput {
    Ready {
        generation: u64,
        batch: SegmentBatch,
    },
    SeekCompleted {
        generation: u64,
        pts: Duration,
    },
    LiveWindow {
        window: Option<EngineLiveWindow>,
        playback_rate_range: Option<EngineLivePlaybackRateRange>,
    },
    Ended {
        generation: u64,
    },
    Error(VideoError),
}

enum DecoderWait {
    Output(PrefetchOutput),
    Command(DecoderControl),
    Closed,
}

#[derive(Debug, Default)]
struct PublishedLiveWindow {
    initialized: bool,
    value: Option<EngineLiveWindow>,
    playback_rate_range: Option<EngineLivePlaybackRateRange>,
}

type SessionFuture<'a> = Pin<Box<dyn Future<Output = Result<SessionPoll, VideoError>> + 'a>>;

trait SegmentSession: Send {
    fn duration(&self) -> Option<Duration>;
    fn live_window(&self) -> Result<Option<EngineLiveWindow>, VideoError>;
    fn live_playback_rate_range(&self) -> Result<Option<EngineLivePlaybackRateRange>, VideoError>;
    fn audio_tracks(&self) -> Vec<SelectableAudioTrack>;
    fn video_tracks(&self) -> Vec<SelectableVideoTrack>;
    fn subtitle_tracks(&self) -> Vec<SelectableSubtitleTrack>;
    fn next(&mut self, buffered: Duration) -> SessionFuture<'_>;
    fn seek(&mut self, position: Duration) -> Result<Duration, VideoError>;
}

struct HlsSession(HlsPlaybackSession);

impl SegmentSession for HlsSession {
    fn duration(&self) -> Option<Duration> {
        self.0.duration()
    }

    fn live_window(&self) -> Result<Option<EngineLiveWindow>, VideoError> {
        Ok(self.0.live_window())
    }

    fn live_playback_rate_range(&self) -> Result<Option<EngineLivePlaybackRateRange>, VideoError> {
        Ok(None)
    }

    fn audio_tracks(&self) -> Vec<SelectableAudioTrack> {
        self.0.audio_tracks()
    }

    fn video_tracks(&self) -> Vec<SelectableVideoTrack> {
        self.0.video_tracks()
    }

    fn subtitle_tracks(&self) -> Vec<SelectableSubtitleTrack> {
        self.0.subtitle_tracks()
    }

    fn next(&mut self, buffered: Duration) -> SessionFuture<'_> {
        Box::pin(async move {
            match self.0.next_segment(buffered, supports_hls_variant).await? {
                HlsSegmentPoll::Ready(segment) => {
                    let duration = segment.duration();
                    let network_throughput = Some(segment.estimated_bits_per_second());
                    let tracks = segment.tracks().to_vec();
                    #[cfg(target_os = "android")]
                    let protection_init_data = segment.protection_init_data().to_vec();
                    let (samples, timed_metadata) = segment.into_media();
                    Ok(SessionPoll::Ready(SegmentBatch {
                        duration,
                        network_throughput,
                        tracks,
                        samples,
                        subtitle_cues: Vec::new(),
                        timed_metadata,
                        #[cfg(target_os = "android")]
                        protection_init_data,
                    }))
                }
                HlsSegmentPoll::Subtitles(segment) => {
                    let duration = segment.duration();
                    let (subtitle_cues, timed_metadata) = segment.into_media();
                    Ok(SessionPoll::Ready(SegmentBatch {
                        duration,
                        network_throughput: None,
                        tracks: Vec::new(),
                        samples: Vec::new(),
                        subtitle_cues,
                        timed_metadata,
                        #[cfg(target_os = "android")]
                        protection_init_data: Vec::new(),
                    }))
                }
                HlsSegmentPoll::AwaitingPlaylist { retry_after } => {
                    Ok(SessionPoll::Awaiting(retry_after))
                }
                HlsSegmentPoll::EndOfStream => Ok(SessionPoll::Ended),
                _ => Err(VideoError::Unsupported(String::from(
                    "HLS session returned a poll result unsupported by this player version",
                ))),
            }
        })
    }

    fn seek(&mut self, position: Duration) -> Result<Duration, VideoError> {
        if self.0.is_live() {
            self.0.seek_to_live_position(position)
        } else {
            let duration = self.0.duration().ok_or_else(|| {
                VideoError::Streaming(String::from("finite HLS duration is unavailable"))
            })?;
            self.0
                .seek_to_progress(normalized_time_progress(position, duration))
        }
    }
}

struct DashSession(DashPlaybackSession);

impl SegmentSession for DashSession {
    fn duration(&self) -> Option<Duration> {
        self.0.duration()
    }

    fn live_window(&self) -> Result<Option<EngineLiveWindow>, VideoError> {
        self.0.live_window_at(SystemTime::now())
    }

    fn live_playback_rate_range(&self) -> Result<Option<EngineLivePlaybackRateRange>, VideoError> {
        self.0.live_playback_rate_range()
    }

    fn audio_tracks(&self) -> Vec<SelectableAudioTrack> {
        self.0.audio_tracks()
    }

    fn video_tracks(&self) -> Vec<SelectableVideoTrack> {
        self.0.video_tracks()
    }

    fn subtitle_tracks(&self) -> Vec<SelectableSubtitleTrack> {
        self.0.subtitle_tracks()
    }

    fn next(&mut self, buffered: Duration) -> SessionFuture<'_> {
        Box::pin(async move {
            match self
                .0
                .next_segment_at(SystemTime::now(), buffered, supports_dash_representation)
                .await?
            {
                DashSegmentPoll::Ready(segment) => {
                    let duration = segment.duration();
                    let network_throughput = Some(segment.estimated_bits_per_second());
                    let tracks = segment.tracks().to_vec();
                    #[cfg(target_os = "android")]
                    let protection_init_data = segment.protection_init_data().to_vec();
                    let (samples, timed_metadata) = segment.into_media();
                    Ok(SessionPoll::Ready(SegmentBatch {
                        duration,
                        network_throughput,
                        tracks,
                        samples,
                        subtitle_cues: Vec::new(),
                        timed_metadata,
                        #[cfg(target_os = "android")]
                        protection_init_data,
                    }))
                }
                DashSegmentPoll::Subtitles(segment) => {
                    let duration = segment.duration();
                    let (subtitle_cues, timed_metadata) = segment.into_media();
                    Ok(SessionPoll::Ready(SegmentBatch {
                        duration,
                        network_throughput: None,
                        tracks: Vec::new(),
                        samples: Vec::new(),
                        subtitle_cues,
                        timed_metadata,
                        #[cfg(target_os = "android")]
                        protection_init_data: Vec::new(),
                    }))
                }
                DashSegmentPoll::AwaitingManifest { retry_after } => {
                    Ok(SessionPoll::Awaiting(retry_after))
                }
                DashSegmentPoll::EndOfStream => Ok(SessionPoll::Ended),
                _ => Err(VideoError::Unsupported(String::from(
                    "DASH session returned a poll result unsupported by this player version",
                ))),
            }
        })
    }

    fn seek(&mut self, position: Duration) -> Result<Duration, VideoError> {
        if self.0.is_live() {
            self.0.seek_to_live_position_at(
                position,
                SystemTime::now(),
                supports_dash_representation,
            )
        } else {
            let duration = self.0.duration().ok_or_else(|| {
                VideoError::Streaming(String::from("static DASH duration is unavailable"))
            })?;
            self.0.seek_to_progress_at(
                normalized_time_progress(position, duration),
                SystemTime::now(),
                supports_dash_representation,
            )
        }
    }
}

#[cfg(target_os = "android")]
struct AndroidProtectedDecoders {
    config: Option<AndroidPlaybackConfig>,
    video_track: Option<TrackInfo>,
    video_decoder: Option<AndroidProtectedVideoDecoder>,
    audio_track: Option<TrackInfo>,
    audio_decoder: Option<AndroidProtectedAudioDecoder>,
    surface_port: Option<AndroidVideoSurfacePort>,
    restart_position: Duration,
}

#[cfg(target_os = "android")]
impl AndroidProtectedDecoders {
    const fn new(config: Option<AndroidPlaybackConfig>) -> Self {
        Self {
            config,
            video_track: None,
            video_decoder: None,
            audio_track: None,
            audio_decoder: None,
            surface_port: None,
            restart_position: Duration::ZERO,
        }
    }

    const fn video_track_info(&self) -> Option<&TrackInfo> {
        self.video_track.as_ref()
    }

    const fn audio_track_info(&self) -> Option<&TrackInfo> {
        self.audio_track.as_ref()
    }

    fn update_video_track(&mut self, track: TrackInfo) -> Result<(), VideoError> {
        if self.video_track.as_ref() == Some(&track) {
            return Ok(());
        }
        self.reset_video_decoder()?;
        self.video_track = Some(track);
        Ok(())
    }

    fn update_audio_track(&mut self, track: TrackInfo) {
        if self.audio_track.as_ref() == Some(&track) {
            return;
        }
        self.audio_decoder = None;
        self.audio_track = Some(track);
    }

    fn clear_video(&mut self) -> Result<(), VideoError> {
        self.reset_video_decoder()?;
        self.video_track = None;
        Ok(())
    }

    fn clear_audio(&mut self) {
        self.audio_decoder = None;
        self.audio_track = None;
    }

    fn reset_video_decoder(&mut self) -> Result<(), VideoError> {
        self.video_decoder = None;
        if let Some(port) = self.surface_port.as_ref() {
            port.deactivate()?;
        }
        Ok(())
    }

    fn flush(&mut self, position: Duration) -> Result<(), VideoError> {
        if let Some(decoder) = self.video_decoder.as_mut() {
            decoder.flush()?;
        }
        if let Some(decoder) = self.audio_decoder.as_mut() {
            decoder.flush()?;
        }
        self.restart_position = position;
        Ok(())
    }

    fn ensure_decoders(
        &mut self,
        init_data: &[ProtectionInitData],
        control: &Receiver<DecoderControl>,
    ) -> Result<Option<SendResult>, VideoError> {
        let needs_video = self.video_track.is_some() && self.video_decoder.is_none();
        let needs_audio = self.audio_track.is_some() && self.audio_decoder.is_none();
        if !needs_video && !needs_audio {
            return Ok(None);
        }
        let surfaces = self
            .config
            .as_ref()
            .ok_or_else(|| {
                VideoError::Unsupported(String::from(
                    "Android protected video requires a secure WaterUI surface host",
                ))
            })?
            .surfaces
            .clone();
        if self.surface_port.is_none() {
            let port = match wait_for_video_surface_host(&surfaces, control) {
                VideoSurfaceHostWait::Ready(port) => port,
                VideoSurfaceHostWait::Command(result) => return Ok(Some(result)),
            };
            self.surface_port = Some(port);
        }
        let port = self
            .surface_port
            .as_ref()
            .expect("protected decoder setup must retain its Surface port");
        let config = self
            .config
            .as_ref()
            .expect("protected decoder setup must retain its configuration");
        if needs_video {
            let surface = port.acquire_protected()?;
            let track = self
                .video_track
                .as_ref()
                .expect("protected decoder setup must retain its video track")
                .clone();
            let init_data = select_android_protection_init_data(&track, init_data, |system_id| {
                surface.supports_system_id(system_id)
            })?;
            self.video_decoder = Some(configure_android_protected_decoder(
                config, surface, track, init_data,
            )?);
        }
        if needs_audio {
            let context = port.drm_context()?;
            let track = self
                .audio_track
                .as_ref()
                .expect("protected decoder setup must retain its audio track")
                .clone();
            let init_data = select_android_protection_init_data(&track, init_data, |system_id| {
                context.supports_system_id(system_id)
            })?;
            self.audio_decoder = Some(configure_android_protected_audio_decoder(
                config, &context, track, init_data,
            )?);
        }
        Ok(None)
    }

    fn process(
        &mut self,
        samples: &[EncodedSample],
        channels: SegmentedDecoderChannels<'_>,
    ) -> Result<SendResult, VideoError> {
        let lifecycle = self
            .surface_port
            .as_ref()
            .and_then(AndroidVideoSurfacePort::poll_lifecycle);
        if let Some(lifecycle) = lifecycle {
            return self.handle_surface_lifecycle(lifecycle);
        }
        let (Some(decoder), Some(port)) = (&mut self.video_decoder, &self.surface_port) else {
            return Ok(SendResult::Sent);
        };
        let track_id = decoder.track_info().id();
        let mut surface_destroyed_at = None;
        for sample in samples
            .iter()
            .filter(|sample| sample.track_id() == track_id)
        {
            self.restart_position = sample.presentation_time().to_duration()?;
            match port.poll_lifecycle() {
                Some(AndroidVideoSurfaceLifecycle::Destroyed) => {
                    surface_destroyed_at = Some(self.restart_position);
                    break;
                }
                Some(AndroidVideoSurfaceLifecycle::HostDisposed) => {
                    return Err(video_surface_disposed());
                }
                None => {}
            }
            decoder.queue(sample)?;
            match drain_available_protected_outputs(decoder, port, channels)? {
                ProtectedDrain::Continue => {}
                ProtectedDrain::Command(result) => return Ok(result),
                ProtectedDrain::SurfaceDestroyed(position) => {
                    surface_destroyed_at = Some(position);
                    break;
                }
                ProtectedDrain::HostDisposed => return Err(video_surface_disposed()),
            }
        }
        if let Some(position) = surface_destroyed_at {
            self.restart_position = position;
            return self.recover_destroyed_surface();
        }
        Ok(SendResult::Sent)
    }

    fn decode_audio(
        &mut self,
        samples: &[EncodedSample],
    ) -> Result<Vec<DecodedAudioFrame>, VideoError> {
        let Some(decoder) = self.audio_decoder.as_mut() else {
            return Ok(Vec::new());
        };
        let track_id = decoder.track_info().id();
        let mut frames = Vec::new();
        for sample in samples
            .iter()
            .filter(|sample| sample.track_id() == track_id)
        {
            frames.extend(decoder.decode(sample)?);
        }
        Ok(frames)
    }

    fn finish_audio(&mut self) -> Result<Vec<DecodedAudioFrame>, VideoError> {
        self.audio_decoder
            .as_mut()
            .map(AndroidProtectedAudioDecoder::finish)
            .transpose()
            .map(Option::unwrap_or_default)
    }

    fn finish(&mut self, channels: SegmentedDecoderChannels<'_>) -> Result<SendResult, VideoError> {
        let lifecycle = self
            .surface_port
            .as_ref()
            .and_then(AndroidVideoSurfacePort::poll_lifecycle);
        if let Some(lifecycle) = lifecycle {
            return self.handle_surface_lifecycle(lifecycle);
        }
        let (Some(decoder), Some(port)) = (&mut self.video_decoder, &self.surface_port) else {
            return Ok(SendResult::Sent);
        };
        decoder.finish_input()?;
        match drain_available_protected_outputs(decoder, port, channels)? {
            ProtectedDrain::Continue => {}
            ProtectedDrain::Command(result) => return Ok(result),
            ProtectedDrain::SurfaceDestroyed(position) => {
                self.restart_position = position;
                return self.recover_destroyed_surface();
            }
            ProtectedDrain::HostDisposed => return Err(video_surface_disposed()),
        }
        loop {
            let output = decoder.wait_dequeue_output()?;
            if output.is_end_of_stream() {
                decoder.discard_output(output)?;
                return Ok(SendResult::Sent);
            }
            match publish_protected_output(decoder, port, output, channels)? {
                ProtectedDrain::Continue => {}
                ProtectedDrain::Command(result) => return Ok(result),
                ProtectedDrain::SurfaceDestroyed(position) => {
                    self.restart_position = position;
                    return self.recover_destroyed_surface();
                }
                ProtectedDrain::HostDisposed => return Err(video_surface_disposed()),
            }
        }
    }

    fn handle_surface_lifecycle(
        &mut self,
        lifecycle: AndroidVideoSurfaceLifecycle,
    ) -> Result<SendResult, VideoError> {
        match lifecycle {
            AndroidVideoSurfaceLifecycle::Destroyed => self.recover_destroyed_surface(),
            AndroidVideoSurfaceLifecycle::HostDisposed => Err(video_surface_disposed()),
        }
    }

    fn recover_destroyed_surface(&mut self) -> Result<SendResult, VideoError> {
        self.reset_video_decoder()?;
        Ok(SendResult::Seek(self.restart_position))
    }

    fn renew_keys_if_needed(
        &mut self,
        channels: SegmentedDecoderChannels<'_>,
    ) -> Result<SendResult, VideoError> {
        let config = self
            .config
            .as_ref()
            .expect("active protected decoder must retain its network configuration");
        let drm = config.drm.clone().unwrap_or_default();
        let result =
            renew_android_decoder_keys(self.video_decoder.as_mut(), config, &drm, channels)?;
        if result != SendResult::Sent {
            return Ok(result);
        }
        renew_android_decoder_keys(self.audio_decoder.as_mut(), config, &drm, channels)
    }
}

#[cfg(target_os = "android")]
trait AndroidRenewableDecoder {
    fn renewal_challenge_if_needed(
        &self,
        threshold: Duration,
    ) -> Result<Option<AndroidLicenseChallenge>, VideoError>;

    fn provide_key_response(
        &mut self,
        response: &LicenseResponse,
    ) -> Result<Option<AndroidOfflineKeySet>, VideoError>;
}

#[cfg(target_os = "android")]
impl AndroidRenewableDecoder for AndroidProtectedVideoDecoder {
    fn renewal_challenge_if_needed(
        &self,
        threshold: Duration,
    ) -> Result<Option<AndroidLicenseChallenge>, VideoError> {
        self.renewal_challenge_if_needed(threshold)
    }

    fn provide_key_response(
        &mut self,
        response: &LicenseResponse,
    ) -> Result<Option<AndroidOfflineKeySet>, VideoError> {
        self.provide_key_response(response)
    }
}

#[cfg(target_os = "android")]
impl AndroidRenewableDecoder for AndroidProtectedAudioDecoder {
    fn renewal_challenge_if_needed(
        &self,
        threshold: Duration,
    ) -> Result<Option<AndroidLicenseChallenge>, VideoError> {
        self.renewal_challenge_if_needed(threshold)
    }

    fn provide_key_response(
        &mut self,
        response: &LicenseResponse,
    ) -> Result<Option<AndroidOfflineKeySet>, VideoError> {
        self.provide_key_response(response)
    }
}

#[cfg(target_os = "android")]
fn renew_android_decoder_keys<T: AndroidRenewableDecoder>(
    decoder: Option<&mut T>,
    config: &AndroidPlaybackConfig,
    drm: &AndroidDrmNetworkConfig,
    channels: SegmentedDecoderChannels<'_>,
) -> Result<SendResult, VideoError> {
    let Some(decoder) = decoder else {
        return Ok(SendResult::Sent);
    };
    let Some(challenge) = decoder.renewal_challenge_if_needed(drm.renewal_threshold)? else {
        return Ok(SendResult::Sent);
    };
    let response = acquire_android_license(config, drm, &challenge)?;
    let Some(key_set) = decoder.provide_key_response(&response)? else {
        return Ok(SendResult::Sent);
    };
    Ok(send_responsive(
        channels.updates,
        channels.control,
        DecoderOutput::OfflineDrmKeySetChanged(OfflineDrmKeySet::new(key_set.as_bytes().to_vec())),
    ))
}

#[cfg(target_os = "android")]
impl Drop for AndroidProtectedDecoders {
    fn drop(&mut self) {
        self.video_decoder = None;
        self.audio_decoder = None;
        if let Some(port) = self.surface_port.take()
            && let Err(error) = port.deactivate()
        {
            tracing::error!(%error, "failed to deactivate Android protected Surface");
        }
    }
}

#[cfg(target_os = "android")]
struct AndroidPowerDecoders {
    config: Option<AndroidPlaybackConfig>,
    audio_track: Option<TrackInfo>,
    video_track: Option<TrackInfo>,
    offload: Option<AndroidOffloadAudioPlayback>,
    tunnel: Option<AndroidTunneledPlayback>,
    surface_port: Option<AndroidVideoSurfacePort>,
    controller_published: bool,
    restart_position: Duration,
}

#[cfg(target_os = "android")]
impl AndroidPowerDecoders {
    const fn new(config: Option<AndroidPlaybackConfig>) -> Self {
        Self {
            config,
            audio_track: None,
            video_track: None,
            offload: None,
            tunnel: None,
            surface_port: None,
            controller_published: false,
            restart_position: Duration::ZERO,
        }
    }

    fn policy(&self) -> PlaybackPowerPolicy {
        self.config
            .as_ref()
            .map_or(PlaybackPowerPolicy::PlatformManaged, |config| config.power)
    }

    fn handles_audio(&self) -> bool {
        self.policy() != PlaybackPowerPolicy::PlatformManaged
    }

    fn handles_video(&self) -> bool {
        self.policy() == PlaybackPowerPolicy::RequireAudioVideoTunneling
    }

    fn update_audio_track(&mut self, track: TrackInfo) -> Result<bool, VideoError> {
        if !self.handles_audio() {
            return Ok(false);
        }
        reject_protected_power_track(&track)?;
        if self.audio_track.as_ref() != Some(&track) {
            self.reset_path()?;
            self.audio_track = Some(track);
        }
        Ok(true)
    }

    fn update_video_track(&mut self, track: TrackInfo) -> Result<bool, VideoError> {
        if !self.handles_video() {
            return Ok(false);
        }
        reject_protected_power_track(&track)?;
        if self.video_track.as_ref() != Some(&track) {
            self.reset_path()?;
            self.video_track = Some(track);
        }
        Ok(true)
    }

    fn ensure_path(
        &mut self,
        channels: SegmentedDecoderChannels<'_>,
    ) -> Result<SendResult, VideoError> {
        if let Some(result) = self.poll_surface_lifecycle()? {
            return Ok(result);
        }
        if self.policy() == PlaybackPowerPolicy::PlatformManaged {
            return Ok(SendResult::Sent);
        }
        let config = self
            .config
            .as_ref()
            .expect("required Android power path must retain its configuration");
        config.validate_power_requirements()?;
        if self.audio_track.is_none() || (self.handles_video() && self.video_track.is_none()) {
            return Ok(SendResult::Sent);
        }
        if self.surface_port.is_none() {
            self.surface_port = Some(
                match wait_for_video_surface_host(&config.surfaces, channels.control) {
                    VideoSurfaceHostWait::Ready(port) => port,
                    VideoSurfaceHostWait::Command(result) => return Ok(result),
                },
            );
        }
        let port = self
            .surface_port
            .as_ref()
            .expect("power path setup must retain its video surface host");
        if self.offload.is_none() && self.tunnel.is_none() {
            let audio_track = self
                .audio_track
                .as_ref()
                .expect("power path setup must retain its audio track")
                .clone();
            match self.policy() {
                PlaybackPowerPolicy::RequireAudioOffload => {
                    let context = port.playback_context()?;
                    self.offload = Some(AndroidOffloadAudioPlayback::new(
                        &context,
                        audio_track,
                        false,
                    )?);
                }
                PlaybackPowerPolicy::RequireAudioVideoTunneling => {
                    let surface = port.acquire_clear()?;
                    let video_track = self
                        .video_track
                        .as_ref()
                        .expect("tunnel setup must retain its video track")
                        .clone();
                    self.tunnel = Some(AndroidTunneledPlayback::new(
                        surface,
                        video_track,
                        audio_track,
                    )?);
                }
                PlaybackPowerPolicy::PlatformManaged => unreachable!(
                    "platform-managed playback returns before required power-path creation"
                ),
            }
        }
        if self.controller_published {
            return Ok(SendResult::Sent);
        }
        let controller = self.offload.as_ref().map_or_else(
            || {
                self.tunnel
                    .as_ref()
                    .expect("required tunnel path must retain its audio controller")
                    .audio_controller()
            },
            AndroidOffloadAudioPlayback::controller,
        );
        let result = send_responsive(
            channels.updates,
            channels.control,
            DecoderOutput::OffloadedAudioOpened {
                controller,
                output_path: match self.policy() {
                    PlaybackPowerPolicy::RequireAudioOffload => PlaybackOutputPath::AudioOffload,
                    PlaybackPowerPolicy::RequireAudioVideoTunneling => {
                        PlaybackOutputPath::AudioVideoTunneling
                    }
                    PlaybackPowerPolicy::PlatformManaged => unreachable!(
                        "platform-managed playback cannot publish a required power controller"
                    ),
                },
            },
        );
        if result == SendResult::Sent {
            self.controller_published = true;
        }
        Ok(result)
    }

    fn process(
        &mut self,
        samples: &[EncodedSample],
        channels: SegmentedDecoderChannels<'_>,
    ) -> Result<SendResult, VideoError> {
        if let Some(result) = self.poll_surface_lifecycle()? {
            return Ok(result);
        }
        if let Some(offload) = self.offload.as_ref() {
            let track_id = offload.track_info().id();
            for sample in samples
                .iter()
                .filter(|sample| sample.track_id() == track_id)
            {
                offload.queue(sample)?;
            }
        }
        if self.tunnel.is_some() {
            let tunnel = self
                .tunnel
                .as_ref()
                .expect("checked tunneled path must remain available");
            let audio_id = tunnel.audio_track_info().id();
            let video_id = tunnel.video_track_info().id();
            for sample in samples.iter().filter(
                |sample| matches!(sample.track_id(), id if id == audio_id || id == video_id),
            ) {
                if let Some(result) = self.poll_surface_lifecycle()? {
                    return Ok(result);
                }
                if sample.track_id() == video_id {
                    self.restart_position = sample.presentation_time().to_duration()?;
                }
                let outputs = match self
                    .tunnel
                    .as_mut()
                    .expect("active tunnel must remain available while queuing")
                    .queue(sample)
                {
                    Ok(outputs) => outputs,
                    Err(error) => {
                        return self.poll_surface_lifecycle()?.ok_or(error);
                    }
                };
                for presentation_time in outputs {
                    self.restart_position = presentation_time;
                    let result = send_responsive(
                        channels.updates,
                        channels.control,
                        DecoderOutput::TunneledVideoOutput { presentation_time },
                    );
                    if result != SendResult::Sent {
                        return Ok(result);
                    }
                }
            }
        }
        Ok(SendResult::Sent)
    }

    fn flush(&mut self, position: Duration) -> Result<(), VideoError> {
        self.restart_position = position;
        if let Some(offload) = self.offload.as_ref() {
            offload.flush(position)?;
        }
        if let Some(tunnel) = self.tunnel.as_mut() {
            tunnel.flush(position)?;
        }
        Ok(())
    }

    fn finish(&mut self, channels: SegmentedDecoderChannels<'_>) -> Result<SendResult, VideoError> {
        if let Some(result) = self.poll_surface_lifecycle()? {
            return Ok(result);
        }
        if self.policy() != PlaybackPowerPolicy::PlatformManaged
            && self.offload.is_none()
            && self.tunnel.is_none()
        {
            return Err(VideoError::Unsupported(String::from(
                "required Android power path reached end-of-stream without a compatible audio/video track set",
            )));
        }
        if let Some(offload) = self.offload.as_ref() {
            offload.controller().finish()?;
        }
        if let Some(tunnel) = self.tunnel.as_mut() {
            let outputs = match tunnel.finish_input() {
                Ok(outputs) => outputs,
                Err(error) => {
                    return self.poll_surface_lifecycle()?.ok_or(error);
                }
            };
            for presentation_time in outputs {
                self.restart_position = presentation_time;
                let result = send_responsive(
                    channels.updates,
                    channels.control,
                    DecoderOutput::TunneledVideoOutput { presentation_time },
                );
                if result != SendResult::Sent {
                    return Ok(result);
                }
            }
        }
        Ok(SendResult::Sent)
    }

    fn reset_path(&mut self) -> Result<(), VideoError> {
        self.offload = None;
        self.tunnel = None;
        self.controller_published = false;
        if let Some(port) = self.surface_port.as_ref() {
            port.deactivate()?;
        }
        Ok(())
    }

    fn poll_surface_lifecycle(&mut self) -> Result<Option<SendResult>, VideoError> {
        let lifecycle = self
            .surface_port
            .as_ref()
            .and_then(AndroidVideoSurfacePort::poll_lifecycle);
        match lifecycle {
            Some(AndroidVideoSurfaceLifecycle::Destroyed) if self.tunnel.is_some() => {
                self.reset_path()?;
                Ok(Some(SendResult::Seek(self.restart_position)))
            }
            Some(AndroidVideoSurfaceLifecycle::Destroyed) | None => Ok(None),
            Some(AndroidVideoSurfaceLifecycle::HostDisposed) => Err(video_surface_disposed()),
        }
    }

    fn video_track_info(&self) -> Option<&TrackInfo> {
        if self.handles_video() {
            self.video_track.as_ref()
        } else {
            None
        }
    }

    fn has_audio_track(&self) -> bool {
        self.handles_audio() && self.audio_track.is_some()
    }
}

#[cfg(target_os = "android")]
impl Drop for AndroidPowerDecoders {
    fn drop(&mut self) {
        self.offload = None;
        self.tunnel = None;
        if let Some(port) = self.surface_port.take()
            && let Err(error) = port.deactivate()
        {
            tracing::error!(%error, "failed to deactivate Android power-path Surface");
        }
    }
}

#[cfg(target_os = "android")]
fn reject_protected_power_track(track: &TrackInfo) -> Result<(), VideoError> {
    if track.protection().is_some() {
        return Err(VideoError::Unsupported(format!(
            "required Android offload/tunneling cannot accept protected {:?} track {} without a CDM-aware compressed-audio path",
            track.kind(),
            track.id().get()
        )));
    }
    Ok(())
}

#[cfg(target_os = "android")]
enum VideoSurfaceHostWait {
    Ready(AndroidVideoSurfacePort),
    Command(SendResult),
}

#[cfg(target_os = "android")]
fn wait_for_video_surface_host(
    surfaces: &AndroidVideoSurfaceReceiver,
    control: &Receiver<DecoderControl>,
) -> VideoSurfaceHostWait {
    let surface_receiver = surfaces.receiver();
    let surface = surface_receiver.recv();
    let command = control.recv();
    futures::pin_mut!(surface, command);
    match futures::executor::block_on(select(surface, command)) {
        Either::Left((Ok(port), _)) => VideoSurfaceHostWait::Ready(port),
        Either::Right((Ok(DecoderControl::Seek { position }), _)) => {
            VideoSurfaceHostWait::Command(SendResult::Seek(position))
        }
        Either::Left((Err(_), _)) | Either::Right((Err(_) | Ok(DecoderControl::Stop), _)) => {
            VideoSurfaceHostWait::Command(SendResult::Stop)
        }
    }
}

#[cfg(target_os = "android")]
fn select_android_protection_init_data(
    track: &TrackInfo,
    init_data: &[ProtectionInitData],
    supports_system_id: impl Fn(&[u8; 16]) -> Result<bool, VideoError>,
) -> Result<ProtectionInitData, VideoError> {
    let key_id = track
        .protection()
        .expect("protected Android track must retain CENC defaults")
        .default_key_id();
    let mut compatible_systems = Vec::new();
    for candidate in init_data
        .iter()
        .filter(|candidate| candidate.key_ids().is_empty() || candidate.key_ids().contains(key_id))
    {
        if supports_system_id(candidate.system_id())? {
            return Ok(candidate.clone());
        }
        compatible_systems.push(format_uuid(candidate.system_id()));
    }
    if compatible_systems.is_empty() {
        return Err(VideoError::Container(format!(
            "protected track {} has no PSSH data for key {}",
            track.id().get(),
            format_uuid(key_id)
        )));
    }
    Err(VideoError::Unsupported(format!(
        "Android supports none of the protected track's DRM systems: {}",
        compatible_systems.join(", ")
    )))
}

#[cfg(target_os = "android")]
fn configure_android_protected_decoder(
    config: &AndroidPlaybackConfig,
    surface: waterkit_video::AndroidProtectedSurface,
    track: TrackInfo,
    init_data: ProtectionInitData,
) -> Result<AndroidProtectedVideoDecoder, VideoError> {
    let drm = config.drm.clone().unwrap_or_default();
    let bootstrap = match &drm.offline_key_set {
        Some(key_set) => AndroidDrmBootstrap::<AndroidVideoDecoderTarget>::restore(
            surface,
            track,
            init_data,
            AndroidOfflineKeySet::new(key_set.clone())?,
        ),
        None => AndroidDrmBootstrap::<AndroidVideoDecoderTarget>::new(surface, track, init_data),
    }?;
    match resolve_android_drm_bootstrap(config, &drm, bootstrap)? {
        ResolvedAndroidDrm::License(pending) => {
            let response = acquire_android_license(config, &drm, pending.challenge())?;
            pending.provide_response(&response)
        }
        ResolvedAndroidDrm::Restored(ready) => ready.configure(),
    }
}

#[cfg(target_os = "android")]
fn configure_android_protected_audio_decoder(
    config: &AndroidPlaybackConfig,
    context: &waterkit_video::AndroidDrmContext,
    track: TrackInfo,
    init_data: ProtectionInitData,
) -> Result<AndroidProtectedAudioDecoder, VideoError> {
    let drm = config.drm.clone().unwrap_or_default();
    let bootstrap = match &drm.offline_key_set {
        Some(key_set) => AndroidAudioDrmBootstrap::restore(
            context,
            track,
            init_data,
            AndroidOfflineKeySet::new(key_set.clone())?,
        ),
        None => AndroidAudioDrmBootstrap::new(context, track, init_data),
    }?;
    match resolve_android_drm_bootstrap(config, &drm, bootstrap)? {
        ResolvedAndroidDrm::License(pending) => {
            let response = acquire_android_license(config, &drm, pending.challenge())?;
            pending.provide_response(&response)
        }
        ResolvedAndroidDrm::Restored(ready) => ready.configure(),
    }
}

#[cfg(target_os = "android")]
enum ResolvedAndroidDrm<T> {
    License(AndroidPendingDecoder<T>),
    Restored(AndroidReadyDecoder<T>),
}

#[cfg(target_os = "android")]
fn resolve_android_drm_bootstrap<T>(
    config: &AndroidPlaybackConfig,
    drm: &AndroidDrmNetworkConfig,
    bootstrap: AndroidDrmBootstrap<T>,
) -> Result<ResolvedAndroidDrm<T>, VideoError> {
    match bootstrap {
        AndroidDrmBootstrap::License(pending) => Ok(ResolvedAndroidDrm::License(pending)),
        AndroidDrmBootstrap::Restored(ready) => Ok(ResolvedAndroidDrm::Restored(ready)),
        AndroidDrmBootstrap::Provisioning(provisioning) => {
            let request = provisioning.request().network_request(
                parse_drm_url(drm.provisioning_url.as_deref(), "provisioning")?,
                drm.maximum_response_bytes,
            )?;
            let request = apply_drm_headers(request, drm)?;
            let response = futures::executor::block_on(config.license_server.acquire(request))?;
            match provisioning.provide_response(&response)? {
                AndroidDrmBootstrap::License(pending) => Ok(ResolvedAndroidDrm::License(pending)),
                AndroidDrmBootstrap::Restored(ready) => Ok(ResolvedAndroidDrm::Restored(ready)),
                AndroidDrmBootstrap::Provisioning(_) => Err(VideoError::Platform(String::from(
                    "Android MediaDrm still requires provisioning after accepting its response",
                ))),
            }
        }
    }
}

#[cfg(target_os = "android")]
fn acquire_android_license(
    config: &AndroidPlaybackConfig,
    drm: &AndroidDrmNetworkConfig,
    challenge: &waterkit_video::AndroidLicenseChallenge,
) -> Result<LicenseResponse, VideoError> {
    let request = challenge.network_request(
        parse_drm_url(drm.license_url.as_deref(), "license")?,
        drm.maximum_response_bytes,
    )?;
    let request = apply_drm_headers(request, drm)?;
    futures::executor::block_on(config.license_server.acquire(request))
}

#[cfg(target_os = "android")]
fn apply_drm_headers(
    mut request: LicenseRequest,
    drm: &AndroidDrmNetworkConfig,
) -> Result<LicenseRequest, VideoError> {
    for (name, value) in &drm.request_headers {
        request = request.with_header(name, value)?;
    }
    Ok(request)
}

#[cfg(target_os = "android")]
fn parse_drm_url(url: Option<&str>, purpose: &str) -> Result<Option<Url>, VideoError> {
    url.map(|url| {
        Url::parse(url)
            .map_err(|error| VideoError::Streaming(format!("invalid DRM {purpose} URL: {error}")))
    })
    .transpose()
}

#[cfg(target_os = "android")]
#[derive(Debug, Clone, Copy, PartialEq)]
enum ProtectedDrain {
    Continue,
    Command(SendResult),
    SurfaceDestroyed(Duration),
    HostDisposed,
}

#[cfg(target_os = "android")]
fn video_surface_disposed() -> VideoError {
    VideoError::Platform(String::from(
        "Android video surface host was disposed during decoding",
    ))
}

#[cfg(target_os = "android")]
fn drain_available_protected_outputs(
    decoder: &mut AndroidProtectedVideoDecoder,
    surface_port: &AndroidVideoSurfacePort,
    channels: SegmentedDecoderChannels<'_>,
) -> Result<ProtectedDrain, VideoError> {
    while let Some(output) = decoder.try_dequeue_output()? {
        if output.is_end_of_stream() {
            decoder.discard_output(output)?;
            return Ok(ProtectedDrain::Continue);
        }
        let result = publish_protected_output(decoder, surface_port, output, channels)?;
        if result != ProtectedDrain::Continue {
            return Ok(result);
        }
    }
    Ok(ProtectedDrain::Continue)
}

#[cfg(target_os = "android")]
fn publish_protected_output(
    decoder: &mut AndroidProtectedVideoDecoder,
    surface_port: &AndroidVideoSurfacePort,
    output: AndroidProtectedVideoOutput,
    channels: SegmentedDecoderChannels<'_>,
) -> Result<ProtectedDrain, VideoError> {
    match send_responsive(
        channels.updates,
        channels.control,
        DecoderOutput::ProtectedFrame {
            sequence: output.sequence(),
            presentation_time: output.presentation_time(),
        },
    ) {
        SendResult::Sent => {}
        result => {
            decoder.discard_output(output)?;
            return Ok(ProtectedDrain::Command(result));
        }
    }
    loop {
        let presentation = channels.protected_control.recv();
        let command = channels.control.recv();
        futures::pin_mut!(presentation, command);
        let interaction = select(presentation, command);
        let lifecycle = surface_port.wait_for_lifecycle();
        futures::pin_mut!(interaction, lifecycle);
        match futures::executor::block_on(select(interaction, lifecycle)) {
            Either::Left((
                Either::Left((Ok(ProtectedOutputControl::Present { sequence, delay }), _)),
                _,
            )) if sequence == output.sequence() => {
                decoder.render_output_after(output, delay)?;
                return Ok(ProtectedDrain::Continue);
            }
            Either::Left((
                Either::Left((Ok(ProtectedOutputControl::Discard { sequence }), _)),
                _,
            )) if sequence == output.sequence() => {
                decoder.discard_output(output)?;
                return Ok(ProtectedDrain::Continue);
            }
            Either::Left((Either::Right((Ok(DecoderControl::Seek { position }), _)), _)) => {
                decoder.discard_output(output)?;
                return Ok(ProtectedDrain::Command(SendResult::Seek(position)));
            }
            Either::Left((
                Either::Left((Err(_), _)) | Either::Right((Err(_) | Ok(DecoderControl::Stop), _)),
                _,
            )) => {
                decoder.discard_output(output)?;
                return Ok(ProtectedDrain::Command(SendResult::Stop));
            }
            Either::Left((
                Either::Left((
                    Ok(
                        ProtectedOutputControl::Present { .. }
                        | ProtectedOutputControl::Discard { .. },
                    ),
                    _,
                )),
                _,
            )) => {}
            Either::Right((AndroidVideoSurfaceLifecycle::Destroyed, _)) => {
                decoder.discard_output(output)?;
                return Ok(ProtectedDrain::SurfaceDestroyed(output.presentation_time()));
            }
            Either::Right((AndroidVideoSurfaceLifecycle::HostDisposed, _)) => {
                decoder.discard_output(output)?;
                return Ok(ProtectedDrain::HostDisposed);
            }
        }
    }
}

#[cfg(target_os = "android")]
fn format_uuid(value: &[u8; 16]) -> String {
    uuid::Uuid::from_bytes(*value).to_string()
}

struct SegmentedDecoders {
    duration: Duration,
    audio_output: AudioOutput,
    video: Option<VideoTrackDecoder>,
    audio: Option<AudioTrackDecoder>,
    streaming_audio: Option<StreamingAudioProducer>,
    audio_buffer_progress: Option<Receiver<()>>,
    opened: bool,
    #[cfg(target_os = "android")]
    protected: AndroidProtectedDecoders,
    #[cfg(target_os = "android")]
    power: AndroidPowerDecoders,
}

fn forward_subtitle_cues(
    batch: &SegmentBatch,
    channels: SegmentedDecoderChannels<'_>,
) -> SendResult {
    if batch.subtitle_cues.is_empty() {
        return SendResult::Sent;
    }
    send_responsive(
        channels.updates,
        channels.control,
        DecoderOutput::SubtitleCues(batch.subtitle_cues.clone()),
    )
}

fn forward_timed_metadata(
    batch: &SegmentBatch,
    channels: SegmentedDecoderChannels<'_>,
) -> SendResult {
    if batch.timed_metadata.is_empty() {
        return SendResult::Sent;
    }
    send_responsive(
        channels.updates,
        channels.control,
        DecoderOutput::TimedMetadata(batch.timed_metadata.clone()),
    )
}

fn publish_decoded_frames(
    frames: impl IntoIterator<Item = DecodedVideoFrame>,
    channels: SegmentedDecoderChannels<'_>,
) -> SendResult {
    for frame in frames {
        let result = publish_latest(
            channels.frames,
            channels.stale_frames,
            channels.control,
            frame,
        );
        if result != SendResult::Sent {
            return result;
        }
    }
    SendResult::Sent
}

impl SegmentedDecoders {
    #[cfg_attr(
        not(target_os = "android"),
        expect(
            clippy::missing_const_for_fn,
            reason = "the same constructor clones Android playback services on Android"
        )
    )]
    fn new(
        duration: Duration,
        audio_output: AudioOutput,
        #[cfg(target_os = "android")] android_playback: Option<AndroidPlaybackConfig>,
    ) -> Self {
        Self {
            duration,
            audio_output,
            video: None,
            audio: None,
            streaming_audio: None,
            audio_buffer_progress: None,
            opened: false,
            #[cfg(target_os = "android")]
            protected: AndroidProtectedDecoders::new(android_playback.clone()),
            #[cfg(target_os = "android")]
            power: AndroidPowerDecoders::new(android_playback),
        }
    }

    fn reset_codecs(&mut self, position: Duration) -> Result<(), VideoError> {
        self.video = self
            .video
            .as_ref()
            .map(|decoder| VideoTrackDecoder::new(decoder.track_info().clone()))
            .transpose()?;
        self.audio = self
            .audio
            .as_ref()
            .map(|decoder| AudioTrackDecoder::new(decoder.track_info().clone()))
            .transpose()?;
        #[cfg(target_os = "android")]
        self.protected.flush(position)?;
        #[cfg(target_os = "android")]
        self.power.flush(position)?;
        if let Some(audio) = self.streaming_audio.as_ref() {
            audio
                .reset_to(position)
                .map_err(|error| VideoError::Codec(error.to_string()))?;
        }
        Ok(())
    }

    fn process(
        &mut self,
        batch: &SegmentBatch,
        channels: SegmentedDecoderChannels<'_>,
    ) -> Result<SendResult, VideoError> {
        if let Some(bits_per_second) = batch.network_throughput {
            let result = send_responsive(
                channels.updates,
                channels.control,
                DecoderOutput::NetworkThroughput(bits_per_second),
            );
            if result != SendResult::Sent {
                return Ok(result);
            }
        }
        let result = forward_timed_metadata(batch, channels);
        if result != SendResult::Sent {
            return Ok(result);
        }
        let result = forward_subtitle_cues(batch, channels);
        if result != SendResult::Sent {
            return Ok(result);
        }
        let drained = self.update_tracks(&batch.tracks)?;
        #[cfg(target_os = "android")]
        if let Some(result) = self
            .protected
            .ensure_decoders(&batch.protection_init_data, channels.control)?
        {
            return Ok(result);
        }
        #[cfg(target_os = "android")]
        match self.protected.renew_keys_if_needed(channels)? {
            SendResult::Sent => {}
            result => return Ok(result),
        }
        #[cfg(target_os = "android")]
        match self.power.ensure_path(channels)? {
            SendResult::Sent => {}
            result => return Ok(result),
        }

        let result = self.ensure_streaming_audio(channels)?;
        if result != SendResult::Sent {
            return Ok(result);
        }
        let result = self.publish_opened(channels);
        if result != SendResult::Sent {
            return Ok(result);
        }
        let result = publish_decoded_frames(drained, channels);
        if result != SendResult::Sent {
            return Ok(result);
        }
        self.decode_audio(&batch.samples)?;
        let result = self.decode_clear_video(&batch.samples, channels)?;
        if result != SendResult::Sent {
            return Ok(result);
        }

        #[cfg(target_os = "android")]
        match self.protected.process(&batch.samples, channels)? {
            SendResult::Sent => {}
            result => return Ok(result),
        }
        #[cfg(target_os = "android")]
        match self.power.process(&batch.samples, channels)? {
            SendResult::Sent => {}
            result => return Ok(result),
        }

        Ok(SendResult::Sent)
    }

    fn ensure_streaming_audio(
        &mut self,
        channels: SegmentedDecoderChannels<'_>,
    ) -> Result<SendResult, VideoError> {
        #[cfg(target_os = "android")]
        if self.power.handles_audio() {
            return Ok(SendResult::Sent);
        }
        if !self.has_audio_track() || self.streaming_audio.is_some() {
            return Ok(SendResult::Sent);
        }
        let audio_player = StreamingAudioPlayer::new_with_output(&self.audio_output)
            .map_err(|error| VideoError::Codec(error.to_string()))?;
        let audio_producer = audio_player.producer();
        let audio_buffer_progress = audio_producer.buffer_progress_receiver();
        let result = send_responsive(
            channels.updates,
            channels.control,
            DecoderOutput::StreamingAudioOpened(audio_player),
        );
        if result == SendResult::Sent {
            self.audio_buffer_progress = Some(audio_buffer_progress);
            self.streaming_audio = Some(audio_producer);
        }
        Ok(result)
    }

    fn publish_opened(&mut self, channels: SegmentedDecoderChannels<'_>) -> SendResult {
        let Some(video) = self.video_track_info().filter(|_| !self.opened) else {
            return SendResult::Sent;
        };
        let dimensions = video
            .video_dimensions()
            .expect("segmented video track must declare coded dimensions");
        let output = DecoderOutput::Opened {
            duration: self.duration,
            has_audio: self.has_audio_track(),
            video_dimensions: (dimensions.width.get(), dimensions.height.get()),
            color_info: video.video_color_info().unwrap_or_default(),
        };
        let result = send_responsive(channels.updates, channels.control, output);
        if result == SendResult::Sent {
            self.opened = true;
        }
        result
    }

    fn decode_audio(&mut self, samples: &[EncodedSample]) -> Result<(), VideoError> {
        let mut frames = Vec::new();
        if let Some(audio) = self.audio.as_mut() {
            let track_id = audio.track_id();
            for sample in samples
                .iter()
                .filter(|sample| sample.track_id() == track_id)
            {
                frames.extend(audio.decode(sample)?);
            }
        }
        #[cfg(target_os = "android")]
        frames.extend(self.protected.decode_audio(samples)?);
        if frames.is_empty() {
            return Ok(());
        }
        let output = self
            .streaming_audio
            .as_ref()
            .expect("decoded segmented audio must own an initialized PCM output");
        for frame in frames {
            output
                .enqueue(frame)
                .map_err(|error| VideoError::Codec(error.to_string()))?;
        }
        Ok(())
    }

    fn decode_clear_video(
        &mut self,
        samples: &[EncodedSample],
        channels: SegmentedDecoderChannels<'_>,
    ) -> Result<SendResult, VideoError> {
        let Some(video) = self.video.as_mut() else {
            return Ok(SendResult::Sent);
        };
        let track_id = video.track_id();
        for sample in samples
            .iter()
            .filter(|sample| sample.track_id() == track_id)
        {
            let progress =
                normalized_time_progress(sample.presentation_time().to_duration()?, self.duration);
            let result = publish_decoded_frames(video.decode(sample, progress)?, channels);
            if result != SendResult::Sent {
                return Ok(result);
            }
        }
        Ok(SendResult::Sent)
    }

    fn video_track_info(&self) -> Option<&TrackInfo> {
        let clear = self.video.as_ref().map(VideoTrackDecoder::track_info);
        #[cfg(target_os = "android")]
        {
            clear
                .or_else(|| self.protected.video_track_info())
                .or_else(|| self.power.video_track_info())
        }
        #[cfg(not(target_os = "android"))]
        clear
    }

    #[cfg_attr(
        not(target_os = "android"),
        expect(
            clippy::missing_const_for_fn,
            reason = "the Android implementation queries non-const protected and offload owners"
        )
    )]
    fn has_audio_track(&self) -> bool {
        self.audio.is_some() || {
            #[cfg(target_os = "android")]
            {
                self.protected.audio_track_info().is_some() || self.power.has_audio_track()
            }
            #[cfg(not(target_os = "android"))]
            false
        }
    }

    fn wait_for_audio_capacity(
        &self,
        maximum_buffer: Duration,
        control: &Receiver<DecoderControl>,
    ) -> SendResult {
        let Some(audio) = self.streaming_audio.as_ref() else {
            return SendResult::Sent;
        };
        let buffer_progress = self
            .audio_buffer_progress
            .as_ref()
            .expect("streaming audio output must expose buffer progress");

        while audio.buffered_duration() > maximum_buffer {
            let progressed = buffer_progress.recv();
            let command = control.recv();
            futures::pin_mut!(progressed, command);
            match futures::executor::block_on(select(progressed, command)) {
                Either::Left((Ok(()), _)) => {}
                Either::Right((Ok(DecoderControl::Seek { position }), _)) => {
                    return SendResult::Seek(position);
                }
                Either::Left((Err(_), _))
                | Either::Right((Err(_) | Ok(DecoderControl::Stop), _)) => {
                    return SendResult::Stop;
                }
            }
        }
        SendResult::Sent
    }

    fn update_tracks(
        &mut self,
        tracks: &[TrackInfo],
    ) -> Result<Vec<DecodedVideoFrame>, VideoError> {
        let mut drained = Vec::new();
        if let Some(track) = tracks.iter().find(|track| track.kind() == TrackKind::Audio)
            && track.protection().is_some()
        {
            #[cfg(target_os = "android")]
            {
                if self.power.handles_audio() {
                    reject_protected_power_track(track)?;
                }
                self.audio = None;
                self.protected.update_audio_track(track.clone());
            }
            #[cfg(not(target_os = "android"))]
            return Err(VideoError::Unsupported(String::from(
                "protected self-drawn audio requires a platform CDM backend",
            )));
        } else if let Some(track) = tracks.iter().find(|track| track.kind() == TrackKind::Audio)
            && self
                .audio
                .as_ref()
                .is_none_or(|decoder| decoder.track_info() != track)
        {
            #[cfg(target_os = "android")]
            {
                self.protected.clear_audio();
                if self.power.update_audio_track(track.clone())? {
                    self.audio = None;
                } else if let Some(decoder) = self.audio.as_mut() {
                    decoder.reconfigure(track.clone())?;
                } else {
                    self.audio = Some(AudioTrackDecoder::new(track.clone())?);
                }
            }
            #[cfg(not(target_os = "android"))]
            if let Some(decoder) = self.audio.as_mut() {
                decoder.reconfigure(track.clone())?;
            } else {
                self.audio = Some(AudioTrackDecoder::new(track.clone())?);
            }
        }
        if let Some(track) = tracks.iter().find(|track| track.kind() == TrackKind::Video)
            && track.protection().is_some()
        {
            #[cfg(target_os = "android")]
            {
                if self.power.handles_video() {
                    reject_protected_power_track(track)?;
                }
                self.video = None;
                self.protected.update_video_track(track.clone())?;
            }
            #[cfg(not(target_os = "android"))]
            return Err(VideoError::Unsupported(String::from(
                "protected self-drawn video requires a platform CDM backend",
            )));
        } else if let Some(track) = tracks.iter().find(|track| track.kind() == TrackKind::Video)
            && self
                .video
                .as_ref()
                .is_none_or(|decoder| decoder.track_info() != track)
        {
            #[cfg(target_os = "android")]
            {
                self.protected.clear_video()?;
                if self.power.update_video_track(track.clone())? {
                    self.video = None;
                } else if let Some(decoder) = self.video.as_mut() {
                    drained.extend(decoder.reconfigure(track.clone())?);
                } else {
                    self.video = Some(VideoTrackDecoder::new(track.clone())?);
                }
            }
            #[cfg(not(target_os = "android"))]
            if let Some(decoder) = self.video.as_mut() {
                drained.extend(decoder.reconfigure(track.clone())?);
            } else {
                self.video = Some(VideoTrackDecoder::new(track.clone())?);
            }
        }
        Ok(drained)
    }

    fn finish_video(
        &mut self,
        channels: SegmentedDecoderChannels<'_>,
    ) -> Result<SendResult, VideoError> {
        if let Some(video) = self.video.as_mut() {
            for frame in video.finish()? {
                let result = publish_latest(
                    channels.frames,
                    channels.stale_frames,
                    channels.control,
                    frame,
                );
                if result != SendResult::Sent {
                    return Ok(result);
                }
            }
        }
        #[cfg(target_os = "android")]
        {
            let result = self.protected.finish(channels)?;
            if result != SendResult::Sent {
                return Ok(result);
            }
            let result = self.power.finish(channels)?;
            if result != SendResult::Sent {
                return Ok(result);
            }
        }
        Ok(SendResult::Sent)
    }

    fn finish_audio(&mut self) -> Result<(), VideoError> {
        #[cfg(target_os = "android")]
        if self.power.handles_audio() {
            return Ok(());
        }
        let Some(output) = self.streaming_audio.as_ref() else {
            return Ok(());
        };
        if let Some(decoder) = self.audio.as_mut() {
            for frame in decoder.finish()? {
                output
                    .enqueue(frame)
                    .map_err(|error| VideoError::Codec(error.to_string()))?;
            }
        }
        #[cfg(target_os = "android")]
        for frame in self.protected.finish_audio()? {
            output
                .enqueue(frame)
                .map_err(|error| VideoError::Codec(error.to_string()))?;
        }
        output
            .finish()
            .map_err(|error| VideoError::Codec(error.to_string()))
    }
}

fn run_segmented_decoder(
    source_url: &str,
    protocol: SegmentedProtocol,
    config: SegmentedDecoderConfig,
    start_progress: f64,
    channels: SegmentedDecoderChannels<'_>,
    buffered_nanos: Arc<AtomicU64>,
) {
    let result = open_segment_session(source_url, protocol, &config);
    let session = match result {
        Ok(session) => session,
        Err(error) => {
            let _ = send_responsive(
                channels.updates,
                channels.control,
                DecoderOutput::Error(error.to_string()),
            );
            return;
        }
    };
    let duration = session.duration().unwrap_or_default();
    if send_responsive(
        channels.updates,
        channels.control,
        DecoderOutput::AudioTracks(session.audio_tracks()),
    ) != SendResult::Sent
    {
        return;
    }
    if send_responsive(
        channels.updates,
        channels.control,
        DecoderOutput::VideoTracks(session.video_tracks()),
    ) != SendResult::Sent
    {
        return;
    }
    if send_responsive(
        channels.updates,
        channels.control,
        DecoderOutput::SubtitleTracks(session.subtitle_tracks()),
    ) != SendResult::Sent
    {
        return;
    }
    let maximum_audio_buffer = config.network.maximum_prefetch_buffer();
    let mut prefetcher = SegmentPrefetcher::spawn(
        session,
        start_progress,
        config.network.maximum_prefetch_buffer(),
        buffered_nanos,
    );
    run_prefetched_decoder(
        duration,
        config.audio_output,
        maximum_audio_buffer,
        #[cfg(target_os = "android")]
        config.android_playback,
        &prefetcher,
        channels,
    );
    prefetcher.stop();
}

fn run_prefetched_decoder(
    duration: Duration,
    audio_output: AudioOutput,
    maximum_audio_buffer: Duration,
    #[cfg(target_os = "android")] android_playback: Option<AndroidPlaybackConfig>,
    prefetcher: &SegmentPrefetcher,
    channels: SegmentedDecoderChannels<'_>,
) {
    let mut decoders = SegmentedDecoders::new(
        duration,
        audio_output,
        #[cfg(target_os = "android")]
        android_playback,
    );
    let mut generation = 0_u64;

    loop {
        let output = match wait_for_prefetch_output(prefetcher, channels.control) {
            DecoderWait::Output(output) => output,
            DecoderWait::Command(DecoderControl::Seek { position }) => {
                prefetcher.seek(position);
                continue;
            }
            DecoderWait::Command(DecoderControl::Stop) | DecoderWait::Closed => return,
        };

        match output {
            PrefetchOutput::Ready {
                generation: batch_generation,
                batch,
            } => {
                let consumed = PrefetchConsumed {
                    generation: batch_generation,
                    duration: batch.duration,
                };
                if batch_generation == generation {
                    match decoders.process(&batch, channels) {
                        Ok(SendResult::Sent) => {
                            match decoders
                                .wait_for_audio_capacity(maximum_audio_buffer, channels.control)
                            {
                                SendResult::Sent => {}
                                SendResult::Seek(position) => prefetcher.seek(position),
                                SendResult::Stop => return,
                            }
                        }
                        Ok(SendResult::Seek(position)) => prefetcher.seek(position),
                        Ok(SendResult::Stop) => return,
                        Err(error) => {
                            send_decoder_error(channels.updates, channels.control, &error);
                            return;
                        }
                    }
                }
                prefetcher.consumed(consumed);
            }
            PrefetchOutput::SeekCompleted {
                generation: next_generation,
                pts,
            } => {
                generation = next_generation;
                if let Err(error) = decoders.reset_codecs(pts) {
                    send_decoder_error(channels.updates, channels.control, &error);
                    return;
                }
                match send_responsive(
                    channels.updates,
                    channels.control,
                    DecoderOutput::SeekCompleted { pts },
                ) {
                    SendResult::Sent => {}
                    SendResult::Seek(next) => prefetcher.seek(next),
                    SendResult::Stop => return,
                }
            }
            PrefetchOutput::LiveWindow {
                window,
                playback_rate_range,
            } => match forward_live_window(window, playback_rate_range, channels) {
                SendResult::Sent => {}
                SendResult::Seek(position) => prefetcher.seek(position),
                SendResult::Stop => return,
            },
            PrefetchOutput::Ended {
                generation: end_generation,
            } if end_generation == generation => {
                if finish_prefetched_decoder(&mut decoders, prefetcher, channels) {
                    continue;
                }
                return;
            }
            PrefetchOutput::Ended { .. } => {}
            PrefetchOutput::Error(error) => {
                send_decoder_error(channels.updates, channels.control, &error);
                return;
            }
        }
    }
}

fn finish_prefetched_decoder(
    decoders: &mut SegmentedDecoders,
    prefetcher: &SegmentPrefetcher,
    channels: SegmentedDecoderChannels<'_>,
) -> bool {
    match decoders.finish_video(channels) {
        Ok(SendResult::Sent) => {
            if let Err(error) = decoders.finish_audio() {
                send_decoder_error(channels.updates, channels.control, &error);
                return false;
            }
            let _ = send_responsive(channels.updates, channels.control, DecoderOutput::Ended);
            false
        }
        Ok(SendResult::Seek(position)) => {
            prefetcher.seek(position);
            true
        }
        Ok(SendResult::Stop) => false,
        Err(error) => {
            send_decoder_error(channels.updates, channels.control, &error);
            false
        }
    }
}

fn forward_live_window(
    window: Option<EngineLiveWindow>,
    playback_rate_range: Option<EngineLivePlaybackRateRange>,
    channels: SegmentedDecoderChannels<'_>,
) -> SendResult {
    send_responsive(
        channels.updates,
        channels.control,
        DecoderOutput::LiveWindow {
            window,
            playback_rate_range,
        },
    )
}

struct SegmentPrefetcher {
    outputs: Receiver<PrefetchOutput>,
    commands: LatestSender<PrefetchCommand>,
    consumed: Sender<PrefetchConsumed>,
    handle: Option<thread::JoinHandle<()>>,
}

impl SegmentPrefetcher {
    fn spawn(
        session: Box<dyn SegmentSession>,
        start_progress: f64,
        maximum_prefetch: Duration,
        buffered_nanos: Arc<AtomicU64>,
    ) -> Self {
        let (output_tx, output_rx) = async_channel::unbounded();
        let (command_tx, command_rx) = latest_channel();
        let (consumed_tx, consumed_rx) = async_channel::unbounded();
        let handle = thread::spawn(move || {
            run_segment_prefetcher(
                session,
                start_progress,
                maximum_prefetch,
                &buffered_nanos,
                &output_tx,
                &command_rx,
                &consumed_rx,
            );
        });
        Self {
            outputs: output_rx,
            commands: command_tx,
            consumed: consumed_tx,
            handle: Some(handle),
        }
    }

    fn seek(&self, position: Duration) {
        let _ = self.commands.send(PrefetchCommand::Seek { position });
    }

    fn consumed(&self, consumed: PrefetchConsumed) {
        let _ = self.consumed.try_send(consumed);
    }

    fn stop(&mut self) {
        let _ = self.commands.send(PrefetchCommand::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for SegmentPrefetcher {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_segment_prefetcher(
    mut session: Box<dyn SegmentSession>,
    start_progress: f64,
    maximum_prefetch: Duration,
    buffered_nanos: &AtomicU64,
    outputs: &Sender<PrefetchOutput>,
    commands: &Receiver<PrefetchCommand>,
    consumed: &Receiver<PrefetchConsumed>,
) {
    let mut state = PrefetchState::new(maximum_prefetch);
    let mut published_live_window = PublishedLiveWindow::default();
    if !publish_live_window_if_changed(session.as_ref(), outputs, &mut published_live_window) {
        return;
    }
    let start_position = session
        .duration()
        .unwrap_or_default()
        .mul_f64(start_progress.clamp(0.0, 1.0));
    if !start_position.is_zero()
        && !state.apply_command(
            PrefetchCommand::Seek {
                position: start_position,
            },
            session.as_mut(),
            outputs,
        )
    {
        return;
    }

    'prefetch: loop {
        state.drain_consumed(consumed);
        while let Ok(command) = commands.try_recv() {
            if !state.apply_command(command, session.as_mut(), outputs) {
                return;
            }
        }
        if state.prefetch_full() {
            match wait_for_prefetch_budget(commands, consumed) {
                PrefetchBudgetEvent::Command(command) => {
                    if !state.apply_command(command, session.as_mut(), outputs) {
                        return;
                    }
                }
                PrefetchBudgetEvent::Consumed(item) => state.consume(item),
                PrefetchBudgetEvent::Closed => return,
            }
            continue;
        }

        let buffered = Duration::from_nanos(buffered_nanos.load(Ordering::Acquire));
        let wait = wait_for_session(session.as_mut(), buffered, commands);
        if !publish_live_window_if_changed(session.as_ref(), outputs, &mut published_live_window) {
            return;
        }
        match wait {
            PrefetchWait::Command(command) => {
                if !state.apply_command(command, session.as_mut(), outputs) {
                    return;
                }
            }
            PrefetchWait::Closed => return,
            PrefetchWait::Poll(Ok(SessionPoll::Ready(batch))) => {
                if let Err(error) = state.validate_segment_duration(batch.duration) {
                    let _ = outputs.send_blocking(PrefetchOutput::Error(error));
                    return;
                }
                let generation = state.generation;
                while !state.has_capacity_for(batch.duration) {
                    match wait_for_prefetch_budget(commands, consumed) {
                        PrefetchBudgetEvent::Command(command) => {
                            if !state.apply_command(command, session.as_mut(), outputs) {
                                return;
                            }
                            if state.generation != generation {
                                continue 'prefetch;
                            }
                        }
                        PrefetchBudgetEvent::Consumed(item) => state.consume(item),
                        PrefetchBudgetEvent::Closed => return,
                    }
                }
                if !state.enqueue(batch, outputs) {
                    return;
                }
            }
            PrefetchWait::Poll(Ok(SessionPoll::Awaiting(retry_after))) => {
                if let Some(command) = wait_for_retry(retry_after, commands)
                    && !state.apply_command(command, session.as_mut(), outputs)
                {
                    return;
                }
            }
            PrefetchWait::Poll(Ok(SessionPoll::Ended)) => {
                let _ = outputs.send_blocking(PrefetchOutput::Ended {
                    generation: state.generation,
                });
                return;
            }
            PrefetchWait::Poll(Err(error)) => {
                let _ = outputs.send_blocking(PrefetchOutput::Error(error));
                return;
            }
        }
    }
}

fn publish_live_window_if_changed(
    session: &dyn SegmentSession,
    outputs: &Sender<PrefetchOutput>,
    published: &mut PublishedLiveWindow,
) -> bool {
    let state = session.live_window().and_then(|window| {
        session
            .live_playback_rate_range()
            .map(|range| (window, range))
    });
    match state {
        Ok((window, playback_rate_range))
            if !published.initialized
                || published.value != window
                || published.playback_rate_range != playback_rate_range =>
        {
            published.initialized = true;
            published.value = window;
            published.playback_rate_range = playback_rate_range;
            outputs
                .send_blocking(PrefetchOutput::LiveWindow {
                    window,
                    playback_rate_range,
                })
                .is_ok()
        }
        Ok(_) => true,
        Err(error) => {
            let _ = outputs.send_blocking(PrefetchOutput::Error(error));
            false
        }
    }
}

struct PrefetchState {
    generation: u64,
    queued_duration: Duration,
    maximum_prefetch: Duration,
}

impl PrefetchState {
    const fn new(maximum_prefetch: Duration) -> Self {
        Self {
            generation: 0,
            queued_duration: Duration::ZERO,
            maximum_prefetch,
        }
    }

    fn apply_command(
        &mut self,
        command: PrefetchCommand,
        session: &mut dyn SegmentSession,
        outputs: &Sender<PrefetchOutput>,
    ) -> bool {
        match command {
            PrefetchCommand::Stop => false,
            PrefetchCommand::Seek { position } => {
                self.generation = self.generation.wrapping_add(1);
                self.queued_duration = Duration::ZERO;
                match session.seek(position) {
                    Ok(pts) => outputs
                        .send_blocking(PrefetchOutput::SeekCompleted {
                            generation: self.generation,
                            pts,
                        })
                        .is_ok(),
                    Err(error) => {
                        let _ = outputs.send_blocking(PrefetchOutput::Error(error));
                        false
                    }
                }
            }
        }
    }

    fn enqueue(&mut self, batch: SegmentBatch, outputs: &Sender<PrefetchOutput>) -> bool {
        if !self.has_capacity_for(batch.duration) {
            let error = VideoError::Streaming(format!(
                "segment duration {:?} cannot fit the remaining prefetch budget {:?}",
                batch.duration,
                self.maximum_prefetch.saturating_sub(self.queued_duration)
            ));
            let _ = outputs.send_blocking(PrefetchOutput::Error(error));
            return false;
        }
        self.queued_duration = self.queued_duration.saturating_add(batch.duration);
        outputs
            .send_blocking(PrefetchOutput::Ready {
                generation: self.generation,
                batch,
            })
            .is_ok()
    }

    fn prefetch_full(&self) -> bool {
        self.queued_duration >= self.maximum_prefetch
    }

    fn validate_segment_duration(&self, duration: Duration) -> Result<(), VideoError> {
        if duration.is_zero() || duration > self.maximum_prefetch {
            return Err(VideoError::Streaming(format!(
                "segment duration {duration:?} is outside the non-zero prefetch bound {:?}",
                self.maximum_prefetch
            )));
        }
        Ok(())
    }

    fn has_capacity_for(&self, duration: Duration) -> bool {
        !duration.is_zero()
            && duration <= self.maximum_prefetch.saturating_sub(self.queued_duration)
    }

    const fn consume(&mut self, item: PrefetchConsumed) {
        if item.generation == self.generation {
            self.queued_duration = self.queued_duration.saturating_sub(item.duration);
        }
    }

    fn drain_consumed(&mut self, consumed: &Receiver<PrefetchConsumed>) {
        while let Ok(item) = consumed.try_recv() {
            self.consume(item);
        }
    }
}

enum PrefetchBudgetEvent {
    Command(PrefetchCommand),
    Consumed(PrefetchConsumed),
    Closed,
}

fn wait_for_prefetch_budget(
    commands: &Receiver<PrefetchCommand>,
    consumed: &Receiver<PrefetchConsumed>,
) -> PrefetchBudgetEvent {
    let command = commands.recv();
    let consumed = consumed.recv();
    futures::pin_mut!(command, consumed);
    match futures::executor::block_on(select(command, consumed)) {
        Either::Left((Ok(command), _)) => PrefetchBudgetEvent::Command(command),
        Either::Right((Ok(item), _)) => PrefetchBudgetEvent::Consumed(item),
        Either::Left((Err(_), _)) | Either::Right((Err(_), _)) => PrefetchBudgetEvent::Closed,
    }
}

fn wait_for_prefetch_output(
    prefetcher: &SegmentPrefetcher,
    control: &Receiver<DecoderControl>,
) -> DecoderWait {
    let output = prefetcher.outputs.recv();
    let command = control.recv();
    futures::pin_mut!(output, command);
    match futures::executor::block_on(select(output, command)) {
        Either::Left((Ok(output), _)) => DecoderWait::Output(output),
        Either::Right((Ok(command), _)) => DecoderWait::Command(command),
        Either::Left((Err(_), _)) | Either::Right((Err(_), _)) => DecoderWait::Closed,
    }
}

fn wait_for_session(
    session: &mut dyn SegmentSession,
    buffered: Duration,
    commands: &Receiver<PrefetchCommand>,
) -> PrefetchWait {
    let next = session.next(buffered);
    let command = commands.recv();
    futures::pin_mut!(command);
    match futures::executor::block_on(select(next, command)) {
        Either::Left((poll, _)) => PrefetchWait::Poll(poll),
        Either::Right((Ok(command), pending)) => {
            drop(pending);
            PrefetchWait::Command(command)
        }
        Either::Right((Err(_), pending)) => {
            drop(pending);
            PrefetchWait::Closed
        }
    }
}

fn send_decoder_error(
    updates: &Sender<DecoderOutput>,
    control: &Receiver<DecoderControl>,
    error: &VideoError,
) {
    let _ = send_responsive(updates, control, DecoderOutput::Error(error.to_string()));
}

fn open_segment_session(
    source_url: &str,
    protocol: SegmentedProtocol,
    config: &SegmentedDecoderConfig,
) -> Result<Box<dyn SegmentSession>, VideoError> {
    let url = Url::parse(source_url)
        .map_err(|error| VideoError::Streaming(format!("invalid manifest URL: {error}")))?;
    let request = MediaRequest::new(url, config.network.maximum_manifest_bytes());
    let options = config.playback_options()?;
    match protocol {
        SegmentedProtocol::Hls => futures::executor::block_on(async {
            HlsPlaybackSession::open(request, options, supports_hls_variant)
                .await
                .map(|session| Box::new(HlsSession(session)) as Box<dyn SegmentSession>)
        }),
        SegmentedProtocol::Dash => futures::executor::block_on(async {
            DashPlaybackSession::open(request, options, supports_dash_representation)
                .await
                .map(|session| Box::new(DashSession(session)) as Box<dyn SegmentSession>)
        }),
    }
}

fn wait_for_retry(
    retry_after: Duration,
    commands: &Receiver<PrefetchCommand>,
) -> Option<PrefetchCommand> {
    let timer = async_io::Timer::after(retry_after);
    let command = commands.recv();
    futures::pin_mut!(timer, command);
    match futures::executor::block_on(select(command, timer)) {
        Either::Left((Ok(command), _)) => Some(command),
        Either::Left((Err(_), _)) | Either::Right(_) => None,
    }
}

fn supports_hls_variant(variant: &StreamVariant) -> bool {
    supports_codec_list(&variant.codecs)
}

fn supports_dash_representation(representation: &DashRepresentation) -> bool {
    representation.mime_type.ends_with("/mp4") && supports_codec_list(&representation.codecs)
}

fn supports_codec_list(codecs: &[String]) -> bool {
    codecs.is_empty() || codecs.iter().all(|codec| supports_codec(codec))
}

fn supports_codec(codec: &str) -> bool {
    let codec = codec.trim().to_ascii_lowercase();
    codec.starts_with("avc1")
        || codec.starts_with("avc3")
        || codec.starts_with("hvc1")
        || codec.starts_with("hev1")
        || codec.starts_with("av01")
        || codec.starts_with("mp4a.40.2")
}

fn normalized_time_progress(position: Duration, duration: Duration) -> f64 {
    if duration.is_zero() {
        0.0
    } else {
        (position.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0)
    }
}

fn send_responsive(
    updates: &Sender<DecoderOutput>,
    control: &Receiver<DecoderControl>,
    output: DecoderOutput,
) -> SendResult {
    let send = updates.send(output);
    let receive = control.recv();
    futures::pin_mut!(send, receive);
    match futures::executor::block_on(select(send, receive)) {
        Either::Left((Ok(()), _)) => SendResult::Sent,
        Either::Left((Err(_), _)) | Either::Right((Err(_) | Ok(DecoderControl::Stop), _)) => {
            SendResult::Stop
        }
        Either::Right((Ok(DecoderControl::Seek { position }), _)) => SendResult::Seek(position),
    }
}

fn send_frame_responsive(
    frames: &Sender<DecodedVideoFrame>,
    control: &Receiver<DecoderControl>,
    frame: DecodedVideoFrame,
) -> SendResult {
    let send = frames.send(frame);
    let receive = control.recv();
    futures::pin_mut!(send, receive);
    match futures::executor::block_on(select(send, receive)) {
        Either::Left((Ok(()), _)) => SendResult::Sent,
        Either::Left((Err(_), _)) | Either::Right((Err(_) | Ok(DecoderControl::Stop), _)) => {
            SendResult::Stop
        }
        Either::Right((Ok(DecoderControl::Seek { position }), _)) => SendResult::Seek(position),
    }
}

fn publish_latest<T>(
    frames: &Sender<T>,
    stale_frames: &Receiver<T>,
    control: &Receiver<DecoderControl>,
    mut item: T,
) -> SendResult {
    match control.try_recv() {
        Ok(DecoderControl::Stop) | Err(TryRecvError::Closed) => return SendResult::Stop,
        Ok(DecoderControl::Seek { position }) => {
            return SendResult::Seek(position);
        }
        Err(TryRecvError::Empty) => {}
    }

    loop {
        match frames.try_send(item) {
            Ok(()) => return SendResult::Sent,
            Err(async_channel::TrySendError::Closed(_)) => return SendResult::Stop,
            Err(async_channel::TrySendError::Full(returned)) => {
                item = returned;
                match stale_frames.try_recv() {
                    Ok(_) | Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Closed) => return SendResult::Stop,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

    use super::{
        DecoderControl, EngineLiveWindow, PrefetchConsumed, PrefetchOutput, SegmentBatch,
        SegmentPrefetcher, SegmentSession, SendResult, SessionFuture, SessionPoll, publish_latest,
    };
    use waterkit_video::VideoError;

    struct FakeSession {
        durations: VecDeque<Duration>,
        fetch_count: Arc<AtomicUsize>,
    }

    #[test]
    fn segmented_video_slot_replaces_stale_frame_without_blocking_audio_decode() {
        let (frames, rendered_frames) = async_channel::bounded(1);
        let (_controls, control) = async_channel::unbounded::<DecoderControl>();

        assert_eq!(
            publish_latest(&frames, &rendered_frames, &control, 1_u8),
            SendResult::Sent
        );
        assert_eq!(
            publish_latest(&frames, &rendered_frames, &control, 2_u8),
            SendResult::Sent
        );
        assert_eq!(
            rendered_frames
                .try_recv()
                .expect("latest frame must remain available"),
            2
        );
    }

    #[test]
    fn segmented_video_publication_yields_immediately_to_seek() {
        let (frames, rendered_frames) = async_channel::bounded(1);
        let (controls, control) = async_channel::unbounded();
        controls
            .try_send(DecoderControl::Seek {
                position: Duration::from_secs(45),
            })
            .expect("seek control must enqueue");

        assert_eq!(
            publish_latest(&frames, &rendered_frames, &control, 1_u8),
            SendResult::Seek(Duration::from_secs(45))
        );
        assert!(rendered_frames.is_empty());
    }

    impl SegmentSession for FakeSession {
        fn duration(&self) -> Option<Duration> {
            Some(self.durations.iter().copied().sum())
        }

        fn live_window(&self) -> Result<Option<EngineLiveWindow>, VideoError> {
            Ok(None)
        }

        fn live_playback_rate_range(
            &self,
        ) -> Result<Option<super::EngineLivePlaybackRateRange>, VideoError> {
            Ok(None)
        }

        fn audio_tracks(&self) -> Vec<waterkit_video::SelectableAudioTrack> {
            Vec::new()
        }

        fn video_tracks(&self) -> Vec<waterkit_video::SelectableVideoTrack> {
            Vec::new()
        }

        fn subtitle_tracks(&self) -> Vec<waterkit_video::SelectableSubtitleTrack> {
            Vec::new()
        }

        fn next(&mut self, _buffered: Duration) -> SessionFuture<'_> {
            Box::pin(async move {
                let Some(duration) = self.durations.pop_front() else {
                    return Ok(SessionPoll::Ended);
                };
                self.fetch_count.fetch_add(1, Ordering::AcqRel);
                Ok(SessionPoll::Ready(SegmentBatch {
                    duration,
                    network_throughput: None,
                    tracks: Vec::new(),
                    samples: Vec::new(),
                    subtitle_cues: Vec::new(),
                    timed_metadata: Vec::new(),
                    #[cfg(target_os = "android")]
                    protection_init_data: Vec::new(),
                }))
            })
        }

        fn seek(&mut self, _position: Duration) -> Result<Duration, VideoError> {
            Ok(Duration::ZERO)
        }
    }

    #[test]
    fn prefetcher_blocks_at_duration_budget_until_decode_consumes_a_batch() {
        let fetch_count = Arc::new(AtomicUsize::new(0));
        let session = FakeSession {
            durations: VecDeque::from([Duration::from_secs(2); 4]),
            fetch_count: Arc::clone(&fetch_count),
        };
        let buffered_nanos = Arc::new(AtomicU64::new(0));
        let mut prefetcher = SegmentPrefetcher::spawn(
            Box::new(session),
            0.0,
            Duration::from_secs(6),
            buffered_nanos,
        );
        assert!(matches!(
            prefetcher
                .outputs
                .recv_blocking()
                .expect("prefetch output must remain connected"),
            PrefetchOutput::LiveWindow {
                window: None,
                playback_rate_range: None,
            }
        ));

        let mut first_generation = None;
        for _ in 0..3 {
            let PrefetchOutput::Ready { generation, batch } = prefetcher
                .outputs
                .recv_blocking()
                .expect("prefetch output must remain connected")
            else {
                panic!("prefetcher must emit a ready batch before reaching its budget");
            };
            if first_generation.is_none() {
                first_generation = Some(generation);
            }
            assert_eq!(batch.duration, Duration::from_secs(2));
        }
        assert_eq!(fetch_count.load(Ordering::Acquire), 3);

        prefetcher.consumed(PrefetchConsumed {
            generation: first_generation.expect("ready batches must expose a generation"),
            duration: Duration::from_secs(2),
        });
        let PrefetchOutput::Ready { batch, .. } = prefetcher
            .outputs
            .recv_blocking()
            .expect("consuming a batch must release prefetch capacity")
        else {
            panic!("released capacity must admit the next ready batch");
        };
        assert_eq!(batch.duration, Duration::from_secs(2));
        assert_eq!(fetch_count.load(Ordering::Acquire), 4);
        prefetcher.stop();
    }
}
