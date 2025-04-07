//! A Photo component that displays an image from a URL.
//!
//! # Example
//!
//! ```
//! use waterui_core::*;
//! use waterui::*;
//!
//! let photo = Photo::new("https://example.com/image.jpg")
//!     .placeholder(Text::new("Loading..."));
//! ```

use waterui_core::{AnyView, configurable};

use crate::Url;

#[derive(Debug)]
pub struct PhotoConfig {
    pub source: Url,
    pub placeholder: AnyView,
}

configurable!(Photo, PhotoConfig);

impl Photo {
    pub fn new(source: impl Into<Url>) -> Self {
        Self(PhotoConfig {
            source: source.into(),
            placeholder: AnyView::default(),
        })
    }

    pub fn placeholder(mut self, placeholder: impl Into<AnyView>) -> Self {
        self.0.placeholder = placeholder.into();
        self
    }

    pub fn disable_live() {}
}
