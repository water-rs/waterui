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
//! ```rust
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
//! ```

use waterui_core::{
    Binding, Computed, View, binding, configurable,
    reactive::{impl_constant, signal::IntoComputed},
};

use crate::Url;

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
pub type Volume = f64;

/// Configuration for the [`VideoPlayer`] component.
///
/// This configuration defines the video source and volume control for the player.
#[derive(Debug)]
pub struct VideoPlayerConfig {
    /// The video to be played.
    pub video: Computed<Video>,
    /// The volume of the video player.
    ///
    /// Uses the special [`Volume`] type that preserves volume levels when muted.
    pub volume: Binding<Volume>,
}

configurable!(VideoPlayer, VideoPlayerConfig);

/// A video source represented by a URL.
///
/// This type represents a video that can be played by a [`VideoPlayer`].
/// When used as a [`View`], it automatically creates a [`VideoPlayer`].
///
/// # Examples
///
/// ```rust
/// use waterui_media::Video;
///
/// let video = Video::new("https://example.com/video.mp4");
/// // When used as a View, automatically becomes a VideoPlayer
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
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
    /// ```rust
    /// use waterui_media::Video;
    ///
    /// let video = Video::new("https://example.com/video.mp4");
    /// let local_video = Video::new("file:///path/to/video.mov");
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
    /// ```rust
    /// use waterui_media::Video;
    ///
    /// let video = Video::new("video.mp4");
    /// let player = video.player();
    /// ```
    #[must_use]
    pub fn player(self) -> VideoPlayer {
        todo!()
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
        })
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
