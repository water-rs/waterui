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
//! ```rust
//! use waterui_media::Photo;
//! use waterui_core::Text;
//!
//! let photo = Photo::new("https://example.com/image.jpg")
//!     .placeholder(Text::new("Loading..."));
//! ```
//!
//! ### Video with Controls
//! ```rust
//! use waterui_media::{Video, VideoPlayer};
//! use waterui_core::binding;
//!
//! let video = Video::new("https://example.com/video.mp4");
//! let muted = binding(false);
//! let player = VideoPlayer::new(video).muted(&muted);
//! ```
//!
//! ### Unified Media Type
//! ```rust
//! use waterui_media::Media;
//!
//! let media = Media::Image("photo.jpg".into());
//! // Automatically renders as a Photo component when used as a View
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

use waterui_core::{AnyView, Environment, Str, View, reactive::impl_constant};

use crate::live::LivePhotoSource;

type Url = Str;

/// A unified media type that can represent different kinds of media content.
///
/// This enum automatically chooses the appropriate component when used as a View:
/// - [`Media::Image`] renders as a [`Photo`] component
/// - [`Media::Video`] renders as a [`Video`] component  
/// - [`Media::LivePhoto`] renders as a [`LivePhoto`] component
///
/// # Examples
///
/// ```rust
/// use waterui_media::{Media, LivePhotoSource};
///
/// let image = Media::Image("photo.jpg".into());
/// let video = Media::Video("video.mp4".into());
/// let live_photo = Media::LivePhoto(LivePhotoSource::new(
///     "photo.jpg".into(),
///     "video.mov".into()
/// ));
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
