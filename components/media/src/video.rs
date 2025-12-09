//! Video components and playback controls.
//!
//! This module provides two distinct video components:
//!
//! - [`Video`]: A raw view that displays video without controls (uses AVPlayerLayer/SurfaceView)
//! - [`VideoPlayer`]: A full-featured player with native controls (uses AVPlayerViewController/ExoPlayer)
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
//! use waterui_media::{Video, VideoPlayer};
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

use waterui_core::{
    Binding, Computed, binding, configurable, layout::StretchAxis, reactive::signal::IntoComputed,
};

use crate::Url;

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

/// Events emitted by video components.
#[derive(Debug, Clone)]
pub enum Event {
    /// The video is ready to play.
    ReadyToPlay,
    /// The video has finished playing.
    Ended,
    /// The video is buffering due to slow network or disk.
    Buffering,
    /// The video has resumed playing after buffering.
    BufferingEnded,
    /// An error occurred while loading or playing the video.
    Error {
        /// The error message describing what went wrong.
        message: String
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
    /// The URL of the video source.
    pub source: Computed<Url>,
    /// The volume of the video.
    pub volume: Binding<Volume>,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: AspectRatio,
    /// Whether the video should loop when it ends.
    pub loops: bool,
    /// The event handler for video events.
    pub on_event: OnEvent,
}

impl core::fmt::Debug for VideoConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VideoConfig")
            .field("aspect_ratio", &self.aspect_ratio)
            .field("loops", &self.loops)
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
    /// - **iOS/macOS**: Uses `AVPlayerLayer` directly
    /// - **Android**: Uses `SurfaceView` with ExoPlayer
    Video,
    VideoConfig,
    |config| match config.aspect_ratio {
        AspectRatio::Fit => StretchAxis::Horizontal,
        AspectRatio::Fill | AspectRatio::Stretch => StretchAxis::Both,
    }
);

impl Video {
    /// Creates a new raw video view.
    pub fn new(source: impl IntoComputed<Url>) -> Self {
        Self(VideoConfig {
            source: source.into_computed(),
            volume: binding(0.5),
            aspect_ratio: AspectRatio::default(),
            loops: true,
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

    /// Sets the event handler for video events.
    #[must_use]
    pub fn on_event(mut self, handler: impl Fn(Event) + 'static) -> Self {
        self.0.on_event = Box::new(handler);
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
}

// =============================================================================
// VideoPlayer - Full-featured player with native controls
// =============================================================================

/// Configuration for the [`VideoPlayer`] component.
///
/// This configuration defines a full-featured video player with native controls.
pub struct VideoPlayerConfig {
    /// The URL of the video source.
    pub source: Computed<Url>,
    /// The volume of the video player.
    pub volume: Binding<Volume>,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: AspectRatio,
    /// Whether to show native playback controls.
    pub show_controls: bool,
    /// The event handler for the video player.
    pub on_event: OnEvent,
}

impl core::fmt::Debug for VideoPlayerConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VideoPlayerConfig")
            .field("aspect_ratio", &self.aspect_ratio)
            .field("show_controls", &self.show_controls)
            .finish_non_exhaustive()
    }
}

configurable!(
    /// A full-featured video player with native playback controls.
    ///
    /// Use this component when you want a complete video playback experience
    /// with platform-native controls (play/pause, seek, fullscreen, etc.).
    /// For a raw video view without controls, use [`Video`] instead.
    ///
    /// # Platform Implementation
    ///
    /// - **iOS/tvOS**: Uses `AVPlayerViewController` with standard iOS controls
    /// - **macOS**: Uses `AVPlayerView` with inline controls
    /// - **Android**: Uses ExoPlayer with `PlayerView`
    VideoPlayer,
    VideoPlayerConfig,
    |config| match config.aspect_ratio {
        AspectRatio::Fit => StretchAxis::Horizontal,
        AspectRatio::Fill | AspectRatio::Stretch => StretchAxis::Both,
    }
);

impl VideoPlayer {
    /// Creates a new video player with native controls.
    pub fn new(source: impl IntoComputed<Url>) -> Self {
        Self(VideoPlayerConfig {
            source: source.into_computed(),
            volume: binding(0.5),
            aspect_ratio: AspectRatio::default(),
            show_controls: true,
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

    /// Sets the event handler for the video player.
    #[must_use]
    pub fn on_event(mut self, handler: impl Fn(Event) + 'static) -> Self {
        self.0.on_event = Box::new(handler);
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
}
