use waterui_core::{
    Computed, configurable,
    reactive::{compute::IntoComputed, ffi_computed},
};

use crate::Url;

#[derive(Debug, uniffi::Record)]
pub struct LivePhotoConfig {
    pub source: Computed<LivePhotoSource>,
}

ffi_computed!(LivePhotoSource);

configurable!(LivePhoto, LivePhotoConfig);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Record)]
pub struct LivePhotoSource {
    image: Url,
    video: Url,
}

impl LivePhotoSource {
    pub fn new(image: Url, video: Url) -> Self {
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
