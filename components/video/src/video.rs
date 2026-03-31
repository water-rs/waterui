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
//! let video = Video::new("https://example.com/video.mp4")
//!     .aspect_ratio(AspectRatio::Fill);
//!
//! // Full-featured video player with native controls
//! let player = VideoPlayer::new("https://example.com/video.mp4")
//!     .show_controls(true);
//!
//! // Control volume/mute state
//! let muted = binding(false);
//! let video = Video::new("https://example.com/video.mp4").muted(&muted);
//! muted.set(true);  // Mute - preserves volume level
//! muted.set(false); // Unmute - restores original volume
//! ```

use core::fmt;
use waterui_core::{
    Binding, Computed, binding, configurable, layout::StretchAxis, reactive::signal::IntoComputed,
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

type OnEvent = Box<dyn Fn(Event) + 'static>;

// =============================================================================
// Video - Raw view without controls
// =============================================================================

/// Configuration for the [`Video`] component (raw video view).
///
/// This is a raw video view that displays video content without any native controls.
/// Use this when you want to build your own custom video UI.
pub struct VideoConfig {
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
    pub on_event: OnEvent,
}

impl fmt::Debug for VideoConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoConfig")
            .field("aspect_ratio", &self.aspect_ratio)
            .field("loops", &self.loops)
            .field("playback_policy", &self.playback_policy)
            .finish_non_exhaustive()
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
    }
);

impl Video {
    /// Creates a new raw video view.
    pub fn new(source: impl IntoComputed<MediaItem>) -> Self {
        Self(VideoConfig {
            source: source.into_computed(),
            subtitle_selection: binding(SubtitleSelection::Auto),
            has_next: Binding::bool(false),
            has_previous: Binding::bool(false),
            volume: binding(0.5),
            playback_rate: binding(1.0),
            preserve_pitch: binding(true),
            aspect_ratio: AspectRatio::default(),
            loops: true,
            playback_policy: PlaybackPolicy::default(),
            on_event: Box::new(|_| {}),
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
    #[must_use]
    pub fn on_event(mut self, handler: impl Fn(Event) + 'static) -> Self {
        self.0.on_event = Box::new(handler);
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
        let volume_binding = self.0.volume;
        self.0.volume = Binding::mapping(
            muted,
            {
                let volume_binding = volume_binding.clone();
                move |value| {
                    if value {
                        -volume_binding.get().abs()
                    } else {
                        volume_binding.get().abs()
                    }
                }
            },
            move |binding, value| {
                binding.set(value <= 0.0);
                volume_binding.set(value);
            },
        );
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
pub struct VideoPlayerConfig {
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
    pub on_event: OnEvent,
}

impl fmt::Debug for VideoPlayerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoPlayerConfig")
            .field("aspect_ratio", &self.aspect_ratio)
            .field("show_controls", &self.show_controls)
            .field("playback_policy", &self.playback_policy)
            .finish_non_exhaustive()
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
    }
);

impl VideoPlayer {
    /// Creates a new video player with native controls.
    pub fn new(source: impl IntoComputed<MediaItem>) -> Self {
        Self(VideoPlayerConfig {
            source: source.into_computed(),
            subtitle_selection: binding(SubtitleSelection::Auto),
            has_next: Binding::bool(false),
            has_previous: Binding::bool(false),
            volume: binding(0.5),
            playback_rate: binding(1.0),
            preserve_pitch: binding(true),
            aspect_ratio: AspectRatio::default(),
            show_controls: true,
            playback_policy: PlaybackPolicy::default(),
            on_event: Box::new(|_| {}),
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
    #[must_use]
    pub fn on_event(mut self, handler: impl Fn(Event) + 'static) -> Self {
        self.0.on_event = Box::new(handler);
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
        let volume_binding = self.0.volume;
        self.0.volume = Binding::mapping(
            muted,
            {
                let volume_binding = volume_binding.clone();
                move |value| {
                    if value {
                        -volume_binding.get().abs()
                    } else {
                        volume_binding.get().abs()
                    }
                }
            },
            move |binding, value| {
                binding.set(value <= 0.0);
                volume_binding.set(value);
            },
        );
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
