use std::{io, path::Path, sync::Arc};

use base64::{Engine, prelude::BASE64_STANDARD};
use blocking::unblock;
use image::{DynamicImage, GenericImageView, ImageFormat};
use mime::Mime;
use waterui_color::{Srgb, WithOpacity};

/// Represents a loaded image.
#[derive(Debug, Clone)]
pub struct Image {
    mime: Mime,
    image: Arc<DynamicImage>,
}

impl Image {
    /// Creates a new `Image` from raw image data.
    ///
    /// It will decode the image on a background thread, preventing UI blocking.
    ///
    /// # Panics
    ///
    /// Panics if the MIME type is not supported or if the image data cannot be decoded.
    #[must_use]
    pub async fn new(mime: Mime, data: Vec<u8>) -> Self {
        let format =
            ImageFormat::from_mime_type(mime.essence_str()).expect("Unsupported MIME type");

        let image = unblock(move || {
            image::load_from_memory_with_format(data.as_ref(), format)
                .expect("Failed to decode image")
        })
        .await;

        Self {
            mime,
            image: Arc::new(image),
        }
    }

    /// Process the image with a closure on a background thread
    pub async fn process<F>(&mut self, func: F)
    where
        F: FnOnce(Arc<DynamicImage>) -> DynamicImage + Send + 'static,
    {
        let image = self.image.clone();
        self.image = unblock(move || Arc::new(func(image))).await;
    }

    /// Encodes the image to the specified MIME type.
    ///
    /// # Panics
    ///
    /// Panics if the MIME type is not supported or if encoding fails.
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub async fn encode(&self, mime: Mime) -> Vec<u8> {
        let format = ImageFormat::from_mime_type(mime.essence_str()).unwrap();
        let image = self.image.clone();
        unblock(move || {
            let mut buf = std::io::Cursor::new(Vec::new());
            image
                .write_to(&mut buf, format)
                .expect("Failed to encode image");
            buf.into_inner()
        })
        .await
    }

    /// Encodes the image as PNG.
    #[must_use]
    pub async fn encode_png(&self) -> Vec<u8> {
        self.encode(mime::IMAGE_PNG).await
    }

    /// Encodes the image as JPEG with the specified quality (currently unused).
    #[must_use]
    pub async fn encode_jpeg(&self, _quality: u8) -> Vec<u8> {
        self.encode(mime::IMAGE_JPEG).await
    }

    /// Rotates the image by the specified angle in degrees (0, 90, 180, or 270).
    ///
    /// # Panics
    ///
    /// Panics if the angle is not a multiple of 90 degrees or is outside the range 0-359.
    pub async fn rotate(&mut self, angle: u32) {
        self.process(move |image| match angle % 360 {
            0 => (*image).clone(),
            90 => image.rotate90(),
            180 => image.rotate180(),
            270 => image.rotate270(),
            _ => panic!("Unsupported rotation angle: {angle}"),
        })
        .await;
    }

    /// Get the width of the image in pixels
    #[must_use]
    pub fn width(&self) -> u32 {
        self.image.width()
    }

    /// Get the height of the image in pixels
    #[must_use]
    pub fn height(&self) -> u32 {
        self.image.height()
    }

    /// Get the dimensions (width, height) of the image
    #[must_use]
    pub fn dimensions(&self) -> (u32, u32) {
        self.image.dimensions()
    }

    /// Resize the image to the specified dimensions
    /// Uses Lanczos3 filter for high quality
    pub async fn resize(&mut self, width: u32, height: u32) {
        self.process(move |image| {
            image.resize(width, height, image::imageops::FilterType::Lanczos3)
        })
        .await;
    }

    /// Resize the image to fit within the specified dimensions while maintaining aspect ratio
    pub async fn resize_to_fit(&mut self, max_width: u32, max_height: u32) {
        self.process(move |image| {
            image.resize(max_width, max_height, image::imageops::FilterType::Lanczos3)
        })
        .await;
    }

    /// Resize the image to fill the specified dimensions while maintaining aspect ratio
    pub async fn resize_to_fill(&mut self, width: u32, height: u32) {
        self.process(move |image| {
            image.resize_to_fill(width, height, image::imageops::FilterType::Lanczos3)
        })
        .await;
    }

    /// Resize the image exactly to the specified dimensions (may distort aspect ratio)
    pub async fn resize_exact(&mut self, width: u32, height: u32) {
        self.process(move |image| {
            image.resize_exact(width, height, image::imageops::FilterType::Lanczos3)
        })
        .await;
    }

