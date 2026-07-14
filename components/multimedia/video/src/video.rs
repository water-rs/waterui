//! Video components and playback controls.
//!
//! This module provides two distinct video components:
//!
//! - [`Video`]: A raw view that displays video without controls.
//! - [`VideoPlayer`]: A full-featured player view for standard playback UX.
//!
//! ## Volume Control System
//!
//! Both video components use a special volume encoding:
//! - Positive values (> 0): Audible volume level
//! - Negative values (< 0): Muted state that preserves the original volume level
//! - When unmuting, the absolute value is restored
//!
//! ## Examples
//!
//! ```ignore
//! use waterui_video::{Video, VideoPlayer};
//! use waterui_core::binding;
//!
//! // Raw video view - no controls, just displays video
//! let video = Video::new("https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4")
//!     .aspect_ratio(AspectRatio::Fill);
//!
//! // Full-featured video player with native controls
//! let player = VideoPlayer::new("https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4")
//!     .show_controls(true);
//!
//! // Control volume/mute state
//! let muted = binding(false);
//! let video = Video::new("https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4").muted(&muted);
//! muted.set(true);  // Mute - preserves volume level
//! muted.set(false); // Unmute - restores original volume
//! ```

use core::fmt;
use nami::{CustomBinding, Signal, SignalExt, SignalIdentity, watcher::Context};
use waterui_core::{
    Binding, Computed, Environment, NativeView, binding, configurable, layout::StretchAxis,
    reactive::signal::IntoComputed,
};

use crate::source::MediaItem;

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

/// A Volume value represents the audio volume level of a player.
///
/// In a non-muted state, the volume is represented as a positive value (> 0).
/// When muted, the volume is stored as a negative value (< 0),
/// which preserves the original volume level. This allows the player
/// to return to the previous volume setting when unmuted.
///
/// # Examples
///
/// - Volume 0.7 (70%) is stored as `0.7`
/// - When muted, 0.7 becomes `-0.7`
/// - When unmuted, `-0.7` becomes `0.7` again
pub type Volume = f32;

#[derive(Clone)]
struct MutedVolumeBinding {
    value: Computed<Volume>,
    muted: Binding<bool>,
    volume: Binding<Volume>,
}

impl Signal for MutedVolumeBinding {
    type Output = Volume;
    type Guard = <Computed<Volume> as Signal>::Guard;

    fn get(&self) -> Self::Output {
        self.value.get()
    }

    fn identity(&self) -> Option<SignalIdentity> {
        self.value.identity()
    }

    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
        self.value.watch(watcher)
    }
}

impl CustomBinding for MutedVolumeBinding {
    fn set(&self, value: Self::Output) {
        let muted = value <= 0.0;
        if muted {
            self.muted.set(true);
            self.volume.set(value.abs());
        } else {
            self.volume.set(value);
            self.muted.set(false);
        }
    }
}

fn muted_volume_binding(volume: Binding<Volume>, muted: Binding<bool>) -> Binding<Volume> {
    let value = muted
        .zip(&volume)
        .map(|(muted, volume)| if muted { -volume.abs() } else { volume.abs() })
        .computed();
    Binding::custom(MutedVolumeBinding {
        value,
        muted,
        volume,
    })
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
}

