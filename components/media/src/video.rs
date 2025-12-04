//! Video components and playback controls.
//!
//! This module provides the [`Video`] type for representing video sources and
//! the [`VideoPlayer`] component for video playback with reactive volume control.
//!
//! ## Volume Control System
//!
//! The video player uses a unique volume system where:
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
//! let video = Video::new("https://example.com/video.mp4");
//! let muted = binding(false);
//! let player = VideoPlayer::new(video).muted(&muted);
//!
//! // Mute the video - internally preserves volume level as negative value
//! muted.set(true);
//!
//! // Unmute - restores original volume level
//! muted.set(false);
//! ```ignore

use waterui_core::{
    Binding, Computed, NativeView, View, binding, configurable,
    layout::StretchAxis,
    reactive::{impl_constant, signal::IntoComputed},
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

/// Configuration for the [`VideoPlayer`] component.
///
/// This configuration defines the video source and volume control for the player.
pub struct VideoPlayerConfig {
    /// The video to be played.
    pub video: Computed<Video>,
    /// The volume of the video player.
    ///
    /// Uses the special [`Volume`] type that preserves volume levels when muted.
    pub volume: Binding<Volume>,
    /// The aspect ratio mode for video playback.
    pub aspect_ratio: AspectRatio,
    /// Whether to show native playback controls.
    pub show_controls: bool,
    /// The event handler for the video player.
    pub on_event: OnEvent,
}

/// Events emitted by the video player.
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
    Error { message: String },
}

type OnEvent = Box<dyn Fn(Event) + 'static>;

configurable!(
    #[doc = "An interactive video player component with reactive volume control."]
    VideoPlayer,
    VideoPlayerConfig
);

/// A video source represented by a URL.
///
/// This type represents a video that can be played by a [`VideoPlayer`].
/// When used as a [`View`], it automatically creates a [`VideoPlayer`].
///
/// # Examples
///
/// ```no_run
/// use waterui_media::{Video, url::Url};
///
/// let video = Video::new(Url::parse("https://example.com/video.mp4").unwrap());
/// let _view = waterui_core::AnyView::new(video);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Video {
    url: Url,
}

impl_constant!(Video);

impl Video {
    /// Creates a new [`Video`] instance from a URL.
    ///
    /// # Arguments
    ///
    /// * `url` - The URL of the video source
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use waterui_media::{Video, url::Url};
    ///
    /// let video = Video::new(Url::parse("https://example.com/video.mp4").unwrap());
    /// let local_video = Video::new(Url::parse("file:///path/to/video.mov").unwrap());
    /// assert!(video.url().as_str().ends_with("video.mp4"));
    /// ```
    pub fn new(url: impl Into<Url>) -> Self {
        Self { url: url.into() }
    }

    /// Creates a video player for this video.
    ///
    /// # Returns
    ///
    /// A [`VideoPlayer`] configured to play this video.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use waterui_media::{Video, url::Url};
    ///
    /// let video = Video::new(Url::parse("https://example.com/video.mp4").unwrap());
    /// let _player = video.player();
    /// ```
    #[must_use]
    pub fn player(self) -> VideoPlayer {
        todo!()
    }

    /// Returns the URL of the video.
    #[must_use]
    pub const fn url(&self) -> &Url {
        &self.url
    }
}

impl View for Video {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        VideoPlayer::new(self)
    }
}

impl VideoPlayer {
    /// Creates a new `VideoPlayer`.
    pub fn new(video: impl IntoComputed<Video>) -> Self {
        Self(VideoPlayerConfig {
            video: video.into_computed(),
            volume: binding(0.5),
            aspect_ratio: AspectRatio::default(),
            show_controls: false,
            on_event: Box::new(|_event| {
                // No-op default handler
            }),
        })
    }

    /// Sets the aspect ratio mode for the video player.
    #[must_use]
    pub fn aspect_ratio(mut self, aspect_ratio: AspectRatio) -> Self {
        self.0.aspect_ratio = aspect_ratio;
        self
    }

    /// Sets whether to show native playback controls.
    #[must_use]
    pub fn show_controls(mut self, show_controls: bool) -> Self {
        self.0.show_controls = show_controls;
        self
    }

    /// Sets the event handler for the video player.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use waterui_media::{VideoPlayer, Video, Event};
    ///
    /// let player = VideoPlayer::new(video)
    ///     .on_event(|event| {
    ///         match event {
    ///             Event::ReadyToPlay => println!("Video ready!"),
    ///             Event::Error { message } => println!("Error: {}", message),
    ///             Event::Ended => println!("Video ended"),
    ///         }
    ///     });
    /// ```
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
                    // Convert the volume based on mute state
                    if value {
                        // If muted, return negative volume (if positive) to preserve the value
                        -volume_binding.get().abs()
                    } else {
                        // If unmuted, return positive volume (if negative)
                        volume_binding.get().abs()
                    }
                }
            },
            move |binding, value| {
                // Handle changes to volume when mute state changes
                binding.set(value <= 0.0);
                volume_binding.set(value);
            },
        );

        self
    }
}

impl NativeView for VideoPlayer {
    fn stretch_axis(&self) -> StretchAxis {
        // Stretch behavior depends on aspect ratio mode:
        // - Fit: Video maintains aspect ratio, so it stretches horizontally but has fixed height based on content
        // - Fill: Video fills entire space in both directions (may crop)
        // - Stretch: Video fills entire space and stretches to fit (distorts aspect ratio)
        match self.0.aspect_ratio {
            AspectRatio::Fit => StretchAxis::Horizontal,
            AspectRatio::Fill | AspectRatio::Stretch => StretchAxis::Both,
        }
    }
}
