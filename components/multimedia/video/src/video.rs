//! Video components and playback controls.
//!
//! This module provides two distinct video components:
//!
//! - [`Video`]: A raw view that displays video without controls.
//! - [`VideoPlayer`]: A full-featured player view for standard playback UX.
//!
//! ## Examples
//!
//! ```no_run
//! use waterui_video::{AspectRatio, video, video_player};
//!
//! // Raw video view - no controls, just displays video
//! let video = video("https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4")
//!     .aspect_ratio(AspectRatio::Fill);
//!
//! // Interactive video player with platform-appropriate controls
//! let player = video_player("https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4")
//!     .show_controls(true);
//! ```

use core::fmt;
use nami::SignalExt as _;
use std::num::{NonZeroU64, NonZeroUsize};
use std::time::Duration;
use waterui_core::{
    Binding, Computed, Environment, NativeView, binding, configurable, layout::StretchAxis,
};

use crate::{
    session::{PlaybackSession, PlayerController, Playlist},
    source::{MediaItem, OfflineDrmKeySet},
};

/// Aspect ratio mode for video playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum AspectRatio {
    /// Fit the video within the bounds while maintaining aspect ratio (letterbox/pillarbox).
    #[default]
    Fit = 0,
    /// Fill the entire bounds, potentially cropping the video.
    Fill = 1,
    /// Stretch the video to fill the bounds, ignoring aspect ratio.
    Stretch = 2,
}

/// Packing layout for an equirectangular spherical-video source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum SphericalStereoLayout {
    /// One panorama shared by both eyes or displayed as an ordinary 360-degree view.
    #[default]
    Mono = 0,
    /// Left-eye panorama above the right-eye panorama.
    TopBottom = 1,
    /// Left-eye panorama to the left of the right-eye panorama.
    LeftRight = 2,
}

/// Reactive camera orientation shared by spherical presentation and input.
#[derive(Clone)]
pub struct SphericalViewport {
    yaw: Binding<f32>,
    pitch: Binding<f32>,
    vertical_field_of_view: Binding<f32>,
}

impl fmt::Debug for SphericalViewport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SphericalViewport")
            .field("yaw_degrees", &self.yaw.get())
            .field("pitch_degrees", &self.pitch.get())
            .field(
                "vertical_field_of_view_degrees",
                &self.vertical_field_of_view.get(),
            )
            .finish()
    }
}

impl SphericalViewport {
    /// Creates a spherical camera with a validated orientation and vertical field of view.
    ///
    /// # Panics
    ///
    /// Panics unless all values are finite, pitch is within `-90..=90`, and
    /// vertical field of view is within `30..=120` degrees.
    #[must_use]
    pub fn new(yaw_degrees: f32, pitch_degrees: f32, vertical_field_of_view_degrees: f32) -> Self {
        validate_spherical_orientation(yaw_degrees, pitch_degrees);
        validate_spherical_field_of_view(vertical_field_of_view_degrees);
        Self {
            yaw: binding(normalize_yaw(yaw_degrees)),
            pitch: binding(pitch_degrees),
            vertical_field_of_view: binding(vertical_field_of_view_degrees),
        }
    }

    /// Creates a forward-facing 90-degree camera.
    #[must_use]
    pub fn centered() -> Self {
        Self::new(0.0, 0.0, 90.0)
    }

    /// Returns the current normalized yaw in degrees.
    #[must_use]
    pub fn yaw_degrees(&self) -> f32 {
        self.yaw.get()
    }

    /// Returns the current pitch in degrees.
    #[must_use]
    pub fn pitch_degrees(&self) -> f32 {
        self.pitch.get()
    }

    /// Returns the current vertical field of view in degrees.
    #[must_use]
    pub fn vertical_field_of_view_degrees(&self) -> f32 {
        self.vertical_field_of_view.get()
    }

    /// Updates the camera orientation.
    ///
    /// # Panics
    ///
    /// Panics unless both values are finite and pitch is within `-90..=90`.
    pub fn set_orientation(&self, yaw_degrees: f32, pitch_degrees: f32) {
        validate_spherical_orientation(yaw_degrees, pitch_degrees);
        self.yaw.set(normalize_yaw(yaw_degrees));
        self.pitch.set(pitch_degrees);
    }

    /// Applies an interactive orientation delta, clamping pitch at the poles.
    ///
    /// # Panics
    ///
    /// Panics unless both deltas are finite.
    pub fn pan_by(&self, yaw_delta_degrees: f32, pitch_delta_degrees: f32) {
        assert!(
            yaw_delta_degrees.is_finite() && pitch_delta_degrees.is_finite(),
            "spherical camera deltas must be finite"
        );
        self.set_orientation(
            self.yaw_degrees() + yaw_delta_degrees,
            (self.pitch_degrees() + pitch_delta_degrees).clamp(-90.0, 90.0),
        );
    }

    /// Updates the vertical field of view.
    ///
    /// # Panics
    ///
    /// Panics unless `degrees` is finite and within `30..=120`.
    pub fn set_vertical_field_of_view_degrees(&self, degrees: f32) {
        validate_spherical_field_of_view(degrees);
        self.vertical_field_of_view.set(degrees);
    }

    /// Applies a positive magnification scale to the current field of view.
    ///
    /// # Panics
    ///
    /// Panics unless `scale` is finite and positive.
    pub fn magnify(&self, scale: f32) {
        assert!(
            scale.is_finite() && scale > 0.0,
            "spherical magnification scale must be finite and positive"
        );
        self.vertical_field_of_view
            .set((self.vertical_field_of_view_degrees() / scale).clamp(30.0, 120.0));
    }

    /// Returns a reactive yaw signal for renderer and application observation.
    #[must_use]
    pub fn yaw_signal(&self) -> Computed<f32> {
        self.yaw.clone().computed()
    }

    /// Returns a reactive pitch signal for renderer and application observation.
    #[must_use]
    pub fn pitch_signal(&self) -> Computed<f32> {
        self.pitch.clone().computed()
    }

    /// Returns a reactive vertical-field-of-view signal.
    #[must_use]
    pub fn vertical_field_of_view_signal(&self) -> Computed<f32> {
        self.vertical_field_of_view.clone().computed()
    }
}

impl Default for SphericalViewport {
    fn default() -> Self {
        Self::centered()
    }
}

/// Equirectangular spherical-video projection configuration.
#[derive(Debug, Clone)]
pub struct EquirectangularProjection {
    viewport: SphericalViewport,
    stereo_layout: SphericalStereoLayout,
}

impl EquirectangularProjection {
    /// Creates a mono equirectangular projection controlled by `viewport`.
    #[must_use]
    pub const fn new(viewport: SphericalViewport) -> Self {
        Self {
            viewport,
            stereo_layout: SphericalStereoLayout::Mono,
        }
    }