impl Default for PlaybackPolicy {
    fn default() -> Self {
        Self::vod_default()
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
        /// Audio/video drift in milliseconds (`audio - video`).
        av_drift_ms: f32,
        /// Cumulative number of dropped video frames.
        dropped_video_frames: u64,
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

    pub(crate) fn into_inner(self) -> OnEvent {
        self.0
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

/// Configuration for the [`Video`] component (raw video view).
///
/// This is a raw video view that displays video content without any native controls.
/// Use this when you want to build your own custom video UI.
pub struct VideoConfig<H = Option<VideoEventHandler>> {
    /// The media item to play.
    pub source: Computed<MediaItem>,
    /// Subtitle selection policy for the player's current runtime track list.
    pub subtitle_selection: Binding<SubtitleSelection>,
    /// Whether the current queue has a next item.
    pub has_next: Binding<bool>,
    /// Whether the current queue has a previous item.
    pub has_previous: Binding<bool>,
    /// The volume of the video.
    pub volume: Binding<Volume>,
    /// Optional mute state composed with [`Self::volume`] during native resolution.
    pub muted: Option<Binding<bool>>,
    /// Playback speed (1.0 = normal speed).
    pub playback_rate: Binding<f32>,
    /// Whether to preserve pitch when changing playback speed.
    pub preserve_pitch: Binding<bool>,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: AspectRatio,
    /// Whether the video should loop when it ends.
    pub loops: bool,
    /// Playback buffering/realtime policy.
    pub playback_policy: PlaybackPolicy,
    /// The event handler for video events.
    pub on_event: H,
}

impl<H> fmt::Debug for VideoConfig<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoConfig")
            .field("aspect_ratio", &self.aspect_ratio)
            .field("loops", &self.loops)
            .field("playback_policy", &self.playback_policy)
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
            source,
            subtitle_selection,
            has_next,
            has_previous,
            volume,
            muted,
            playback_rate,
            preserve_pitch,
            aspect_ratio,
            loops,
            playback_policy,
            on_event,
        } = self;
        VideoConfig {
            source,
            subtitle_selection,
            has_next,
            has_previous,
            volume,
            muted,
            playback_rate,
            preserve_pitch,
            aspect_ratio,
            loops,
            playback_policy,
            on_event: map(on_event),
        }
    }
}

impl VideoConfig {
    fn bind_event_environment(self, env: &Environment) -> NativeVideoConfig {
        self.resolve_muted_volume()
            .map_event_handler(|handler| handler.map(|handler| handler.bind(env)))
    }
}

impl<H> VideoConfig<H> {
    pub(crate) fn resolve_muted_volume(mut self) -> Self {
        if let Some(muted) = self.muted.take() {
            self.volume = muted_volume_binding(self.volume, muted);
        }
        self
    }
}

impl NativeView for NativeVideoConfig {
    fn stretch_axis(&self) -> StretchAxis {
        match self.aspect_ratio {
            AspectRatio::Fit => StretchAxis::Horizontal,
            AspectRatio::Fill | AspectRatio::Stretch => StretchAxis::Both,
        }
    }
}

configurable!(
    /// A raw video view that displays video without native controls.
    ///
    /// Use this component when you want to display video content and build
    /// your own custom UI controls. For a full-featured player with native
    /// controls, use [`VideoPlayer`] instead.
    ///
    /// # Platform Implementation
    ///
    /// - **iOS/macOS**: native-backed raw surface by default
    /// - **Android**: runtime-managed raw surface
    Video,
    VideoConfig,
    |config| match config.aspect_ratio {
        AspectRatio::Fit => StretchAxis::Horizontal,
        AspectRatio::Fill | AspectRatio::Stretch => StretchAxis::Both,
    },
    resolve |config, env| config.bind_event_environment(env)
);

impl Video {
    /// Creates a new raw video view.
    pub fn new(source: impl IntoComputed<MediaItem>) -> Self {
        Self(VideoConfig {
            source: source.into_computed(),
            subtitle_selection: binding(SubtitleSelection::Auto),
            has_next: Binding::bool(false),
            has_previous: Binding::bool(false),
            volume: binding(0.5_f32),
            muted: None,
            playback_rate: binding(1.0_f32),
            preserve_pitch: binding(true),
            aspect_ratio: AspectRatio::default(),
            loops: true,
            playback_policy: PlaybackPolicy::default(),
            on_event: None,
        })
    }

    /// Sets the aspect ratio mode for the video.
    #[must_use]
    pub const fn aspect_ratio(mut self, aspect_ratio: AspectRatio) -> Self {
        self.0.aspect_ratio = aspect_ratio;
        self
    }

    /// Sets whether the video should loop when it ends.
    #[must_use]
    pub const fn loops(mut self, loops: bool) -> Self {
        self.0.loops = loops;
        self
    }

