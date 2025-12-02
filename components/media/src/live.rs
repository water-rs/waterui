use waterui_core::{Computed, configurable, reactive::signal::IntoComputed};

use crate::Url;

#[derive(Debug)]
/// Configuration for the [`LivePhoto`] component.
pub struct LivePhotoConfig {
    /// The source of the live photo.
    pub source: Computed<LivePhotoSource>,
}

configurable!(
    #[doc = "A live photo widget that combines still and motion imagery."]
    LivePhoto,
    LivePhotoConfig
);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Represents the source URLs for a live photo, including the image and video components.
pub struct LivePhotoSource {
    /// The URL for the still image component of the live photo.
    pub image: Url,
    /// The URL for the video component of the live photo.
    pub video: Url,
}

impl LivePhotoSource {
    /// Creates a new `LivePhotoSource` instance.
    #[must_use]
    pub const fn new(image: Url, video: Url) -> Self {
        Self { image, video }
    }
}

impl LivePhoto {
    /// Creates a new `LivePhoto` instance.
    #[must_use]
    pub fn new(source: impl IntoComputed<LivePhotoSource>) -> Self {
        Self(LivePhotoConfig {
            source: source.into_computed(),
        })
    }
}