    /// Flip the image horizontally
    pub async fn flip_horizontal(&mut self) {
        self.process(|image| image.fliph()).await;
    }

    /// Flip the image vertically
    pub async fn flip_vertical(&mut self) {
        self.process(|image| image.flipv()).await;
    }

    /// Crop the image to the specified rectangle
    /// Returns true if successful, false if the rectangle is out of bounds
    pub async fn crop(&mut self, x: u32, y: u32, width: u32, height: u32) -> bool {
        let img_width = self.width();
        let img_height = self.height();

        if x + width <= img_width && y + height <= img_height {
            self.process(move |image| image.crop_imm(x, y, width, height))
                .await;
            true
        } else {
            false
        }
    }

    /// Blur the image with the specified sigma value
    pub async fn blur(&mut self, sigma: f32) {
        self.process(move |image| image.blur(sigma)).await;
    }

    /// Adjust the brightness of the image
    /// value: -100 to 100 (negative for darker, positive for brighter)
    pub async fn brighten(&mut self, value: i32) {
        self.process(move |image| image.brighten(value)).await;
    }

    /// Adjust the contrast of the image
    /// contrast: floating point value (1.0 = no change, < 1.0 = less contrast, > 1.0 = more contrast)
    pub async fn adjust_contrast(&mut self, contrast: f32) {
        self.process(move |image| image.adjust_contrast(contrast))
            .await;
    }

    /// Convert the image to grayscale
    pub async fn grayscale(&mut self) {
        self.process(|image| image.grayscale()).await;
    }

    /// Invert the colors of the image
    pub async fn invert(&mut self) {
        self.process(|image| {
            let mut img = (*image).clone();
            img.invert();
            img
        })
        .await;
    }

    /// Apply an unsharpen mask to the image
    pub async fn unsharpen(&mut self, sigma: f32, threshold: i32) {
        self.process(move |image| image.unsharpen(sigma, threshold))
            .await;
    }

    /// Get the color of a specific pixel
    /// Returns None if the coordinates are out of bounds
    #[must_use]
    #[allow(clippy::many_single_char_names)]
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<WithOpacity<Srgb>> {
        if x < self.width() && y < self.height() {
            let [r, g, b, a] = self.image.get_pixel(x, y).0;
            Some(Srgb::new_u8(r, g, b).with_opacity(f32::from(a) / 255.0))
        } else {
            None
        }
    }

    /// Create a thumbnail of the image with the specified maximum dimension
    pub async fn thumbnail(&mut self, max_size: u32) {
        self.process(move |image| image.thumbnail(max_size, max_size))
            .await;
    }

    /// Rotate the image 90 degrees clockwise
    pub async fn rotate_90(&mut self) {
        self.process(|image| image.rotate90()).await;
    }

    /// Rotate the image 180 degrees
    pub async fn rotate_180(&mut self) {
        self.process(|image| image.rotate180()).await;
    }

    /// Rotate the image 270 degrees clockwise (90 degrees counter-clockwise)
    pub async fn rotate_270(&mut self) {
        self.process(|image| image.rotate270()).await;
    }

    /// Apply a Gaussian blur with the specified sigma
    pub async fn gaussian_blur(&mut self, sigma: f32) {
        self.process(move |image| {
            DynamicImage::ImageRgba8(image::imageops::blur(&image.to_rgba8(), sigma))
        })
        .await;
    }

    /// Hue rotate the image by the specified degrees
    pub async fn huerotate(&mut self, degrees: i32) {
        self.process(move |image| image.huerotate(degrees)).await;
    }

    /// Get the MIME type of the image
    #[must_use]
    pub const fn mime(&self) -> &Mime {
        &self.mime
    }

    /// Write the image to a file with the specified format
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub async fn write(&self, format: Mime, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref().to_owned();
        let data = self.encode(format).await;
        unblock(move || std::fs::write(path, data)).await
    }

    /// Generate a base64-encoded data URL for the image encoded as PNG
    ///
    /// For large images (>10KB), encoding is done on a background thread to prevent UI blocking.
    #[must_use]
    pub async fn url(&self) -> String {
        let data = self.encode_png().await;
        // If the data is too large, encoded as base64 may block the UI thread, so do it in a background thread
        if data.len() > 10 * 1024 {
            // Move data to the background thread, and it will also be released on that thread, preventing UI blocking
            unblock(move || {
                let mut base64 = String::from("data:image/png;base64,");
                BASE64_STANDARD.encode_string(data, &mut base64);
                base64
            })
            .await
        } else {
            let data = BASE64_STANDARD.encode(data);
            format!("data:image/png;base64,{data}")
        }
    }
}