    /// Sets playback buffering/realtime policy.
    #[must_use]
    pub const fn playback_policy(mut self, playback_policy: PlaybackPolicy) -> Self {
        self.0.playback_policy = playback_policy;
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
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui_video::Video;
    /// use waterui_video::video::Event;
    ///
    /// let last_event = binding(String::new());
    /// Video::new(media)
    ///     .on_event(|event: Event, State(last): State<Binding<String>>| {
    ///         last.set(format!("{event:?}"));
    ///     })
    ///     .state(&last_event);
    /// ```
    #[must_use]
    pub fn on_event<H, A>(mut self, handler: H) -> Self
    where
        H: waterui_core::handler::EventHandler<Event, A, ()> + 'static,
    {
        self.0.on_event = Some(VideoEventHandler::new(
            waterui_core::handler::boxed_event_handler(handler),
        ));
        self
    }

    /// Sets subtitle selection binding for the video.
    #[must_use]
    pub fn subtitle_selection(mut self, subtitle_selection: &Binding<SubtitleSelection>) -> Self {
        self.0.subtitle_selection = subtitle_selection.clone();
        self
    }

    /// Sets whether the current queue has a next item.
    #[must_use]
    pub fn has_next(mut self, has_next: &Binding<bool>) -> Self {
        self.0.has_next = has_next.clone();
        self
    }

    /// Sets whether the current queue has a previous item.
    #[must_use]
    pub fn has_previous(mut self, has_previous: &Binding<bool>) -> Self {
        self.0.has_previous = has_previous.clone();
        self
    }

    /// Mutes or unmutes the video based on the provided boolean binding.
    #[must_use]
    pub fn muted(mut self, muted: &Binding<bool>) -> Self {
        self.0.muted = Some(muted.clone());
        self
    }

    /// Sets the volume binding for the video.
    #[must_use]
    pub fn volume(mut self, volume: &Binding<Volume>) -> Self {
        self.0.volume = volume.clone();
        self
    }

    /// Sets playback speed binding for the video.
    #[must_use]
    pub fn playback_rate(mut self, playback_rate: &Binding<f32>) -> Self {
        self.0.playback_rate = playback_rate.clone();
        self
    }

    /// Enables/disables pitch preservation when playback rate is not 1x.
    #[must_use]
    pub fn preserve_pitch(mut self, preserve_pitch: &Binding<bool>) -> Self {
        self.0.preserve_pitch = preserve_pitch.clone();
        self
    }
}

// =============================================================================
// VideoPlayer - Full-featured player with native controls
// =============================================================================

/// Configuration for the [`VideoPlayer`] component.
///
/// This configuration defines a full-featured video player with native controls.
pub struct VideoPlayerConfig<H = Option<VideoEventHandler>> {
    /// The media item to play.
    pub source: Computed<MediaItem>,
    /// Subtitle selection policy for the player's current runtime track list.
    pub subtitle_selection: Binding<SubtitleSelection>,
    /// Whether the current queue has a next item.
    pub has_next: Binding<bool>,
    /// Whether the current queue has a previous item.
    pub has_previous: Binding<bool>,
    /// The volume of the video player.
    pub volume: Binding<Volume>,
    /// Optional mute state composed with [`Self::volume`] during native resolution.
    pub muted: Option<Binding<bool>>,
    /// Playback speed (1.0 = normal speed).
    pub playback_rate: Binding<f32>,
    /// Whether to preserve pitch when changing playback speed.
    pub preserve_pitch: Binding<bool>,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: AspectRatio,
    /// Whether to show native playback controls.
    pub show_controls: bool,
    /// Playback buffering/realtime policy.
    pub playback_policy: PlaybackPolicy,
    /// The event handler for the video player.
    pub on_event: H,
}

impl<H> fmt::Debug for VideoPlayerConfig<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoPlayerConfig")
            .field("aspect_ratio", &self.aspect_ratio)
            .field("show_controls", &self.show_controls)
            .field("playback_policy", &self.playback_policy)
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
            source,
            subtitle_selection,
            has_next,
            has_previous,
            volume,
            muted,
            playback_rate,
            preserve_pitch,
            aspect_ratio,
            show_controls,
            playback_policy,
            on_event,
        } = self;
        VideoPlayerConfig {
            source,
            subtitle_selection,
            has_next,
            has_previous,
            volume,
            muted,
            playback_rate,
            preserve_pitch,
            aspect_ratio,
            show_controls,
            playback_policy,
            on_event: map(on_event),
        }
    }
}

impl VideoPlayerConfig {
    fn bind_event_environment(self, env: &Environment) -> NativeVideoPlayerConfig {
        self.resolve_muted_volume()
            .map_event_handler(|handler| handler.map(|handler| handler.bind(env)))
    }
}

impl<H> VideoPlayerConfig<H> {
    pub(crate) fn resolve_muted_volume(mut self) -> Self {
        if let Some(muted) = self.muted.take() {
            self.volume = muted_volume_binding(self.volume, muted);
        }
        self
    }
}

