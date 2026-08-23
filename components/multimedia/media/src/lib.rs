#![doc = include_str!("../README.md")]
//! # `WaterUI` Media Components
//!
//! This crate provides media handling and display components for the `WaterUI` framework.
//! It includes support for images, videos, and Live Photos with a reactive, configurable API.
//!
//! ## Components
//!
//! - [`Photo`]: Display static images with customizable placeholders
//! - [`Video`]: Video sources that can be used with [`VideoPlayer`]
//! - [`VideoPlayer`]: Video playback with reactive volume control
//! - [`LivePhoto`]: Live media display with paired still and motion resources
//! - [`Media`]: Unified enum for different media types
//!
//! ## Features
//!
//! - **Reactive**: All components integrate with `WaterUI`'s reactive system
//! - **Configurable**: Built using `WaterUI`'s configuration pattern
//! - **Media Picker**: Platform-native media selection (feature: `media-picker`)
//! - **Type Safety**: Strong typing with URLs and media sources
//!
//! ## Examples
//!
//! ### Basic Photo
//! ```rust
//! use waterui_media::{Photo, url::Url};
//!
//! let url = Url::parse("https://waterui.dev/favicon.ico").unwrap();
//! let _photo = Photo::new(url);
//! ```
//!
//! ### Video with Controls
//! ```rust
//! use waterui_media::{url::Url, video, video_player};
//!
//! let url = Url::parse("https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4").unwrap();
//! let _video = video::video(url.clone());
//! let _player = video_player(url);
//! ```
//!
//! ### Unified Media Type
//! ```rust
//! use waterui_media::{Media, live::LivePhotoSource, url::Url};
//!
//! let image = Media::Image(Url::parse("https://waterui.dev/favicon.ico").unwrap());
//! let video = Media::Video(Url::parse("https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4").unwrap());
//! let live_photo = Media::LivePhoto(LivePhotoSource::new(
//!     Url::parse("https://waterui.dev/favicon.ico").unwrap(),
//!     Url::parse("https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4").unwrap(),
//! ));
//! assert!(matches!(image, Media::Image(_)));
//! assert!(matches!(video, Media::Video(_)));
//! assert!(matches!(live_photo, Media::LivePhoto(_)));
//! ```

extern crate alloc;

/// Live Photo components and types.
///
/// This module provides the [`LivePhoto`] component for displaying paired still
/// and motion resources, including Apple Live Photos and Android Motion Photos.
pub mod live;
/// Photo components and types.
///
/// This module provides the [`Photo`] component for displaying static images
/// with customizable placeholder views.
pub mod photo;

/// Media picker functionality for platform-native media selection.
pub mod media_picker;
/// Video components and types re-exported from `waterui-video`.
pub mod video {
    pub use waterui_video::video::*;
}
pub use {
    live::LivePhoto,
    media_picker::MediaPicker,
    photo::Photo,
    // `ContentMode` is deliberately not re-exported unqualified: the layout crate
    // owns that name in the prelude for its ratio-box fill mode. Video's own
    // gravity mode stays at `media::video::ContentMode`.
    video::{Event, SubtitleSelection, Video, VideoConfig, VideoPlayer, VideoPlayerConfig, Volume},
    waterui_image::Image,
    waterui_video::{
        Delivery, MediaItem, MediaItemId, PlaybackError, PlaybackPhase, PlaybackSession,
        PlayerController, Playlist, RepeatMode, SubtitleTrack, video_player,
    },
};

/// Re-export the stable [`Filter`] trait from `filtrate-core` for
/// GPU-accelerated image filters.
pub use filtrate_core::Filter;

/// URL types for working with media resources
pub mod url;
pub use url::Url;

use waterui_core::{AnyView, Environment, View, reactive::impl_constant};

use crate::live::LivePhotoSource;

/// A unified media type that can represent different kinds of media content.
///
/// This enum automatically chooses the appropriate component when used as a View:
/// - [`Media::Image`] renders as a [`Photo`] component
/// - [`Media::Video`] renders as a [`VideoPlayer`] component
/// - [`Media::LivePhoto`] renders as a [`LivePhoto`] component
///
/// # Examples
///
/// ```rust
/// use waterui_media::{Media, live::LivePhotoSource, url::Url};
///
/// let image = Media::Image(Url::parse("https://waterui.dev/favicon.ico").unwrap());
/// let video = Media::Video(Url::parse("https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4").unwrap());
/// let live_photo = Media::LivePhoto(LivePhotoSource::new(
///     Url::parse("https://waterui.dev/favicon.ico").unwrap(),
///     Url::parse("https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4").unwrap(),
/// ));
/// assert!(matches!(image, Media::Image(_)));
/// assert!(matches!(video, Media::Video(_)));
/// assert!(matches!(live_photo, Media::LivePhoto(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Media {
    /// An image from a URL that will be displayed using the [`Photo`] component.
    Image(Url),
    /// A Live Photo with image and video components that will be displayed using the [`LivePhoto`] component.
    LivePhoto(LivePhotoSource),
    /// A video from a URL that will be displayed using the [`VideoPlayer`] component.
    Video(Url),
}

impl_constant!(LivePhotoSource, Media);

impl View for Media {
    fn body(self, _env: &Environment) -> impl View {
        match self {
            Self::Image(url) => AnyView::new(Photo::new(url)),
            Self::LivePhoto(live) => AnyView::new(LivePhoto::new(live)),
            Self::Video(url) => AnyView::new(waterui_video::video_player(url)),
        }
    }
}