    /// Selects the source's stereoscopic packing layout.
    #[must_use]
    pub const fn stereo_layout(mut self, stereo_layout: SphericalStereoLayout) -> Self {
        self.stereo_layout = stereo_layout;
        self
    }

    /// Returns the shared camera controller.
    #[must_use]
    pub const fn viewport(&self) -> &SphericalViewport {
        &self.viewport
    }

    /// Returns the source's stereoscopic packing layout.
    #[must_use]
    pub const fn layout(&self) -> SphericalStereoLayout {
        self.stereo_layout
    }
}

/// Projection used to present decoded video frames.
#[derive(Debug, Clone, Default)]
pub enum VideoProjection {
    /// Ordinary planar presentation using [`AspectRatio`].
    #[default]
    Rectilinear,
    /// Interactive equirectangular spherical presentation.
    Equirectangular(EquirectangularProjection),
}

impl VideoProjection {
    /// Returns whether decoded frames are projected onto a spherical viewport.
    #[must_use]
    pub const fn is_spherical(&self) -> bool {
        matches!(self, Self::Equirectangular(_))
    }
}

fn validate_spherical_orientation(yaw_degrees: f32, pitch_degrees: f32) {
    assert!(
        yaw_degrees.is_finite() && pitch_degrees.is_finite(),
        "spherical camera orientation must be finite"
    );
    assert!(
        (-90.0..=90.0).contains(&pitch_degrees),
        "spherical camera pitch must be within -90..=90 degrees"
    );
}

fn validate_spherical_field_of_view(degrees: f32) {
    assert!(
        degrees.is_finite() && (30.0..=120.0).contains(&degrees),
        "spherical vertical field of view must be within 30..=120 degrees"
    );
}

fn normalize_yaw(yaw_degrees: f32) -> f32 {
    (yaw_degrees + 180.0).rem_euclid(360.0) - 180.0
}

/// A finite linear playback-volume level in the inclusive `0.0..=1.0` range.
///
/// Muting is represented by a separate `Binding<bool>` so a valid volume can
/// never encode two unrelated states.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Volume(f32);

impl Volume {
    /// Silence without setting the player's mute state.
    pub const SILENT: Self = Self(0.0);
    /// Full playback volume.
    pub const FULL: Self = Self(1.0);

    /// Creates a validated linear playback-volume level.
    ///
    /// # Panics
    ///
    /// Panics when `level` is not finite or is outside `0.0..=1.0`.
    #[must_use]
    pub const fn new(level: f32) -> Self {
        assert!(
            level >= 0.0 && level <= 1.0,
            "video volume must be finite and within 0.0..=1.0"
        );
        Self(level)
    }

    /// Returns the validated linear level.
    #[must_use]
    pub const fn level(self) -> f32 {
        self.0
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self::new(0.5)
    }
}

/// Subtitle track selection policy for a player instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubtitleSelection {
    /// Use the player's default choice.
    #[default]
    Auto,
    /// Disable subtitle rendering.
    Off,
    /// Force a specific subtitle track by index in the player's current runtime track list.
    Track(usize),
}

/// Audio track selection policy for a player instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioTrackSelection {
    /// Use the player's default or autoselected audio track.
    #[default]
    Auto,
    /// Force a specific audio track by index in the player's current runtime track list.
    Track(usize),
}

/// Video-quality track selection policy for a player instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoTrackSelection {
    /// Let adaptive playback select the video quality.
    #[default]
    Auto,
    /// Force a specific video track by index in the player's current runtime track list.
    Track(usize),
}

/// One atomic snapshot of a live presentation's seekable timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveWindow {
    seekable_start: Duration,
    seekable_end: Duration,
    live_edge: Duration,
    target_position: Duration,
}

impl LiveWindow {
    /// Creates a validated live-window snapshot.
    ///
    /// # Panics
    ///
    /// Panics unless `seekable_start <= target_position <= seekable_end <= live_edge`.
    #[must_use]
    pub fn new(
        seekable_start: Duration,
        seekable_end: Duration,
        live_edge: Duration,
        target_position: Duration,
    ) -> Self {
        assert!(
            seekable_start <= target_position,
            "live target position must not precede the seekable window"
        );
        assert!(
            target_position <= seekable_end,
            "live target position must not exceed the seekable window"
        );
        assert!(
            seekable_end <= live_edge,
            "live edge must not precede the seekable window"
        );
        Self {
            seekable_start,
            seekable_end,
            live_edge,
            target_position,
        }
    }

    /// Returns the earliest currently seekable presentation position.
    #[must_use]
    pub const fn seekable_start(self) -> Duration {
        self.seekable_start
    }

    /// Returns the latest currently seekable presentation position.
    #[must_use]
    pub const fn seekable_end(self) -> Duration {
        self.seekable_end
    }

    /// Returns the current live edge.
    #[must_use]
    pub const fn live_edge(self) -> Duration {
        self.live_edge
    }

    /// Returns the backend-recommended playback position near the live edge.
    #[must_use]
    pub const fn target_position(self) -> Duration {
        self.target_position
    }

    /// Returns the target latency behind the live edge.
    #[must_use]
    pub const fn target_live_offset(self) -> Duration {
        self.live_edge.saturating_sub(self.target_position)
    }

    /// Returns whether `position` is currently seekable.
    #[must_use]
    pub fn contains(self, position: Duration) -> bool {
        position >= self.seekable_start && position <= self.seekable_end
    }
}

/// One user-selectable audio track in the active presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioTrackInfo {
    label: String,
    language: Option<String>,
    roles: Vec<String>,
}

impl AudioTrackInfo {
    /// Creates an audio-track descriptor.
    ///
    /// # Panics
    ///
    /// Panics when `label` is empty or whitespace-only.
    #[must_use]
    pub fn new(label: impl Into<String>, language: Option<String>, roles: Vec<String>) -> Self {
        let label = label.into();
        assert!(
            !label.trim().is_empty(),
            "audio-track label must not be empty"
        );
        Self {
            label,
            language,
            roles,
        }
    }

    /// Returns the human-readable track label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the BCP-47 or container language tag when declared.
    #[must_use]
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// Returns semantic roles such as `main`, `alternate`, or `commentary`.
    #[must_use]
    pub fn roles(&self) -> &[String] {
        &self.roles
    }
}

/// One user-selectable video representation in stable quality order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoTrackInfo {
    id: String,
    label: String,
    bandwidth: Option<NonZeroU64>,
    dimensions: Option<(u32, u32)>,
    codecs: Vec<String>,
    hdr: bool,
}

