//! Cross-platform RGBA8 PNG image representation and blank-frame detection.

use std::fs;
use std::path::Path;

use image::{ColorType, ImageReader, RgbaImage as RawRgbaImage};
use serde::{Deserialize, Serialize};

use crate::error::ConformanceError;

/// An owned RGBA8 image in row-major order (`4 * width * height` bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    rgba8: Vec<u8>,
}

impl RgbaImage {
    /// Creates an RGBA8 image from raw row-major pixels.
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError::InvalidImageBuffer`] when `rgba8.len()` does
    /// not equal `width * height * 4`.
    pub fn new(width: u32, height: u32, rgba8: Vec<u8>) -> Result<Self, ConformanceError> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(ConformanceError::InvalidImageBuffer {
                width,
                height,
                expected: usize::MAX,
                actual: rgba8.len(),
            })?;
        if rgba8.len() != expected {
            return Err(ConformanceError::InvalidImageBuffer {
                width,
                height,
                expected,
                actual: rgba8.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba8,
        })
    }

    /// Loads a PNG file into an RGBA8 image using pure-Rust decoding.
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError::Io`] or [`ConformanceError::Image`] if the
    /// file cannot be read or decoded as an image.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, ConformanceError> {
        let path = path.as_ref();
        let reader = ImageReader::open(path).map_err(|source| ConformanceError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let decoded = reader
            .decode()
            .map_err(|source| ConformanceError::Image {
                path: path.to_path_buf(),
                source,
            })?
            .into_rgba8();
        let (width, height) = decoded.dimensions();
        Ok(Self {
            width,
            height,
            rgba8: decoded.into_raw(),
        })
    }

    /// Writes this RGBA8 image to `path` as a PNG file, creating parent
    /// directories as needed.
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError::Io`] or [`ConformanceError::Image`] if the
    /// directory cannot be created or PNG encoding fails.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), ConformanceError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| ConformanceError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        image::save_buffer_with_format(
            path,
            &self.rgba8,
            self.width,
            self.height,
            ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|source| ConformanceError::Image {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Converts this image into an [`image::RgbaImage`].
    ///
    /// # Panics
    ///
    /// Never panics because [`RgbaImage::new`] upholds the buffer-length
    /// invariant.
    #[must_use]
    pub fn to_image_buffer(&self) -> RawRgbaImage {
        RawRgbaImage::from_raw(self.width, self.height, self.rgba8.clone())
            .expect("RgbaImage invariant guarantees buffer length matches dimensions")
    }

    /// Image width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Raw RGBA8 pixels in row-major order.
    #[must_use]
    pub const fn rgba8(&self) -> &[u8] {
        self.rgba8.as_slice()
    }

    /// Analyzes the frame to detect blank (empty, fully transparent, or
    /// uniform single-color) captures.
    #[must_use]
    pub fn analyze_blank(&self) -> BlankFrameAnalysis {
        analyze_blank_frame(self)
    }
}

#[cfg(feature = "testing")]
impl From<waterui_testing::Snapshot> for RgbaImage {
    fn from(snapshot: waterui_testing::Snapshot) -> Self {
        let width = snapshot.width();
        let height = snapshot.height();
        let rgba8 = snapshot.rgba8().to_vec();
        Self::new(width, height, rgba8)
            .expect("waterui_testing::Snapshot buffer always matches dimensions")
    }
}

#[cfg(feature = "capture")]
impl From<waterui_capture::RenderResult> for RgbaImage {
    fn from(result: waterui_capture::RenderResult) -> Self {
        let (rgba8, width, height) = result.into_parts();
        Self::new(width, height, rgba8)
            .expect("waterui_capture::RenderResult buffer always matches dimensions")
    }
}

/// Why a captured frame was classified as blank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum BlankFrameReason {
    /// Width or height is zero.
    EmptyDimensions,
    /// Every pixel has alpha equal to `0`.
    FullyTransparent,
    /// Every pixel has the exact same RGBA8 value (for example, an uncleared
    /// black frame or a plain background with no rendered content).
    UniformColor {
        /// The uniform RGBA8 pixel value observed across the entire frame.
        rgba: [u8; 4],
    },
}

/// Result of analyzing a rendered frame for blankness.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BlankFrameAnalysis {
    /// Reason the frame is blank, or `None` when real visual variation exists.
    pub reason: Option<BlankFrameReason>,
    /// Fraction of pixels with non-zero alpha (`0.0..=1.0`).
    pub non_transparent_ratio: f64,
    /// Maximum `(max - min)` channel spread across R, G, B, and A (`0..=255`).
    pub max_channel_spread: u8,
}

impl BlankFrameAnalysis {
    /// Returns `true` when the frame is classified as blank.
    #[must_use]
    pub const fn is_blank(&self) -> bool {
        self.reason.is_some()
    }
}

/// Analyzes an [`RgbaImage`] for empty dimensions, full transparency, or a
/// uniform single-color fill.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel counts are far below 2^53"
)]
pub fn analyze_blank_frame(image: &RgbaImage) -> BlankFrameAnalysis {
    if image.width == 0 || image.height == 0 || image.rgba8.is_empty() {
        return BlankFrameAnalysis {
            reason: Some(BlankFrameReason::EmptyDimensions),
            non_transparent_ratio: 0.0,
            max_channel_spread: 0,
        };
    }

    let first = [
        image.rgba8[0],
        image.rgba8[1],
        image.rgba8[2],
        image.rgba8[3],
    ];
    let mut min_ch = first;
    let mut max_ch = first;
    let mut non_transparent: usize = 0;
    let mut total_pixels: usize = 0;

    for px in image.rgba8.as_chunks::<4>().0 {
        total_pixels += 1;
        if px[3] > 0 {
            non_transparent += 1;
        }
        for i in 0..4 {
            min_ch[i] = min_ch[i].min(px[i]);
            max_ch[i] = max_ch[i].max(px[i]);
        }
    }

    let max_channel_spread = (0..4).map(|i| max_ch[i] - min_ch[i]).max().unwrap_or(0);
    let non_transparent_ratio = non_transparent as f64 / total_pixels as f64;

    let reason = if non_transparent == 0 {
        Some(BlankFrameReason::FullyTransparent)
    } else if max_channel_spread == 0 {
        Some(BlankFrameReason::UniformColor { rgba: first })
    } else {
        None
    };

    BlankFrameAnalysis {
        reason,
        non_transparent_ratio,
        max_channel_spread,
    }
}
