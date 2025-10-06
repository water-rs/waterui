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
//! - [`LivePhoto`]: Apple Live Photo display with image and video components
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
//! ```no_run
//! use waterui_media::{Photo, url::Url};
//!
//! let url = Url::parse("https://example.com/image.jpg").unwrap();
//! let _photo = Photo::new(url);
//! ```
//!
//! ### Video with Controls
//! ```no_run
//! use waterui_core::binding;
//! use waterui_media::{Video, VideoPlayer, url::Url};
//!
//! let url = Url::parse("https://example.com/video.mp4").unwrap();
//! let video = Video::new(url);
//! let muted = binding(false);
//! let _player = VideoPlayer::new(video).muted(&muted);
//! ```
//!
//! ### Unified Media Type
//! ```no_run
//! use waterui_media::{Media, live::LivePhotoSource, url::Url};
//!
//! let image = Media::Image(Url::parse("https://example.com/photo.jpg").unwrap());
//! let video = Media::Video(Url::parse("https://example.com/video.mp4").unwrap());
//! let live_photo = Media::LivePhoto(LivePhotoSource::new(
//!     Url::parse("https://example.com/photo.jpg").unwrap(),
//!     Url::parse("https://example.com/video.mov").unwrap(),
//! ));
//! assert!(matches!(image, Media::Image(_)));
//! assert!(matches!(video, Media::Video(_)));
//! assert!(matches!(live_photo, Media::LivePhoto(_)));
//! ```

#![allow(clippy::future_not_send)]

extern crate alloc;

/// Live Photo components and types.
///
/// This module provides the [`LivePhoto`] component for displaying Apple Live Photos,
/// which consist of both an image and a video component.
pub mod live;
/// Photo components and types.
///
/// This module provides the [`Photo`] component for displaying static images
/// with customizable placeholder views.
pub mod photo;

#[cfg(feature = "media-picker")]
/// Media picker functionality for platform-native media selection.
pub mod picker;
/// Video components and types.
///
/// This module provides [`Video`] sources and [`VideoPlayer`] components
/// for video playback with reactive controls.
pub mod video;
pub use {
    live::LivePhoto,
    photo::Photo,
    video::{Video, VideoPlayer},
};

/// URL types for working with media resources
pub mod url;
pub use url::Url;
/// Image view primitives and supporting types.
pub mod image;

use waterui_core::{AnyView, Environment, View, reactive::impl_constant};

use crate::live::LivePhotoSource;

/// A unified media type that can represent different kinds of media content.
///
/// This enum automatically chooses the appropriate component when used as a View:
/// - [`Media::Image`] renders as a [`Photo`] component
/// - [`Media::Video`] renders as a [`Video`] component  
/// - [`Media::LivePhoto`] renders as a [`LivePhoto`] component
///
/// # Examples
///
/// ```no_run
/// use waterui_media::{Media, live::LivePhotoSource, url::Url};
///
/// let image = Media::Image(Url::parse("https://example.com/photo.jpg").unwrap());
/// let video = Media::Video(Url::parse("https://example.com/video.mp4").unwrap());
/// let live_photo = Media::LivePhoto(LivePhotoSource::new(
///     Url::parse("https://example.com/photo.jpg").unwrap(),
///     Url::parse("https://example.com/video.mov").unwrap(),
/// ));
/// assert!(matches!(image, Media::Image(_)));
/// assert!(matches!(video, Media::Video(_)));
/// assert!(matches!(live_photo, Media::LivePhoto(_)));
/// ```
#[derive(Debug, Clone)]
pub enum Media {
    /// An image from a URL that will be displayed using the [`Photo`] component.
    Image(Url),
    /// A Live Photo with image and video components that will be displayed using the [`LivePhoto`] component.
    LivePhoto(LivePhotoSource),
    /// A video from a URL that will be displayed using the [`Video`] component.
    Video(Url),
}

impl_constant!(LivePhotoSource, Media);

impl View for Media {
    fn body(self, _env: &Environment) -> impl View {
        match self {
            Self::Image(url) => AnyView::new(Photo::new(url)),
            Self::LivePhoto(live) => AnyView::new(LivePhoto::new(live)),
            Self::Video(url) => AnyView::new(Video::new(url)),
        }
    }
}
