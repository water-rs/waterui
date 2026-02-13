//! Video components and playback API for WaterUI.

use waterui_core::Environment;

pub mod url;
pub use url::Url;

pub mod video;
pub use video::{AspectRatio, Event, Video, VideoConfig, VideoPlayer, VideoPlayerConfig, Volume};

#[cfg(target_os = "android")]
mod fallback;

/// Installs platform video hooks into the provided environment.
///
/// On Android this installs Rust fallback hooks so `Video`/`VideoPlayer`
/// render through the WaterUI GPU pipeline instead of native player widgets.
pub fn install_platform_hooks(env: &mut Environment) {
    #[cfg(target_os = "android")]
    {
        fallback::install_platform_hooks(env);
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = env;
    }
}
