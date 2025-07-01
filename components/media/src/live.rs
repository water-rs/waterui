use waterui_core::{Computed, configurable, reactive::compute::IntoComputed};

use crate::Url;

#[derive(Debug)]
/// Configuration for the [`LivePhoto`] component.
pub struct LivePhotoConfig {
    /// The source of the live photo.
    pub source: Computed<LivePhotoSource>,
}

configurable!(LivePhoto, LivePhotoConfig);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Represents the source URLs for a live photo, including the image and video components.
pub struct LivePhotoSource {
    image: Url,
    video: Url,
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