impl VideoTrackInfo {
    /// Creates a video-track descriptor.
    ///
    /// # Panics
    ///
    /// Panics when `id` or `label` is empty or whitespace-only.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        bandwidth: Option<NonZeroU64>,
        dimensions: Option<(u32, u32)>,
        codecs: Vec<String>,
        hdr: bool,
    ) -> Self {
        let id = id.into();
        let label = label.into();
        assert!(
            !id.trim().is_empty(),
            "video-track identity must not be empty"
        );
        assert!(
            !label.trim().is_empty(),
            "video-track label must not be empty"
        );
        Self {
            id,
            label,
            bandwidth,
            dimensions,
            codecs,
            hdr,
        }
    }

    /// Returns the manifest or native representation identity.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the human-readable quality label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns declared encoded bandwidth in bits per second when available.
    #[must_use]
    pub const fn bandwidth(&self) -> Option<NonZeroU64> {
        self.bandwidth
    }

    /// Returns encoded dimensions when known.
    #[must_use]
    pub const fn dimensions(&self) -> Option<(u32, u32)> {
        self.dimensions
    }

    /// Returns RFC 6381 codec identifiers when declared.
    #[must_use]
    pub fn codecs(&self) -> &[String] {
        &self.codecs
    }

    /// Returns whether the representation explicitly signals HDR.
    #[must_use]
    pub const fn is_hdr(&self) -> bool {
        self.hdr
    }
}

/// Origin of one subtitle track in the active presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubtitleTrackOrigin {
    /// Application-supplied sidecar asset.
    Sidecar,
    /// Subtitle track embedded in a progressive container.
    Embedded,
    /// Subtitle rendition declared by a streaming manifest.
    Manifest,
    /// Platform-native media-selection option.
    Native,
}

/// One user-selectable subtitle track in the active presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleTrackInfo {
    label: String,
    language: Option<String>,
    roles: Vec<String>,
    forced: bool,
    origin: SubtitleTrackOrigin,
}

impl SubtitleTrackInfo {
    /// Creates a subtitle-track descriptor.
    ///
    /// # Panics
    ///
    /// Panics when `label` is empty or whitespace-only.
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        language: Option<String>,
        roles: Vec<String>,
        forced: bool,
        origin: SubtitleTrackOrigin,
    ) -> Self {
        let label = label.into();
        assert!(
            !label.trim().is_empty(),
            "subtitle-track label must not be empty"
        );
        Self {
            label,
            language,
            roles,
            forced,
            origin,
        }
    }

    /// Returns the human-readable track label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the BCP-47 or container language tag when declared.
    #[must_use]
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// Returns semantic roles such as `caption` or `forced-subtitle`.
    #[must_use]
    pub fn roles(&self) -> &[String] {
        &self.roles
    }

    /// Returns whether the track carries forced narrative text.
    #[must_use]
    pub const fn is_forced(&self) -> bool {
        self.forced
    }

    /// Returns where the track was discovered.
    #[must_use]
    pub const fn origin(&self) -> SubtitleTrackOrigin {
        self.origin
    }
}

/// Complete selectable-track catalog for the active media item.
///
/// Vector indices are the values accepted by the corresponding `Track(usize)`
/// selection variants for the lifetime of this catalog revision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackCatalog {
    audio: Vec<AudioTrackInfo>,
    video: Vec<VideoTrackInfo>,
    subtitles: Vec<SubtitleTrackInfo>,
}

impl TrackCatalog {
    /// Returns audio tracks in selection-index order.
    #[must_use]
    pub fn audio(&self) -> &[AudioTrackInfo] {
        &self.audio
    }

    /// Returns video representations in selection-index order.
    #[must_use]
    pub fn video(&self) -> &[VideoTrackInfo] {
        &self.video
    }

    /// Returns subtitle tracks in selection-index order.
    #[must_use]
    pub fn subtitles(&self) -> &[SubtitleTrackInfo] {
        &self.subtitles
    }

    /// Replaces the audio portion while preserving video and subtitles.
    #[doc(hidden)]
    #[must_use]
    pub fn replacing_audio(mut self, tracks: Vec<AudioTrackInfo>) -> Self {
        self.audio = tracks;
        self
    }

    /// Replaces the video portion while preserving audio and subtitles.
    #[doc(hidden)]
    #[must_use]
    pub fn replacing_video(mut self, tracks: Vec<VideoTrackInfo>) -> Self {
        self.video = tracks;
        self
    }

    /// Replaces the subtitle portion while preserving audio and video.
    #[doc(hidden)]
    #[must_use]
    pub fn replacing_subtitles(mut self, tracks: Vec<SubtitleTrackInfo>) -> Self {
        self.subtitles = tracks;
        self
    }
}

/// Playback strategy for network and realtime sources.
///
/// `Video` / `VideoPlayer` use one policy object so apps can explicitly choose
/// between VOD buffering behavior and realtime behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackPolicy {
    /// `true` for realtime streams (low-latency/live), `false` for VOD/static files.
    pub realtime: bool,
    /// Minimum buffered duration before starting playback for VOD.
    pub vod_start_buffer_ms: u32,
    /// Minimum buffered duration to resume after a VOD stall.
    pub vod_resume_buffer_ms: u32,
    /// Buffer level below which VOD enters buffering state.
    pub vod_stall_buffer_ms: u32,
    /// For realtime mode, drop video frames that are later than this threshold.
    pub live_max_video_late_ms: u32,
    /// Bounded segmented-network and adaptive-selection policy.
    pub network: NetworkPlaybackPolicy,
    /// Required platform power path for this session.
    pub power: PlaybackPowerPolicy,
}

/// Power-path requirement for media playback.
///
/// Required modes fail during player preparation when the platform, codec,
/// output route, or active interactive features cannot satisfy the invariant.
/// They never begin on one path and silently fall back to another.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PlaybackPowerPolicy {
    /// Let the backend select its canonical path while preserving all features.
    #[default]
    PlatformManaged,
    /// Require compressed audio to remain on a platform hardware-offload path.
    RequireAudioOffload,
    /// Require compressed audio and video to use a hardware A/V tunnel.
    RequireAudioVideoTunneling,
}

/// Active media-output architecture selected by a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaybackOutputPath {
    /// A native player owns its platform-defined media path.
    PlatformManaged,
    /// The application decodes video for GPU rendering and audio to PCM.
    DecodedPcmGpu,
    /// Compressed audio is hardware-offloaded while video remains GPU-rendered.
    AudioOffload,
    /// Compressed audio and video share a platform hardware A/V tunnel.
    AudioVideoTunneling,
}

