//! Cross-platform RGBA8 PNG image representation and blank-frame detection.

use std::fs;
use std::path::Path;

use image::{ImageReader, RgbaImage as RawRgbaImage};
use png::{BitDepth, ColorType as PngColorType, Encoder as PngEncoder, chunk};
use serde::{Deserialize, Serialize};

use crate::error::VisualDiffError;
#[cfg(feature = "flip")]
use crate::flip::{FlipConfig, FlipResult, compare_flip, compare_flip_with};
use crate::snapshot::Snapshot;

/// ITU-T H.273 / PNG 3rd Edition `cICP` payload for SDR Display P3:
/// Display P3 primaries (`12`), sRGB transfer (`13`), identity matrix (`0`),
/// full range (`1`).
pub const CICP_DISPLAY_P3_SDR: [u8; 4] = [12, 13, 0, 1];

/// ITU-T H.273 / PNG 3rd Edition `cICP` payload for HDR Display P3 PQ:
/// Display P3 primaries (`12`), SMPTE ST 2084 PQ transfer (`16`), identity
/// matrix (`0`), full range (`1`).
pub const CICP_DISPLAY_P3_PQ: [u8; 4] = [12, 16, 0, 1];

/// ITU-R BT.2408 SDR reference white luminance in nits for HDR PQ encoding:
/// linear `1.0` maps to `203` nits.
pub const HDR_REFERENCE_WHITE_NITS: f32 = 203.0;

/// SMPTE ST 2084 peak absolute luminance in nits.
const PQ_PEAK_NITS: f64 = 10_000.0;

/// Validates that `actual` equals `width * height * 4` without overflow.
pub(crate) fn validate_rgba8_len(
    width: u32,
    height: u32,
    actual: usize,
) -> Result<usize, VisualDiffError> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(VisualDiffError::InvalidImageBuffer {
            width,
            height,
            expected: usize::MAX,
            actual,
        })?;
    if actual != expected {
        return Err(VisualDiffError::InvalidImageBuffer {
            width,
            height,
            expected,
            actual,
        });
    }
    Ok(expected)
}

/// Encodes an RGBA8 Display P3 buffer into PNG bytes tagged with
/// [`CICP_DISPLAY_P3_SDR`] (`(12, 13, 0, 1)`).
///
/// # Errors
///
/// Returns [`VisualDiffError::InvalidImageBuffer`] when `rgba8.len()` does not
/// match `width * height * 4`, or [`VisualDiffError::EncodePng`] when PNG
/// encoding fails.
pub(crate) fn encode_png_display_p3_rgba8(
    width: u32,
    height: u32,
    rgba8: &[u8],
) -> Result<Vec<u8>, VisualDiffError> {
    validate_rgba8_len(width, height, rgba8.len())?;
    let mut out = Vec::new();
    let mut encoder = PngEncoder::new(&mut out, width, height);
    encoder.set_color(PngColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_chunk(chunk::cICP, &CICP_DISPLAY_P3_SDR)?;
    writer.write_image_data(rgba8)?;
    writer.finish()?;
    Ok(out)
}

/// Encodes linear Display P3 luminance (where `1.0` = 203 nits per ITU-R
/// BT.2408) with the SMPTE ST 2084 PQ OETF into an unsigned 16-bit code word.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "PQ output is clamped to [0.0, 1.0] before scaling to 0..=65535"
)]
pub(crate) fn linear_display_p3_to_pq_u16(linear: f32) -> u16 {
    // Exact SMPTE ST 2084 / ITU-R BT.2100 rational constants.
    const M1: f64 = 2610.0 / 16384.0;
    const M2: f64 = (2523.0 / 4096.0) * 128.0;
    const C1: f64 = 3424.0 / 4096.0;
    const C2: f64 = (2413.0 / 4096.0) * 32.0;
    const C3: f64 = (2392.0 / 4096.0) * 32.0;

    if !linear.is_finite() || linear <= 0.0 {
        return 0;
    }
    let y =
        (f64::from(linear) * f64::from(HDR_REFERENCE_WHITE_NITS) / PQ_PEAK_NITS).clamp(0.0, 1.0);
    let ym1 = y.powf(M1);
    let pq = (C2.mul_add(ym1, C1) / C3.mul_add(ym1, 1.0)).powf(M2);
    (pq.clamp(0.0, 1.0) * f64::from(u16::MAX)).round() as u16
}

