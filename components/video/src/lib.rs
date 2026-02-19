//! Video components and playback API for WaterUI.

use waterui_core::Environment;

pub mod url;
pub use url::Url;

pub mod video;
pub use video::{AspectRatio, Event, Video, VideoConfig, VideoPlayer, VideoPlayerConfig, Volume};

#[cfg(any(target_os = "android", feature = "rust-fallback-player"))]
mod fallback;

/// Installs platform video hooks into the provided environment.
///
/// On Android this installs Rust fallback hooks so `Video`/`VideoPlayer`
/// render through the WaterUI GPU pipeline instead of native player widgets.
///
/// Enable crate feature `rust-fallback-player` to force this route on
/// non-Android platforms (useful for demoing and validating the Rust player).
pub fn install_platform_hooks(env: &mut Environment) {
    #[cfg(any(target_os = "android", feature = "rust-fallback-player"))]
    {
        fallback::install_platform_hooks(env);
    }

    #[cfg(not(any(target_os = "android", feature = "rust-fallback-player")))]
    {
        let _ = env;
    }
}