/// Resource and adaptive-selection bounds for HLS and DASH playback.
///
/// Every allocation- or bandwidth-sensitive value is explicit so a manifest
/// cannot silently select an unbounded response or buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkPlaybackPolicy {
    maximum_manifest_bytes: NonZeroUsize,
    maximum_segment_bytes: NonZeroUsize,
    initial_bandwidth: NonZeroU64,
    bandwidth_fraction_per_mille: u16,
    minimum_buffer_for_upgrade: Duration,
    emergency_buffer: Duration,
    maximum_prefetch_buffer: Duration,
    live_catch_up_tolerance: Duration,
    live_minimum_rate_per_mille: u16,
    live_maximum_rate_per_mille: u16,
}

impl NetworkPlaybackPolicy {
    /// Balanced bounded defaults for ordinary adaptive playback.
    ///
    /// # Panics
    ///
    /// This implementation constructs compile-time non-zero literals and
    /// therefore cannot panic.
    #[must_use]
    pub const fn standard() -> Self {
        Self::new(
            NonZeroUsize::new(4 * 1_024 * 1_024).expect("manifest limit must be non-zero"),
            NonZeroUsize::new(64 * 1_024 * 1_024).expect("segment limit must be non-zero"),
            NonZeroU64::new(5_000_000).expect("initial bandwidth must be non-zero"),
            750,
            Duration::from_secs(10),
            Duration::from_secs(2),
            Duration::from_secs(30),
        )
    }

    /// Creates a validated segmented-network policy.
    ///
    /// # Panics
    ///
    /// Panics unless the bandwidth fraction is in `1..=1000`, the emergency
    /// buffer is below the upgrade buffer, and the prefetch bound is non-zero.
    #[must_use]
    pub const fn new(
        maximum_manifest_bytes: NonZeroUsize,
        maximum_segment_bytes: NonZeroUsize,
        initial_bandwidth: NonZeroU64,
        bandwidth_fraction_per_mille: u16,
        minimum_buffer_for_upgrade: Duration,
        emergency_buffer: Duration,
        maximum_prefetch_buffer: Duration,
    ) -> Self {
        assert!(
            bandwidth_fraction_per_mille > 0 && bandwidth_fraction_per_mille <= 1_000,
            "adaptive bandwidth fraction must be in 1..=1000"
        );
        assert!(
            emergency_buffer.as_secs() < minimum_buffer_for_upgrade.as_secs()
                || (emergency_buffer.as_secs() == minimum_buffer_for_upgrade.as_secs()
                    && emergency_buffer.subsec_nanos() < minimum_buffer_for_upgrade.subsec_nanos()),
            "adaptive emergency buffer must be below the upgrade buffer"
        );
        assert!(
            !maximum_prefetch_buffer.is_zero(),
            "maximum segmented prefetch buffer must be non-zero"
        );
        Self {
            maximum_manifest_bytes,
            maximum_segment_bytes,
            initial_bandwidth,
            bandwidth_fraction_per_mille,
            minimum_buffer_for_upgrade,
            emergency_buffer,
            maximum_prefetch_buffer,
            live_catch_up_tolerance: Duration::from_millis(500),
            live_minimum_rate_per_mille: 970,
            live_maximum_rate_per_mille: 1_030,
        }
    }

    /// Configures bounded live-latency correction with hysteresis.
    ///
    /// Rates are expressed per mille: `970` is `0.97x`, `1000` is normal,
    /// and `1030` is `1.03x`. DASH service-description bounds further restrict
    /// this application policy when present.
    ///
    /// # Panics
    ///
    /// Panics when the tolerance is zero or the rates do not form a positive,
    /// ordered interval containing `1000`.
    #[must_use]
    pub const fn live_catch_up(
        mut self,
        tolerance: Duration,
        minimum_rate_per_mille: u16,
        maximum_rate_per_mille: u16,
    ) -> Self {
        assert!(
            !tolerance.is_zero(),
            "live catch-up tolerance must be non-zero"
        );
        assert!(
            minimum_rate_per_mille >= 250
                && minimum_rate_per_mille <= 1_000
                && maximum_rate_per_mille >= 1_000
                && maximum_rate_per_mille <= 4_000
                && minimum_rate_per_mille <= maximum_rate_per_mille,
            "live catch-up rates must be within 250..=4000, ordered, and contain 1000 per mille"
        );
        self.live_catch_up_tolerance = tolerance;
        self.live_minimum_rate_per_mille = minimum_rate_per_mille;
        self.live_maximum_rate_per_mille = maximum_rate_per_mille;
        self
    }

    /// Returns the maximum accepted manifest response size.
    #[must_use]
    pub const fn maximum_manifest_bytes(self) -> NonZeroUsize {
        self.maximum_manifest_bytes
    }

    /// Returns the maximum accepted initialization or media segment size.
    #[must_use]
    pub const fn maximum_segment_bytes(self) -> NonZeroUsize {
        self.maximum_segment_bytes
    }

    /// Returns the conservative initial bandwidth estimate in bits per second.
    #[must_use]
    pub const fn initial_bandwidth(self) -> NonZeroU64 {
        self.initial_bandwidth
    }

    /// Returns the fraction of measured bandwidth available to adaptive selection.
    #[must_use]
    pub const fn bandwidth_fraction_per_mille(self) -> u16 {
        self.bandwidth_fraction_per_mille
    }

    /// Returns buffered duration required before an adaptive upgrade.
    #[must_use]
    pub const fn minimum_buffer_for_upgrade(self) -> Duration {
        self.minimum_buffer_for_upgrade
    }

    /// Returns the buffer threshold that permits an emergency downgrade.
    #[must_use]
    pub const fn emergency_buffer(self) -> Duration {
        self.emergency_buffer
    }

    /// Returns the maximum encoded duration admitted to the pre-decode queue.
    ///
    /// The transport may additionally have one segment in flight; its memory is
    /// independently bounded by [`Self::maximum_segment_bytes`].
    #[must_use]
    pub const fn maximum_prefetch_buffer(self) -> Duration {
        self.maximum_prefetch_buffer
    }

    /// Returns the live-latency error required to start rate correction.
    #[must_use]
    pub const fn live_catch_up_tolerance(self) -> Duration {
        self.live_catch_up_tolerance
    }

    /// Returns the application-authorized slowest live playback rate.
    #[must_use]
    pub fn live_minimum_playback_rate(self) -> f32 {
        f32::from(self.live_minimum_rate_per_mille) / 1_000.0
    }

    /// Returns the application-authorized fastest live playback rate.
    #[must_use]
    pub fn live_maximum_playback_rate(self) -> f32 {
        f32::from(self.live_maximum_rate_per_mille) / 1_000.0
    }
}

impl Default for NetworkPlaybackPolicy {
    fn default() -> Self {
        Self::standard()
    }
}

