//! Image loading and handling for Canvas.
//!
//! This module provides image loading from various sources (raw pixels, PNG, JPEG)
//! for use with the Canvas drawing API.

use waterui_core::layout::Size;

// Internal imports for rendering
use vello::peniko;

/// An image that can be drawn on the canvas.
///
/// Images can be created from raw RGBA pixels or decoded from PNG/JPEG bytes.
///
/// # Example
///
/// ```ignore
/// // Load from PNG bytes
/// let image = CanvasImage::from_bytes(png_data)?;
///
/// // Draw at position
/// ctx.draw_image(&image, Point::new(10.0, 10.0));
///
/// // Draw scaled
/// ctx.draw_image_scaled(&image, Rect::new(Point::ZERO, Size::new(200.0, 150.0)));
/// ```
pub struct CanvasImage {
    /// Internal peniko image (not exposed to users)
    image: peniko::ImageData,
    width: u32,
    height: u32,
}

impl CanvasImage {
    /// Creates an image from raw RGBA pixels.
    ///
    /// # Arguments
    /// * `width` - Width of the image in pixels
    /// * `height` - Height of the image in pixels
    /// * `pixels` - RGBA pixel data (4 bytes per pixel, must be width * height * 4 bytes)
    ///
    /// # Errors
    /// Returns an error if the pixel data length doesn't match width * height * 4.
    pub fn from_rgba_pixels(width: u32, height: u32, pixels: &[u8]) -> Result<Self, ImageError> {
        let expected_len = (width * height * 4) as usize;
        if pixels.len() != expected_len {
            return Err(ImageError::InvalidPixelData {
                expected: expected_len,
                got: pixels.len(),
            });
        }

        // Create peniko image from RGBA data
        let image = peniko::ImageData {
            data: peniko::Blob::from(pixels.to_vec()),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width,
            height,
        };

        Ok(Self {
            image,
            width,
            height,
        })
    }

    /// Creates an image by decoding PNG or JPEG bytes.
    ///
    /// # Arguments
    /// * `bytes` - PNG or JPEG image data
    ///
    /// # Errors
    /// Returns an error if the image format is unsupported or decoding fails.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ImageError> {
        // Decode image using the image crate
        let img = image::load_from_memory(bytes).map_err(ImageError::DecodeError)?;

        // Convert to RGBA8
        let rgba = img.to_rgba8();
        let width = rgba.width();
        let height = rgba.height();
        let pixels = rgba.into_raw();

        // Create peniko image
        let image = peniko::ImageData {
            data: peniko::Blob::from(pixels),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width,
            height,
        };

        Ok(Self {
            image,
            width,
            height,
        })
    }

    /// Returns the width of the image in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the height of the image in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the size of the image.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub const fn size(&self) -> Size {
        Size::new(self.width as f32, self.height as f32)
    }

    /// Returns a reference to the internal peniko `ImageData`.
    ///
    /// This is used internally by the canvas renderer.
    #[must_use]
    pub(crate) const fn inner(&self) -> &peniko::ImageData {
        &self.image
    }
}

impl core::fmt::Debug for CanvasImage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CanvasImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Errors that can occur when loading or creating images.
#[derive(Debug)]
pub enum ImageError {
    /// The pixel data length doesn't match the expected size.
    InvalidPixelData {
        /// The expected length of the pixel data in bytes.
        expected: usize,
        /// The actual length of the pixel data in bytes.
        got: usize,
    },
    /// Failed to decode image from bytes.
    DecodeError(image::ImageError),
}

impl core::fmt::Display for ImageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidPixelData { expected, got } => {
                write!(
                    f,
                    "Invalid pixel data: expected {expected} bytes, got {got}"
                )
            }
            Self::DecodeError(err) => write!(f, "Failed to decode image: {err}"),
        }
    }
}

impl std::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPixelData { .. } => None,
            Self::DecodeError(err) => Some(err),
        }
    }
}
