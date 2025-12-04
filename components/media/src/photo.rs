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
use waterui_core::{AnyView, Environment, View, configurable};

use crate::Url;

/// Configuration for the Photo component, including the image source and placeholder view.
pub struct PhotoConfig {
    /// The URL of the image to display.
    pub source: Url,
    /// The view to display while the image is loading or unavailable.
    pub placeholder: AnyView,
    pub on_event: OnEvent,
}

type OnEvent = Box<dyn Fn(Event) + 'static>;

#[derive(Debug, Clone)]
pub enum Event {
    /// The image has finished loading.
    Loaded,
    /// The image has failed to load.
    Error(String),
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
            on_event: Box::new(|_event| {
                // No-op default handler
            }),
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

    /// Sets the event handler for the photo.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use waterui_media::{Photo, photo::Event};
    ///
    /// let photo = Photo::new(url)
    ///     .on_event(|event| {
    ///         match event {
    ///             Event::Loaded => println!("Image loaded!"),
    ///             Event::Error(msg) => println!("Error: {}", msg),
    ///         }
    ///     });
    /// ```
    #[must_use]
    pub fn on_event(mut self, handler: impl Fn(Event) + 'static) -> Self {
        self.0.on_event = Box::new(handler);
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
