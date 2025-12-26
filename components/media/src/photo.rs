//! A Photo component that displays an image from a URL.
//!
//! The Photo component fetches an image from a URL, decodes it, and displays
//! it using the GPU-accelerated [`Image`] view. It uses async loading with
//! a dynamic view that updates when the image is ready.
//!
//! # Example
//!
//! ```ignore
//! use waterui_media::Photo;
//! use waterui_media::url::Url;
//!
//! let url = Url::parse("https://example.com/image.jpg").unwrap();
//! let photo = Photo::new(url);
//! ```

use alloc::boxed::Box;
use alloc::vec::Vec;

use executor_core::spawn_local;
use image::GenericImageView;
use waterui_core::dynamic::{Dynamic, DynamicHandler};
use waterui_core::{Environment, View};
use crate::image::Image;
use crate::Url;

/// A photo component that displays an image from a URL.
///
/// Photo fetches the image asynchronously and displays it using the
/// GPU-accelerated [`Image`] view once loaded.
///
/// # Example
///
/// ```ignore
/// use waterui_media::Photo;
///
/// Photo::new("https://example.com/image.jpg")
/// ```
pub struct Photo {
    /// The URL of the image to display.
    url: Url,
    /// Event handler for photo loading events.
    on_event: Option<Box<dyn Fn(Event) + 'static>>,
}

impl core::fmt::Debug for Photo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Photo")
            .field("url", &self.url)
            .field("on_event", &self.on_event.is_some())
            .finish()
    }
}

/// Events emitted by the Photo component.
#[derive(Debug, Clone)]
pub enum Event {
    /// The image has finished loading.
    Loaded,
    /// The image has failed to load.
    Error(String),
}

impl Photo {
    /// Creates a new `Photo` component with the specified image source URL.
    ///
    /// # Arguments
    ///
    /// * `source` - The URL of the image to display.
    #[must_use]
    pub fn new(source: impl Into<Url>) -> Self {
        Self {
            url: source.into(),
            on_event: None,
        }
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
        self.on_event = Some(Box::new(handler));
        self
    }
}

impl View for Photo {
    fn body(self, _env: &Environment) -> impl View {
        let (handler, view) = Dynamic::new();
        // Set initial empty view (loading state)
        handler.set(());

        spawn_load_task(self.url, self.on_event, handler);

        view
    }
}

fn spawn_load_task(
    url: Url,
    on_event: Option<Box<dyn Fn(Event) + 'static>>,
    handler: DynamicHandler,
) {
    spawn_local(async move {
        match fetch_and_decode(url).await {
            Ok((pixels, width, height)) => {
                let image = Image::new(pixels, width, height);
                handler.set(image);

                if let Some(on_event) = on_event {
                    on_event(Event::Loaded);
                }
            }
            Err(e) => {
                tracing::error!("[Photo] Failed to load: {}", e);
                if let Some(on_event) = on_event {
                    on_event(Event::Error(e));
                }
            }
        }
    })
    .detach();
}

async fn fetch_and_decode(url: Url) -> Result<(Vec<u8>, u32, u32), String> {
    // Fetch the image data using redirect-following client
    use zenwave::{Client, Method, redirect::FollowRedirect};
    let mut client = FollowRedirect::new(zenwave::client());
    let response = client
        .method(Method::GET, url.as_str())
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let data = response.into_body().into_bytes().await.map_err(|e| e.to_string())?;

    // Decode on a background thread to avoid blocking
    blocking::unblock(move || {
        let img = image::load_from_memory(&data).map_err(|e| format!("Image decode failed: {}", e))?;

        let (width, height) = img.dimensions();
        let rgba = img.into_rgba8();
        let pixels = rgba.into_raw();

        Ok((pixels, width, height))
    })
    .await
}

/// Convenience constructor for building a `Photo` component inline.
#[must_use]
pub fn photo(source: impl Into<Url>) -> Photo {
    Photo::new(source)
}
