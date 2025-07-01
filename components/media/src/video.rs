use waterui_core::{
    Binding, Computed, View, binding, configurable,
    reactive::{compute::IntoComputed, impl_constant},
};

use crate::Url;

/// A Volume value represents the audio volume level of a player.
///
/// In a non-muted state, the volume is represented as a positive value (> 0).
/// When muted, the volume is stored as a negative value (< 0),
/// which preserves the original volume level. This allows the player
/// to return to the previous volume setting when unmuted.
///
/// For example:
/// - Volume 0.7 (70%) is stored as 0.7
/// - When muted, 0.7 becomes -0.7
/// - When unmuted, -0.7 becomes 0.7 again
type Volume = f64;

#[derive(Debug)]
pub struct VideoPlayerConfig {
    pub video: Computed<Video>,
    pub volume: Binding<Volume>,
}


configurable!(VideoPlayer, VideoPlayerConfig);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Video {
    url: Url,
}

impl_constant!(Video);

impl Video {
    pub fn new(url: impl Into<Url>) -> Self {
        Self { url: url.into() }
    }
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
    pub fn new(video: impl IntoComputed<Video>) -> Self {
        Self(VideoPlayerConfig {
            video: video.into_computed(),
            volume: binding(0.5),
        })
    }

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
                        volume_binding.get().abs() * -1.0
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