impl PlaybackPolicy {
    /// Default VOD policy.
    #[must_use]
    pub const fn vod_default() -> Self {
        Self {
            realtime: false,
            vod_start_buffer_ms: 1200,
            vod_resume_buffer_ms: 800,
            vod_stall_buffer_ms: 200,
            live_max_video_late_ms: 50,
            network: NetworkPlaybackPolicy::standard(),
            power: PlaybackPowerPolicy::PlatformManaged,
        }
    }

    /// Default realtime/live policy.
    #[must_use]
    pub const fn live_default() -> Self {
        Self {
            realtime: true,
            ..Self::vod_default()
        }
    }

    /// Requires one explicit platform playback power path.
    ///
    /// A required path is an invariant: preparation fails when the selected
    /// platform, route, codec, protection scheme, or interactive feature is
    /// incompatible. The player never silently changes to another path.
    #[must_use]
    pub const fn power(mut self, power: PlaybackPowerPolicy) -> Self {
        self.power = power;
        self
    }
}

impl Default for PlaybackPolicy {
    fn default() -> Self {
        Self::vod_default()
    }
}

/// Scheme-defined metadata scheduled on the media presentation timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedMetadata {
    scheme_id_uri: String,
    value: String,
    id: u32,
    presentation_time: Duration,
    duration: Duration,
    message_data: Vec<u8>,
}

impl TimedMetadata {
    /// Creates a timed metadata event decoded from a media container.
    #[must_use]
    pub fn new(
        scheme_id_uri: impl Into<String>,
        value: impl Into<String>,
        id: u32,
        presentation_time: Duration,
        duration: Duration,
        message_data: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            scheme_id_uri: scheme_id_uri.into(),
            value: value.into(),
            id,
            presentation_time,
            duration,
            message_data: message_data.into(),
        }
    }

    /// Returns the metadata scheme identifier.
    #[must_use]
    pub fn scheme_id_uri(&self) -> &str {
        &self.scheme_id_uri
    }

    /// Returns the scheme-specific event value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns the scheme-local event identifier.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Returns the event time on the complete presentation timeline.
    #[must_use]
    pub const fn presentation_time(&self) -> Duration {
        self.presentation_time
    }

    /// Returns the event duration declared by the media container.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    /// Returns the uninterpreted scheme-owned message bytes.
    #[must_use]
    pub fn message_data(&self) -> &[u8] {
        &self.message_data
    }
}

/// One backend-independent playback observability snapshot.
///
/// Counters and durations are cumulative for the active [`MediaItem`]. A
/// source change starts a new metrics lifetime; seeks and adaptive rendition
/// switches do not reset it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackMetrics {
    position: Duration,
    buffered_ahead: Duration,
    startup_time: Duration,
    av_drift_ms: Option<f32>,
    dropped_video_frames: u64,
    rebuffer_count: u64,
    rebuffer_duration: Duration,
    observed_network_throughput: Option<NonZeroU64>,
}

impl PlaybackMetrics {
    /// Creates the required timeline portion of a playback snapshot.
    #[must_use]
    pub const fn new(position: Duration, buffered_ahead: Duration, startup_time: Duration) -> Self {
        Self {
            position,
            buffered_ahead,
            startup_time,
            av_drift_ms: None,
            dropped_video_frames: 0,
            rebuffer_count: 0,
            rebuffer_duration: Duration::ZERO,
            observed_network_throughput: None,
        }
    }

    /// Records an observed audio-minus-video clock drift.
    ///
    /// # Panics
    ///
    /// Panics when `drift_ms` is not finite.
    #[must_use]
    pub fn av_drift_ms(mut self, drift_ms: f32) -> Self {
        assert!(drift_ms.is_finite(), "audio-video drift must be finite");
        self.av_drift_ms = Some(drift_ms);
        self
    }

    /// Records the cumulative number of video frames dropped by the backend.
    #[must_use]
    pub const fn dropped_video_frames(mut self, dropped_video_frames: u64) -> Self {
        self.dropped_video_frames = dropped_video_frames;
        self
    }

    /// Records cumulative rebuffer incidents and wall-clock rebuffer duration.
    #[must_use]
    pub const fn rebuffering(mut self, count: u64, duration: Duration) -> Self {
        self.rebuffer_count = count;
        self.rebuffer_duration = duration;
        self
    }

    /// Records the backend's latest conservative network-throughput estimate.
    #[must_use]
    pub const fn observed_network_throughput(mut self, bits_per_second: NonZeroU64) -> Self {
        self.observed_network_throughput = Some(bits_per_second);
        self
    }

    /// Returns the media position represented by this snapshot.
    #[must_use]
    pub const fn position(self) -> Duration {
        self.position
    }

    /// Returns the estimated playable media duration ahead of `position`.
    #[must_use]
    pub const fn buffered_ahead(self) -> Duration {
        self.buffered_ahead
    }

    /// Returns elapsed wall time from source selection to first presentation.
    #[must_use]
    pub const fn startup_time(self) -> Duration {
        self.startup_time
    }

    /// Returns audio-minus-video clock drift when the backend can observe it.
    #[must_use]
    pub const fn observed_av_drift_ms(self) -> Option<f32> {
        self.av_drift_ms
    }

    /// Returns the cumulative number of dropped video frames.
    #[must_use]
    pub const fn dropped_video_frame_count(self) -> u64 {
        self.dropped_video_frames
    }

    /// Returns the cumulative number of post-startup rebuffer incidents.
    #[must_use]
    pub const fn rebuffer_count(self) -> u64 {
        self.rebuffer_count
    }

    /// Returns cumulative post-startup rebuffer wall time.
    #[must_use]
    pub const fn rebuffer_duration(self) -> Duration {
        self.rebuffer_duration
    }

    /// Returns the latest conservative transport estimate in bits per second.
    #[must_use]
    pub const fn observed_network_throughput_bps(self) -> Option<NonZeroU64> {
        self.observed_network_throughput
    }
}