/// Encodes an extended linear Display P3 `f32` RGBA buffer into 16-bit
/// big-endian RGBA PNG bytes tagged with [`CICP_DISPLAY_P3_PQ`]
/// (`(12, 16, 0, 1)`).
///
/// # Errors
///
/// Returns [`VisualDiffError::InvalidImageBuffer`] when `rgba_f32.len()` does
/// not match `width * height * 4`, or [`VisualDiffError::EncodePng`] when PNG
/// encoding fails.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "alpha is clamped to [0.0, 1.0] before scaling to 0..=65535"
)]
pub(crate) fn encode_png_display_p3_pq_rgba16(
    width: u32,
    height: u32,
    rgba_f32: &[f32],
) -> Result<Vec<u8>, VisualDiffError> {
    validate_rgba8_len(width, height, rgba_f32.len())?;
    let mut bytes = Vec::with_capacity(rgba_f32.len() * 2);
    for px in rgba_f32.as_chunks::<4>().0 {
        let r = linear_display_p3_to_pq_u16(px[0]);
        let g = linear_display_p3_to_pq_u16(px[1]);
        let b = linear_display_p3_to_pq_u16(px[2]);
        let a_norm = if px[3].is_finite() {
            f64::from(px[3]).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let a = (a_norm * f64::from(u16::MAX)).round() as u16;
        bytes.extend_from_slice(&r.to_be_bytes());
        bytes.extend_from_slice(&g.to_be_bytes());
        bytes.extend_from_slice(&b.to_be_bytes());
        bytes.extend_from_slice(&a.to_be_bytes());
    }
    let mut out = Vec::new();
    let mut encoder = PngEncoder::new(&mut out, width, height);
    encoder.set_color(PngColorType::Rgba);
    encoder.set_depth(BitDepth::Sixteen);
    let mut writer = encoder.write_header()?;
    writer.write_chunk(chunk::cICP, &CICP_DISPLAY_P3_PQ)?;
    writer.write_image_data(&bytes)?;
    writer.finish()?;
    Ok(out)
}

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
    /// Returns [`VisualDiffError::InvalidImageBuffer`] when `rgba8.len()` does
    /// not equal `width * height * 4`.
    pub fn new(width: u32, height: u32, rgba8: Vec<u8>) -> Result<Self, VisualDiffError> {
        validate_rgba8_len(width, height, rgba8.len())?;
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
    /// Returns [`VisualDiffError::Io`] or [`VisualDiffError::Image`] if the
    /// file cannot be read or decoded as an image.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, VisualDiffError> {
        let path = path.as_ref();
        let reader = ImageReader::open(path).map_err(|source| VisualDiffError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let decoded = reader
            .decode()
            .map_err(|source| VisualDiffError::Image {
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

    /// Writes this Display P3 RGBA8 image to `path` as a PNG file tagged with
    /// the Display P3 `cICP` chunk (`(12, 13, 0, 1)`), creating parent
    /// directories as needed.
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::Io`] or [`VisualDiffError::EncodePng`] if the
    /// directory cannot be created or PNG encoding fails.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), VisualDiffError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| VisualDiffError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let encoded = encode_png_display_p3_rgba8(self.width, self.height, &self.rgba8)?;
        fs::write(path, encoded).map_err(|source| VisualDiffError::Io {
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

    /// Consumes this image and returns `(width, height, rgba8)`.
    #[must_use]
    pub fn into_parts(self) -> (u32, u32, Vec<u8>) {
        (self.width, self.height, self.rgba8)
    }

    /// Analyzes the frame to detect blank (empty, fully transparent, or
    /// uniform single-color) captures.
    #[must_use]
    pub fn analyze_blank(&self) -> BlankFrameAnalysis {
        analyze_blank_frame(self)
    }

    /// Compares this reference image against `candidate` using NVIDIA LDR-FLIP
    /// with [`FlipConfig::default`].
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::DimensionMismatch`] when the images differ in
    /// size or [`VisualDiffError::EmptyImage`] when either dimension is zero.
    #[cfg(feature = "flip")]
    pub fn compare_flip(&self, candidate: &Self) -> Result<FlipResult, VisualDiffError> {
        compare_flip(self, candidate)
    }

    /// Compares this reference image against `candidate` using NVIDIA LDR-FLIP
    /// with an explicit [`FlipConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::InvalidConfig`] for invalid parameters,
    /// [`VisualDiffError::DimensionMismatch`] when the images differ in size, or
    /// [`VisualDiffError::EmptyImage`] when either dimension is zero.
    #[cfg(feature = "flip")]
    pub fn compare_flip_with(
        &self,
        candidate: &Self,
        config: FlipConfig,
    ) -> Result<FlipResult, VisualDiffError> {
        compare_flip_with(self, candidate, config)
    }
}

impl From<RgbaImage> for Snapshot {
    fn from(image: RgbaImage) -> Self {
        Self::new(image.width, image.height, image.rgba8)
    }
}

impl TryFrom<Snapshot> for RgbaImage {
    type Error = VisualDiffError;

    fn try_from(snapshot: Snapshot) -> Result<Self, Self::Error> {
        let (width, height, rgba8) = snapshot.into_parts();
        Self::new(width, height, rgba8)
    }
}

impl TryFrom<&Snapshot> for RgbaImage {
    type Error = VisualDiffError;

    fn try_from(snapshot: &Snapshot) -> Result<Self, Self::Error> {
        Self::new(
            snapshot.width(),
            snapshot.height(),
            snapshot.rgba8().to_vec(),
        )
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

/// Analyzes raw RGBA8 pixel data for empty dimensions, full transparency, or a
/// uniform single-color fill.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel counts are far below 2^53"
)]
pub(crate) fn analyze_blank_rgba8(width: u32, height: u32, rgba8: &[u8]) -> BlankFrameAnalysis {
    if width == 0 || height == 0 || rgba8.len() < 4 {
        return BlankFrameAnalysis {
            reason: Some(BlankFrameReason::EmptyDimensions),
            non_transparent_ratio: 0.0,
            max_channel_spread: 0,
        };
    }

    let first = [rgba8[0], rgba8[1], rgba8[2], rgba8[3]];
    let mut min_ch = first;
    let mut max_ch = first;
    let mut non_transparent: usize = 0;
    let mut total_pixels: usize = 0;

    for px in rgba8.as_chunks::<4>().0 {
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

/// Analyzes an [`RgbaImage`] for empty dimensions, full transparency, or a
/// uniform single-color fill.
#[must_use]
pub fn analyze_blank_frame(image: &RgbaImage) -> BlankFrameAnalysis {
    analyze_blank_rgba8(image.width, image.height, &image.rgba8)
}
