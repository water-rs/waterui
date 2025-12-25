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
//! let photo = Photo::new(url)
//!     .blur(5.0)
//!     .brightness(0.1);
//! ```

use alloc::boxed::Box;
use alloc::vec::Vec;

use executor_core::spawn_local;
use filtrate::Filter;
use image::GenericImageView;
use waterui_core::dynamic::{Dynamic, DynamicHandler};
use waterui_core::{Environment, View};
use zenwave::ResponseExt;

use crate::image::Image;
use crate::Url;

/// A photo component that displays an image from a URL.
///
/// Photo fetches the image asynchronously and displays it using the
/// GPU-accelerated [`Image`] view once loaded. Filters can be applied
/// that will run on the GPU.
///
/// # Example
///
/// ```ignore
/// use waterui_media::Photo;
///
/// Photo::new("https://example.com/image.jpg")
///     .blur(5.0)
///     .saturation(1.2)
/// ```
pub struct Photo {
    /// The URL of the image to display.
    url: Url,
    /// Filters to apply to the image.
    filters: Vec<Filter>,
    /// Event handler for photo loading events.
    on_event: Option<Box<dyn Fn(Event) + 'static>>,
}

impl core::fmt::Debug for Photo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Photo")
            .field("url", &self.url)
            .field("filters", &self.filters)
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
            filters: Vec::new(),
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

    /// Apply a gaussian blur filter.
    ///
    /// # Arguments
    ///
    /// * `radius` - Blur radius in pixels (higher = more blur)
    #[must_use]
    pub fn blur(mut self, radius: f32) -> Self {
        self.filters.push(Filter::Blur { radius });
        self
    }

    /// Adjust brightness.
    ///
    /// # Arguments
    ///
    /// * `amount` - Brightness adjustment (-1.0 = black, 0.0 = unchanged, 1.0 = white)
    #[must_use]
    pub fn brightness(mut self, amount: f32) -> Self {
        self.filters.push(Filter::Brightness { amount });
        self
    }

    /// Adjust color saturation.
    ///
    /// # Arguments
    ///
    /// * `amount` - Saturation multiplier (0.0 = grayscale, 1.0 = unchanged, >1.0 = more saturated)
    #[must_use]
    pub fn saturation(mut self, amount: f32) -> Self {
        self.filters.push(Filter::Saturation { amount });
        self
    }

    /// Adjust contrast.
    ///
    /// # Arguments
    ///
    /// * `amount` - Contrast multiplier (0.0 = gray, 1.0 = unchanged, >1.0 = more contrast)
    #[must_use]
    pub fn contrast(mut self, amount: f32) -> Self {
        self.filters.push(Filter::Contrast { amount });
        self
    }

    /// Convert to grayscale.
    ///
    /// # Arguments
    ///
    /// * `intensity` - Mix factor (0.0 = original, 1.0 = full grayscale)
    #[must_use]
    pub fn grayscale(mut self, intensity: f32) -> Self {
        self.filters.push(Filter::Grayscale { intensity });
        self
    }

    /// Rotate hue around the color wheel.
    ///
    /// # Arguments
    ///
    /// * `angle` - Rotation angle in degrees (0-360)
    #[must_use]
    pub fn hue_rotate(mut self, angle: f32) -> Self {
        self.filters.push(Filter::HueRotation { angle });
        self
    }

    /// Invert all colors.
    #[must_use]
    pub fn invert(mut self) -> Self {
        self.filters.push(Filter::Invert);
        self
    }

    /// Apply sepia tone effect.
    ///
    /// # Arguments
    ///
    /// * `intensity` - Sepia intensity (0.0 = original, 1.0 = full sepia)
    #[must_use]
    pub fn sepia(mut self, intensity: f32) -> Self {
        self.filters.push(Filter::Sepia { intensity });
        self
    }

    /// Add vignette effect (darkened corners).
    ///
    /// # Arguments
    ///
    /// * `radius` - Inner radius where vignette starts (0.0-1.0)
    /// * `softness` - How soft the vignette edge is (0.0-1.0)
    #[must_use]
    pub fn vignette(mut self, radius: f32, softness: f32) -> Self {
        self.filters.push(Filter::Vignette { radius, softness });
        self
    }

    /// Sharpen image details.
    ///
    /// # Arguments
    ///
    /// * `amount` - Sharpening strength (0.0 = unchanged, 1.0 = normal, >1.0 = more sharp)
    #[must_use]
    pub fn sharpen(mut self, amount: f32) -> Self {
        self.filters.push(Filter::Sharpen { amount });
        self
    }

    /// Apply a custom filter.
    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filters.push(filter);
        self
    }
}

impl View for Photo {
    fn body(self, _env: &Environment) -> impl View {
        let (handler, view) = Dynamic::new();
        // Set initial empty view (loading state)
        handler.set(());

        spawn_load_task(self.url, self.filters, self.on_event, handler);

        view
    }
}

fn spawn_load_task(
    url: Url,
    filters: Vec<Filter>,
    on_event: Option<Box<dyn Fn(Event) + 'static>>,
    handler: DynamicHandler,
) {
    spawn_local(async move {
        match fetch_and_decode(url).await {
            Ok((pixels, width, height)) => {
                // Create Image with filters
                let mut image = Image::new(pixels, width, height);
                for filter in filters {
                    image = image.filter(filter);
                }

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
