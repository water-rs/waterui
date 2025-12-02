//! A Photo component that displays an image from a URL.
//!
//! # Example
//!
//! ```no_run
//! use waterui_media::Photo;
//! use waterui_media::url::Url;
//!
//! let url = Url::parse("https://example.com/image.jpg").unwrap();
//! let _photo = Photo::new(url).placeholder(waterui_core::AnyView::new(()));
//! ```
use crate::image::Image;
use waterui_core::{AnyView, configurable};

use crate::Url;

/// Configuration for the Photo component, including the image source and placeholder view.
#[derive(Debug)]
pub struct PhotoConfig {
    /// The URL of the image to display.
    pub source: Url,
    /// The view to display while the image is loading or unavailable.
    pub placeholder: AnyView,
}

configurable!(
    #[doc = "A static photo component that displays remote imagery with placeholders."]
    Photo,
    PhotoConfig
);

impl Photo {
    /// Creates a new `Photo` component with the specified image source URL.
    ///
    /// # Arguments
    ///
    /// * `source` - The URL of the image to display.
    pub fn new(source: impl Into<Url>) -> Self {
        Self(PhotoConfig {
            source: source.into(),
            placeholder: AnyView::default(),
        })
    }

    /// Sets the placeholder view to display while the image is loading or unavailable.
    ///
    /// # Arguments
    ///
    /// * `placeholder` - The view to display as a placeholder.
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<AnyView>) -> Self {
        self.0.placeholder = placeholder.into();
        self
    }

    /// Loads the image associated with this `Photo`.
    ///
    /// # Panics
    ///
    /// Panics because the loader is not implemented yet.
    #[allow(clippy::unused_async)]
    pub async fn load(&self) -> Image {
        todo!()
    }
}

/// Convenience constructor for building a `Photo` component inline.
pub fn photo(source: impl Into<Url>) -> Photo {
    Photo::new(source)
}
