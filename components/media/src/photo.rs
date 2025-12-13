//! A Photo component that displays an image from a URL.
//!
//! # Example
//!
//! ```no_run
//! use waterui_media::Photo;
//! use waterui_media::url::Url;
//!
//! let url = Url::parse("https://example.com/image.jpg").unwrap();
//! let _photo = Photo::new(url);
//! ```
use crate::image::Image;
use waterui_core::configurable;

use crate::Url;

/// Configuration for the Photo component.
#[allow(missing_debug_implementations)]
pub struct PhotoConfig {
    /// The URL of the image to display.
    pub source: Url,
    /// Event handler for photo loading events.
    pub on_event: OnEvent,
}

type OnEvent = Box<dyn Fn(Event) + 'static>;

/// Events emitted by the Photo component.
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
            on_event: Box::new(|_event| {
                // No-op default handler
            }),
        })
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