/// Events emitted by video components.
#[derive(Debug, Clone)]
pub enum Event {
    /// The video is ready to play.
    ReadyToPlay,
    /// The video has finished playing.
    Ended,
    /// Playback transitioned between playing and paused/stopped states.
    PlaybackStateChanged {
        /// `true` while media time is actively advancing.
        playing: bool,
    },
    /// Picture in picture mode changed for the current player instance.
    PictureInPictureChanged {
        /// `true` when playback is currently presented in picture in picture.
        active: bool,
    },
    /// Native or system-managed external playback changed for this player.
    ExternalPlaybackChanged {
        /// `true` while the platform presents video on an external playback route.
        active: bool,
    },
    /// The video is buffering due to slow network or disk.
    Buffering,
    /// The video has resumed playing after buffering.
    BufferingEnded,
    /// Current buffered duration in milliseconds.
    BufferLevel {
        /// Approximate buffered duration ahead of current playback position.
        buffered_ms: u32,
    },
    /// Runtime playback diagnostics emitted periodically during playback.
    PlaybackMetrics {
        /// Backend-independent metrics for the active source lifetime.
        metrics: PlaybackMetrics,
    },
    /// The backend selected or changed its concrete playback output path.
    PlaybackOutputPathChanged {
        /// Active native, decoded, offloaded, or tunneled path.
        path: PlaybackOutputPath,
    },
    /// Scheme-defined metadata reached its presentation timestamp.
    TimedMetadata {
        /// The container event mapped onto the complete presentation timeline.
        metadata: TimedMetadata,
    },
    /// A persistent license was renewed and must replace the stored key set.
    OfflineDrmKeySetChanged {
        /// Updated opaque platform key-set identity.
        key_set: OfflineDrmKeySet,
    },
    /// The system or player UI requested the next item in the active queue.
    NextRequested,
    /// The system or player UI requested the previous item in the active queue.
    PreviousRequested,
    /// An error occurred while loading or playing the video.
    Error {
        /// The error message describing what went wrong.
        message: String,
    },
}

/// Event-handler slot for the video components.
///
/// Carries the user's [`Event`] callback as a [`BoxedEventAction`], the
/// `Handler`-style sibling that pairs an event payload with the same
/// `State<T>` / `Environment` extractor machinery used by [`Button::action`].
type OnEvent = waterui_core::handler::BoxedEventAction<Event>;

/// An event handler that has not yet been bound to a rendering environment.
///
/// This is part of the public configuration type only so native resolution can
/// enforce the environment-binding transition at compile time.
#[doc(hidden)]
pub struct VideoEventHandler(OnEvent);

impl fmt::Debug for VideoEventHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoEventHandler").finish_non_exhaustive()
    }
}

impl VideoEventHandler {
    fn new(handler: OnEvent) -> Self {
        Self(handler)
    }

    /// Binds this handler to a rendering environment and returns the callback
    /// surface used by backend realization crates.
    #[doc(hidden)]
    #[must_use]
    pub fn bind_callback(self, env: Environment) -> std::rc::Rc<dyn Fn(Event)> {
        let handler = core::cell::RefCell::new(self.0);
        std::rc::Rc::new(move |event| {
            (handler.borrow_mut())(event, &env);
        })
    }

    fn bind(self, env: &Environment) -> BoundVideoEventHandler {
        BoundVideoEventHandler {
            handler: core::cell::RefCell::new(self.0),
            env: env.clone(),
        }
    }
}

/// A video event handler bound to the environment of its rendered view.
#[doc(hidden)]
pub struct BoundVideoEventHandler {
    handler: core::cell::RefCell<OnEvent>,
    env: Environment,
}

impl fmt::Debug for BoundVideoEventHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundVideoEventHandler")
            .finish_non_exhaustive()
    }
}

impl BoundVideoEventHandler {
    /// Dispatches an event using the rendering environment captured during
    /// native view resolution.
    pub fn call(&self, event: Event) {
        (self.handler.borrow_mut())(event, &self.env);
    }
}

// =============================================================================
// Video - Raw view without controls
// =============================================================================

/// Shared reactive playback contract consumed by raw and interactive views.
///
/// Backends must source playback state from this object rather than creating a
/// second view-specific player state.
#[doc(hidden)]
pub struct PlaybackConfiguration<H = Option<VideoEventHandler>> {
    /// Command handle for playlist navigation and transport controls.
    pub controller: PlayerController,
    /// The active media item.
    pub source: Computed<MediaItem>,
    /// Subtitle selection policy for the current runtime track list.
    pub subtitle_selection: Binding<SubtitleSelection>,
    /// Audio-track selection policy for the current runtime track list.
    pub audio_track_selection: Binding<AudioTrackSelection>,
    /// Video-quality selection policy for the current runtime track list.
    pub video_track_selection: Binding<VideoTrackSelection>,
    /// Complete selectable-track catalog for the active item.
    pub track_catalog: Binding<TrackCatalog>,
    /// Current live timeline, or `None` for finite media.
    pub live_window: Binding<Option<LiveWindow>>,
    /// Whether the active playlist has a next item.
    pub has_next: Binding<bool>,
    /// Whether the active playlist has a previous item.
    pub has_previous: Binding<bool>,
    /// Validated playback volume.
    pub volume: Binding<Volume>,
    /// Whether playback audio is muted.
    pub muted: Binding<bool>,
    /// Requested playback rate.
    pub playback_rate: Binding<f32>,
    /// Whether rate changes preserve pitch.
    pub preserve_pitch: Binding<bool>,
    /// Requested playing state shared with [`crate::PlayerController`].
    pub desired_playing: Binding<bool>,
    /// Latest absolute seek target in presentation seconds.
    pub seek_target_seconds: Binding<f64>,
    /// Generation advanced for every controller seek, including repeated targets.
    pub seek_generation: Binding<u64>,
    /// Generation advanced for every forward frame-step request.
    pub step_forward_generation: Binding<u64>,
    /// Generation advanced for every backward frame-step request.
    pub step_backward_generation: Binding<u64>,
    /// Current presentation position in seconds.
    pub position_seconds: Binding<f64>,
    /// Current presentation duration in seconds, or zero while unknown.
    pub duration_seconds: Binding<f64>,
    /// Shared high-level playback phase.
    pub phase: Binding<crate::PlaybackPhase>,
    /// Current playlist repeat policy.
    pub repeat: Binding<crate::RepeatMode>,
    /// Whether playlist traversal uses the stable shuffled order.
    pub shuffle: Binding<bool>,
    /// Buffering and realtime playback strategy.
    pub playback_policy: PlaybackPolicy,
    /// The event handler for video events.
    pub on_event: H,
}

impl<H> fmt::Debug for PlaybackConfiguration<H> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlaybackConfiguration")
            .field("controller", &self.controller)
            .field("playback_policy", &self.playback_policy)
            .finish_non_exhaustive()
    }
}

impl<H> PlaybackConfiguration<H> {
    fn from_session(session: PlaybackSession, on_event: H) -> Self {
        let (bindings, controller) = session.into_parts();
        Self {
            controller,
            source: bindings.source.computed(),
            subtitle_selection: bindings.subtitle_selection,
            audio_track_selection: bindings.audio_track_selection,
            video_track_selection: bindings.video_track_selection,
            track_catalog: bindings.track_catalog,
            live_window: bindings.live_window,
            has_next: bindings.has_next,
            has_previous: bindings.has_previous,
            volume: bindings.volume,
            muted: bindings.muted,
            playback_rate: bindings.playback_rate,
            preserve_pitch: bindings.preserve_pitch,
            desired_playing: bindings.desired_playing,
            seek_target_seconds: bindings.seek_target_seconds,
            seek_generation: bindings.seek_generation,
            step_forward_generation: bindings.step_forward_generation,
            step_backward_generation: bindings.step_backward_generation,
            position_seconds: bindings.position_seconds,
            duration_seconds: bindings.duration_seconds,
            phase: bindings.phase,
            repeat: bindings.repeat,
            shuffle: bindings.shuffle,
            playback_policy: bindings.playback_policy,
            on_event,
        }
    }

