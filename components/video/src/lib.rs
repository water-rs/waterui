//! Video components and playback API for `WaterUI`.

#![allow(
    dead_code,
    missing_docs,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::collapsible_if,
    clippy::doc_markdown,
    clippy::large_stack_arrays,
    clippy::manual_is_multiple_of,
    clippy::manual_let_else,
    clippy::manual_range_contains,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::must_use_candidate,
    clippy::manual_range_patterns,
    clippy::needless_continue,
    clippy::needless_pass_by_value,
    clippy::needless_return,
    clippy::option_if_let_else,
    clippy::map_unwrap_or,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::redundant_closure_for_method_calls,
    clippy::redundant_pub_crate,
    clippy::semicolon_if_nothing_returned,
    clippy::struct_excessive_bools,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::used_underscore_binding,
    clippy::while_let_on_iterator
)]

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
/// This forces `Video` / `VideoPlayer` to render through the `WaterUI`
/// `GpuSurface` playback pipeline.
pub fn install_rust_player_hooks(env: &mut Environment) {
    runtime_player::install_platform_hooks(env);
}

/// Installs platform video hooks into the provided environment.
///
/// On Android this installs Rust player hooks so `Video`/`VideoPlayer`
/// render through the `WaterUI` GPU pipeline instead of native player widgets.
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
