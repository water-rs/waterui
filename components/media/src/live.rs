use waterui_core::{Computed, configurable, reactive::compute::IntoComputed};

use crate::Url;

#[derive(Debug)]
pub struct LivePhotoConfig {
    pub source: Computed<LivePhotoSource>,
}

configurable!(LivePhoto, LivePhotoConfig);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LivePhotoSource {
    image: Url,
    video: Url,
}

impl LivePhotoSource {
    pub const fn new(image: Url, video: Url) -> Self {
        Self { image, video }
    }
}

impl LivePhoto {
    pub fn new(source: impl IntoComputed<LivePhotoSource>) -> Self {
        Self(LivePhotoConfig {
            source: source.into_computed(),
        })
    }
}