    fn map_event_handler<T>(self, map: impl FnOnce(H) -> T) -> PlaybackConfiguration<T> {
        let Self {
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
            shuffle,
            playback_policy,
            on_event,
        } = self;
        PlaybackConfiguration {
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
            shuffle,
            playback_policy,
            on_event: map(on_event),
        }
    }
}

/// Configuration for the [`Video`] component (raw video view).
///
/// This is a raw video view that displays video content without any native controls.
/// Use this when you want to build your own custom video UI.
pub struct VideoConfig<H = Option<VideoEventHandler>> {
    /// Shared session state and controls.
    pub playback: PlaybackConfiguration<H>,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: AspectRatio,
    /// Projection used to present decoded frames.
    pub projection: VideoProjection,
    /// Whether the video should loop when it ends.
    pub loops: bool,
}

impl<H> fmt::Debug for VideoConfig<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoConfig")
            .field("aspect_ratio", &self.aspect_ratio)
            .field("projection", &self.projection)
            .field("loops", &self.loops)
            .field("playback_policy", &self.playback.playback_policy)
            .finish_non_exhaustive()
    }
}

/// Native video payload whose optional event handler carries the live rendering
/// environment.
#[doc(hidden)]
pub type NativeVideoConfig = VideoConfig<Option<BoundVideoEventHandler>>;

impl<H> VideoConfig<H> {
    fn map_event_handler<T>(self, map: impl FnOnce(H) -> T) -> VideoConfig<T> {
        let Self {
            playback,
            aspect_ratio,
            projection,
            loops,
        } = self;
        VideoConfig {
            playback: playback.map_event_handler(map),
            aspect_ratio,
            projection,
            loops,
        }
    }
}

impl VideoConfig {
    fn bind_event_environment(self, env: &Environment) -> NativeVideoConfig {
        self.map_event_handler(|handler| handler.map(|handler| handler.bind(env)))
    }
}

impl NativeView for NativeVideoConfig {
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

configurable!(
    /// A raw video view that displays video without controls.
    ///
    /// Use this component when you want to display video content and build
    /// your own custom UI controls. For a full-featured player with interactive
    /// controls, use [`VideoPlayer`] instead.
    ///
    /// # Platform Implementation
    ///
    /// - **iOS/macOS**: native-backed raw surface by default
    /// - **Android**: runtime-managed raw surface
    Video,
    VideoConfig,
    |config| if config.projection.is_spherical() {
        StretchAxis::Both
    } else {
        match config.aspect_ratio {
            AspectRatio::Fit => StretchAxis::Horizontal,
            AspectRatio::Fill | AspectRatio::Stretch => StretchAxis::Both,
        }
    },
    resolve |config, env| config.bind_event_environment(env)
);

impl Video {
    /// Creates a new raw video view.
    #[must_use]
    pub fn new(session: PlaybackSession) -> Self {
        Self(VideoConfig {
            playback: PlaybackConfiguration::from_session(session, None),
            aspect_ratio: AspectRatio::default(),
            projection: VideoProjection::default(),
            loops: true,
        })
    }

    /// Sets the aspect ratio mode for the video.
    #[must_use]
    pub const fn aspect_ratio(mut self, aspect_ratio: AspectRatio) -> Self {
        self.0.aspect_ratio = aspect_ratio;
        self
    }

    /// Selects planar or equirectangular spherical presentation.
    #[must_use]
    pub fn projection(mut self, projection: VideoProjection) -> Self {
        self.0.projection = projection;
        self
    }

    /// Sets whether the video should loop when it ends.
    #[must_use]
    pub const fn loops(mut self, loops: bool) -> Self {
        self.0.loops = loops;
        self
    }

    /// Sets the event handler for video events.
    ///
    /// The handler may extract dependencies from the environment using the
    /// same `Handler`-style extractor pattern as `Button::action`; the
    /// leading argument is always the [`Event`] payload, and any additional
    /// arguments after it implement [`Extractor`](waterui_core::extract::Extractor).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use waterui_core::binding;
    /// use waterui_video::video;
    /// use waterui_video::video::Event;
    ///
    /// let last_event = binding(String::new());
    /// let last_event_for_handler = last_event.clone();
    /// video("https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4")
    ///     .on_event(move |event: Event| {
    ///         last_event_for_handler.set(format!("{event:?}"));
    ///     });
    /// ```
    #[must_use]
    pub fn on_event<H, A>(mut self, handler: H) -> Self
    where
        H: waterui_core::handler::EventHandler<Event, A, ()> + 'static,
    {
        self.0.playback.on_event = Some(VideoEventHandler::new(
            waterui_core::handler::boxed_event_handler(handler),
        ));
        self
    }
}

/// Creates an autoplaying raw video session for one media item.
#[must_use]
pub fn video(item: impl Into<MediaItem>) -> Video {
    Video::new(PlaybackSession::new(Playlist::single(item)).autoplay())
}

// =============================================================================
// VideoPlayer - Full-featured player with native controls
// =============================================================================

/// Configuration for the [`VideoPlayer`] component.
///
/// This configuration defines an interactive player with platform-appropriate controls.
pub struct VideoPlayerConfig<H = Option<VideoEventHandler>> {
    /// Shared session state and controls.
    pub playback: PlaybackConfiguration<H>,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: AspectRatio,
    /// Projection used to present decoded frames.
    pub projection: VideoProjection,
    /// Whether to show interactive playback controls.
    pub show_controls: bool,
}

impl<H> fmt::Debug for VideoPlayerConfig<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoPlayerConfig")
            .field("aspect_ratio", &self.aspect_ratio)
            .field("projection", &self.projection)
            .field("show_controls", &self.show_controls)
            .field("playback_policy", &self.playback.playback_policy)
            .finish_non_exhaustive()
    }
}

/// Native video-player payload whose optional event handler carries the live
/// rendering environment.
#[doc(hidden)]
pub type NativeVideoPlayerConfig = VideoPlayerConfig<Option<BoundVideoEventHandler>>;

