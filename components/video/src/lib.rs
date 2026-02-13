//! Video components and playback API for WaterUI.

pub mod url;
pub use url::Url;

pub mod video;
pub use video::{AspectRatio, Event, Video, VideoConfig, VideoPlayer, VideoPlayerConfig, Volume};
