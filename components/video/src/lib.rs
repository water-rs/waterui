//! Video components and playback API for WaterUI.

use waterui_core::Environment;

pub mod url;
pub use url::Url;

pub mod source;
pub use source::{Delivery, MediaItem, SubtitleTrack};

pub mod video;
pub use video::{
    AspectRatio, Event, PlaybackPolicy, SubtitleSelection, Video, VideoConfig, VideoPlayer,
    VideoPlayerConfig, Volume,
};

mod runtime_player;
mod subtitles;

/// Installs the Rust/GPU video player hooks into the provided environment.
///
/// This forces `Video` / `VideoPlayer` to render through the WaterUI
/// `GpuSurface` playback pipeline.
pub fn install_rust_player_hooks(env: &mut Environment) {
    runtime_player::install_platform_hooks(env);
}

/// Installs platform video hooks into the provided environment.
///
/// On Android this installs Rust player hooks so `Video`/`VideoPlayer`
/// render through the WaterUI GPU pipeline instead of native player widgets.
pub fn install_platform_hooks(env: &mut Environment) {
    #[cfg(target_os = "android")]
    {
        install_rust_player_hooks(env);
    }

    #[cfg(not(target_os = "android"))]
    {
        if std::env::var_os("WATERUI_VIDEO_FORCE_RUST_PLAYER").is_some() {
            install_rust_player_hooks(env);
        }
    }
}