impl<H> VideoPlayerConfig<H> {
    fn map_event_handler<T>(self, map: impl FnOnce(H) -> T) -> VideoPlayerConfig<T> {
        let Self {
            playback,
            aspect_ratio,
            projection,
            show_controls,
        } = self;
        VideoPlayerConfig {
            playback: playback.map_event_handler(map),
            aspect_ratio,
            projection,
            show_controls,
        }
    }
}

impl VideoPlayerConfig {
    fn bind_event_environment(self, env: &Environment) -> NativeVideoPlayerConfig {
        self.map_event_handler(|handler| handler.map(|handler| handler.bind(env)))
    }
}

impl NativeView for NativeVideoPlayerConfig {
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

configurable!(
    /// A full-featured video player component.
    ///
    /// Use this component when you want a complete video playback experience
    /// with a platform-appropriate control experience (play/pause, seek,
    /// fullscreen, etc.).
    /// For a raw video view without controls, use [`Video`] instead.
    ///
    /// # Platform Implementation
    ///
    /// - **Apple platforms**: native player controls by default
    /// - **Android**: WaterUI/Rust player controls
    VideoPlayer,
    VideoPlayerConfig,
    |config| if config.projection.is_spherical() {
        StretchAxis::Both
    } else {
        match config.aspect_ratio {
            AspectRatio::Fit => StretchAxis::Horizontal,
            AspectRatio::Fill | AspectRatio::Stretch => StretchAxis::Both,
        }
    },
    resolve |config, env| config.bind_event_environment(env)
);

impl VideoPlayer {
    /// Creates a new interactive video player.
    #[must_use]
    pub fn new(session: PlaybackSession) -> Self {
        Self(VideoPlayerConfig {
            playback: PlaybackConfiguration::from_session(session, None),
            aspect_ratio: AspectRatio::default(),
            projection: VideoProjection::default(),
            show_controls: true,
        })
    }

    /// Sets the aspect ratio mode for the video player.
    #[must_use]
    pub const fn aspect_ratio(mut self, aspect_ratio: AspectRatio) -> Self {
        self.0.aspect_ratio = aspect_ratio;
        self
    }

    /// Selects planar or interactive equirectangular spherical presentation.
    #[must_use]
    pub fn projection(mut self, projection: VideoProjection) -> Self {
        self.0.projection = projection;
        self
    }

    /// Sets whether to show interactive playback controls.
    #[must_use]
    pub const fn show_controls(mut self, show_controls: bool) -> Self {
        self.0.show_controls = show_controls;
        self
    }

    /// Sets the event handler for the video player.
    ///
    /// See [`Video::on_event`] for the full extractor-based handler shape.
    #[must_use]
    pub fn on_event<H, A>(mut self, handler: H) -> Self
    where
        H: waterui_core::handler::EventHandler<Event, A, ()> + 'static,
    {
        self.0.playback.on_event = Some(VideoEventHandler::new(
            waterui_core::handler::boxed_event_handler(handler),
        ));
        self
    }
}

/// Creates a paused interactive player session for one media item.
#[must_use]
pub fn video_player(item: impl Into<MediaItem>) -> VideoPlayer {
    VideoPlayer::new(PlaybackSession::new(Playlist::single(item)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_preserves_valid_level() {
        assert!((Volume::new(0.8).level() - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    #[should_panic(expected = "video volume must be finite and within 0.0..=1.0")]
    fn volume_rejects_negative_level() {
        let _ = Volume::new(-0.1);
    }

    #[test]
    #[should_panic(expected = "video volume must be finite and within 0.0..=1.0")]
    fn volume_rejects_nan() {
        let _ = Volume::new(f32::NAN);
    }

    #[test]
    fn timed_metadata_preserves_container_event_fields() {
        let metadata = TimedMetadata::new(
            "https://aomedia.org/emsg/ID3",
            "id3",
            11,
            Duration::from_secs(3),
            Duration::from_millis(750),
            b"metadata".to_vec(),
        );

        assert_eq!(metadata.scheme_id_uri(), "https://aomedia.org/emsg/ID3");
        assert_eq!(metadata.value(), "id3");
        assert_eq!(metadata.id(), 11);
        assert_eq!(metadata.presentation_time(), Duration::from_secs(3));
        assert_eq!(metadata.duration(), Duration::from_millis(750));
        assert_eq!(metadata.message_data(), b"metadata");
    }

    #[test]
    fn playback_metrics_preserve_optional_and_cumulative_observations() {
        let throughput = NonZeroU64::new(18_000_000).expect("throughput must be non-zero");
        let metrics = PlaybackMetrics::new(
            Duration::from_secs(12),
            Duration::from_secs(4),
            Duration::from_millis(420),
        )
        .av_drift_ms(-3.5)
        .dropped_video_frames(2)
        .rebuffering(1, Duration::from_millis(125))
        .observed_network_throughput(throughput);

        assert_eq!(metrics.position(), Duration::from_secs(12));
        assert_eq!(metrics.buffered_ahead(), Duration::from_secs(4));
        assert_eq!(metrics.startup_time(), Duration::from_millis(420));
        assert_eq!(metrics.observed_av_drift_ms(), Some(-3.5));
        assert_eq!(metrics.dropped_video_frame_count(), 2);
        assert_eq!(metrics.rebuffer_count(), 1);
        assert_eq!(metrics.rebuffer_duration(), Duration::from_millis(125));
        assert_eq!(metrics.observed_network_throughput_bps(), Some(throughput));
    }

    #[test]
    fn playback_power_policy_is_platform_managed_unless_explicitly_required() {
        assert_eq!(
            PlaybackPolicy::vod_default().power,
            PlaybackPowerPolicy::PlatformManaged
        );
        assert_eq!(
            PlaybackPolicy::vod_default()
                .power(PlaybackPowerPolicy::RequireAudioVideoTunneling)
                .power,
            PlaybackPowerPolicy::RequireAudioVideoTunneling
        );
    }

    #[test]
    fn spherical_viewport_normalizes_pan_and_magnification() {
        let viewport = SphericalViewport::new(540.0, 80.0, 90.0);
        assert!((viewport.yaw_degrees() + 180.0).abs() < f32::EPSILON);

        viewport.pan_by(45.0, 20.0);
        assert!((viewport.yaw_degrees() + 135.0).abs() < f32::EPSILON);
        assert!((viewport.pitch_degrees() - 90.0).abs() < f32::EPSILON);

        viewport.magnify(2.0);
        assert!((viewport.vertical_field_of_view_degrees() - 45.0).abs() < f32::EPSILON);
        viewport.magnify(10.0);
        assert!((viewport.vertical_field_of_view_degrees() - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    #[should_panic(expected = "spherical vertical field of view must be within 30..=120 degrees")]
    fn spherical_viewport_rejects_invalid_field_of_view() {
        let _ = SphericalViewport::new(0.0, 0.0, 121.0);
    }
}