impl NativeView for NativeVideoPlayerConfig {
    fn stretch_axis(&self) -> StretchAxis {
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
    |config| match config.aspect_ratio {
        AspectRatio::Fit => StretchAxis::Horizontal,
        AspectRatio::Fill | AspectRatio::Stretch => StretchAxis::Both,
    },
    resolve |config, env| config.bind_event_environment(env)
);

impl VideoPlayer {
    /// Creates a new video player with native controls.
    pub fn new(source: impl IntoComputed<MediaItem>) -> Self {
        Self(VideoPlayerConfig {
            source: source.into_computed(),
            subtitle_selection: binding(SubtitleSelection::Auto),
            has_next: Binding::bool(false),
            has_previous: Binding::bool(false),
            volume: binding(0.5_f32),
            muted: None,
            playback_rate: binding(1.0_f32),
            preserve_pitch: binding(true),
            aspect_ratio: AspectRatio::default(),
            show_controls: true,
            playback_policy: PlaybackPolicy::default(),
            on_event: None,
        })
    }

    /// Sets the aspect ratio mode for the video player.
    #[must_use]
    pub const fn aspect_ratio(mut self, aspect_ratio: AspectRatio) -> Self {
        self.0.aspect_ratio = aspect_ratio;
        self
    }

    /// Sets whether to show native playback controls.
    #[must_use]
    pub const fn show_controls(mut self, show_controls: bool) -> Self {
        self.0.show_controls = show_controls;
        self
    }

    /// Sets playback buffering/realtime policy.
    #[must_use]
    pub const fn playback_policy(mut self, playback_policy: PlaybackPolicy) -> Self {
        self.0.playback_policy = playback_policy;
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
        self.0.on_event = Some(VideoEventHandler::new(
            waterui_core::handler::boxed_event_handler(handler),
        ));
        self
    }

    /// Sets subtitle selection binding for the video player.
    #[must_use]
    pub fn subtitle_selection(mut self, subtitle_selection: &Binding<SubtitleSelection>) -> Self {
        self.0.subtitle_selection = subtitle_selection.clone();
        self
    }

    /// Sets whether the current queue has a next item.
    #[must_use]
    pub fn has_next(mut self, has_next: &Binding<bool>) -> Self {
        self.0.has_next = has_next.clone();
        self
    }

    /// Sets whether the current queue has a previous item.
    #[must_use]
    pub fn has_previous(mut self, has_previous: &Binding<bool>) -> Self {
        self.0.has_previous = has_previous.clone();
        self
    }

    /// Mutes or unmutes the video player based on the provided boolean binding.
    #[must_use]
    pub fn muted(mut self, muted: &Binding<bool>) -> Self {
        self.0.muted = Some(muted.clone());
        self
    }

    /// Sets the volume binding for the video player.
    #[must_use]
    pub fn volume(mut self, volume: &Binding<Volume>) -> Self {
        self.0.volume = volume.clone();
        self
    }

    /// Sets playback speed binding for the video player.
    #[must_use]
    pub fn playback_rate(mut self, playback_rate: &Binding<f32>) -> Self {
        self.0.playback_rate = playback_rate.clone();
        self
    }

    /// Enables/disables pitch preservation when playback rate is not 1x.
    #[must_use]
    pub fn preserve_pitch(mut self, preserve_pitch: &Binding<bool>) -> Self {
        self.0.preserve_pitch = preserve_pitch.clone();
        self
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn muted_volume_tracks_both_sources_and_writes_back() {
        let volume = binding(0.5_f32);
        let muted = binding(false);
        let effective = muted_volume_binding(volume.clone(), muted.clone());
        let observed = Rc::new(Cell::new(0.0));
        let captured = Rc::clone(&observed);
        let _guard = effective.watch(move |context| captured.set(context.into_value()));

        volume.set(0.8);
        assert!((observed.get() - 0.8).abs() < f32::EPSILON);

        muted.set(true);
        assert!((observed.get() + 0.8).abs() < f32::EPSILON);

        effective.set(0.4);
        assert!(!muted.get());
        assert!((volume.get() - 0.4).abs() < f32::EPSILON);
        assert!((effective.get() - 0.4).abs() < f32::EPSILON);

        effective.set(-0.3);
        assert!(muted.get());
        assert!((volume.get() - 0.3).abs() < f32::EPSILON);
        assert!((effective.get() + 0.3).abs() < f32::EPSILON);
    }
}
