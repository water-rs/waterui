//! Video components and playback API for `WaterUI`.

/// URL type used by video sources.
pub mod url;
pub use url::Url;

/// Media source and subtitle track descriptions.
pub mod source;
pub use source::{
    Delivery, DrmConfiguration, MediaItem, MediaItemId, MediaMetadata, OfflineDrmKeySet,
    SubtitleTrack,
};

/// Owned playback sessions, controllers, playlists, and reactive state.
pub mod session;
pub use session::{
    PlaybackError, PlaybackPhase, PlaybackSession, PlayerController, Playlist, RepeatMode,
};

/// The leaf contract a `Fit` video answers by.
pub mod layout;
pub use layout::{DEFAULT_ASPECT, FitVideoLayout, fit_video};

/// Public video view configuration types.
pub mod video;
pub use video::{
    AudioTrackInfo, AudioTrackSelection, ContentMode, EquirectangularProjection, Event, LiveWindow,
    NetworkPlaybackPolicy, PlaybackConfiguration, PlaybackMetrics, PlaybackOutputPath,
    PlaybackPolicy, PlaybackPowerPolicy, SphericalStereoLayout, SphericalViewport,
    SubtitleSelection, SubtitleTrackInfo, SubtitleTrackOrigin, TimedMetadata, TrackCatalog, Video,
    VideoConfig, VideoPlayer, VideoPlayerConfig, VideoProjection, VideoTrackInfo,
    VideoTrackSelection, Volume, video, video_player,
};
