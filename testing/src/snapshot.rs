use std::fs;
use std::io;
use std::path::Path;

use image::{ImageError, ImageResult};

use crate::error::VisualDiffError;
#[cfg(feature = "flip")]
use crate::flip::{FlipConfig, FlipResult, compare_flip_rgba8};
use crate::image::{
    BlankFrameAnalysis, RgbaImage, analyze_blank_rgba8, encode_png_display_p3_pq_rgba16,
    encode_png_display_p3_rgba8, validate_rgba8_len,
};

/// RGBA8 frame captured from a headless hydrolysis render pass.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Snapshot {
    /// Frame width in pixels.
    width: u32,
    /// Frame height in pixels.
    height: u32,
    /// Pixel data in RGBA8 row-major order.
    rgba8: Vec<u8>,
}

impl Snapshot {
    /// Creates a Snapshot from its parts.
    #[must_use]
    pub const fn new(width: u32, height: u32, rgba8: Vec<u8>) -> Self {
        Self {
            width,
            height,
            rgba8,
        }
    }

    /// Frame width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Pixel data in RGBA8 row-major order.
    #[must_use]
    pub const fn rgba8(&self) -> &[u8] {
        self.rgba8.as_slice()
    }

    /// Pixel data in RGBA8 row-major order, for editing in place.
    pub const fn rgba8_mut(&mut self) -> &mut [u8] {
        self.rgba8.as_mut_slice()
    }

    /// Consumes the snapshot and returns `(width, height, rgba8)`.
    #[must_use]
    pub fn into_parts(self) -> (u32, u32, Vec<u8>) {
        (self.width, self.height, self.rgba8)
    }
}

impl Snapshot {
    /// Loads a PNG file into a [`Snapshot`] using pure-Rust decoding.
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::Io`] or [`VisualDiffError::Image`] if the
    /// file cannot be read or decoded as a PNG image.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, VisualDiffError> {
        RgbaImage::load_png(path).map(Self::from)
    }

    /// Saves the snapshot to a PNG file tagged with the Display P3 `cICP`
    /// chunk (`(12, 13, 0, 1)`).
    ///
    /// # Errors
    ///
    /// Returns image I/O or encoding errors from the underlying PNG writer.
    ///
    /// # Panics
    ///
    /// Panics if the stored RGBA buffer length does not match the snapshot dimensions.
    pub fn save_png(&self, path: impl AsRef<Path>) -> ImageResult<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(ImageError::IoError)?;
        }
        assert_eq!(
            self.rgba8.len(),
            self.width as usize * self.height as usize * 4,
            "Snapshot::save_png: rgba buffer shape must match dimensions"
        );
        let encoded =
            encode_png_display_p3_rgba8(self.width, self.height, &self.rgba8).map_err(|error| {
                ImageError::IoError(io::Error::other(format!(
                    "failed to encode Display P3 PNG: {error}"
                )))
            })?;
        fs::write(path, encoded).map_err(ImageError::IoError)
    }

    /// Analyzes the snapshot to detect blank (empty, fully transparent, or
    /// uniform single-color) captures.
    #[must_use]
    pub fn analyze_blank(&self) -> BlankFrameAnalysis {
        analyze_blank_rgba8(self.width, self.height, &self.rgba8)
    }

    /// Compares this reference snapshot against `candidate` using NVIDIA
    /// LDR-FLIP with [`FlipConfig::default`].
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::InvalidImageBuffer`] if either snapshot's
    /// buffer length does not match its dimensions,
    /// [`VisualDiffError::DimensionMismatch`] when the snapshots differ in
    /// size, or [`VisualDiffError::EmptyImage`] when either dimension is zero.
    #[cfg(feature = "flip")]
    pub fn compare_flip(&self, candidate: &Self) -> Result<FlipResult, VisualDiffError> {
        self.compare_flip_with(candidate, FlipConfig::default())
    }

    /// Compares this reference snapshot against `candidate` using NVIDIA
    /// LDR-FLIP with an explicit [`FlipConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::InvalidConfig`] for invalid parameters,
    /// [`VisualDiffError::InvalidImageBuffer`] if either snapshot's buffer
    /// length does not match its dimensions,
    /// [`VisualDiffError::DimensionMismatch`] when the snapshots differ in
    /// size, or [`VisualDiffError::EmptyImage`] when either dimension is zero.
    #[cfg(feature = "flip")]
    pub fn compare_flip_with(
        &self,
        candidate: &Self,
        config: FlipConfig,
    ) -> Result<FlipResult, VisualDiffError> {
        compare_flip_rgba8(
            self.width,
            self.height,
            &self.rgba8,
            candidate.width,
            candidate.height,
            &candidate.rgba8,
            config,
        )
    }
}

/// Extended linear Display P3 `f32` RGBA frame captured from an HDR offscreen
/// render pass (`Rgba16Float` / `OutputColor::LinearDisplayP3`).
///
/// RGB channels hold unclamped linear light in Display P3 primaries where `1.0`
/// corresponds to SDR reference white (`203` nits under ITU-R BT.2408) and
/// values above `1.0` preserve HDR / EDR highlights.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct HdrSnapshot {
    width: u32,
    height: u32,
    rgba_f32: Vec<f32>,
    headroom: f32,
}

impl HdrSnapshot {
    /// Creates an [`HdrSnapshot`] from row-major linear Display P3 `f32` RGBA
    /// pixels (`width * height * 4` floats).
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::InvalidImageBuffer`] when `rgba_f32.len()`
    /// does not equal `width * height * 4`.
    pub fn new(width: u32, height: u32, rgba_f32: Vec<f32>) -> Result<Self, VisualDiffError> {
        Self::with_headroom(width, height, rgba_f32, 10_000.0 / 203.0)
    }

    /// Creates an [`HdrSnapshot`] recording the capture target's display
    /// headroom ceiling (`>= 1.0`).
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::InvalidImageBuffer`] when `rgba_f32.len()`
    /// does not equal `width * height * 4`.
    pub fn with_headroom(
        width: u32,
        height: u32,
        rgba_f32: Vec<f32>,
        headroom: f32,
    ) -> Result<Self, VisualDiffError> {
        validate_rgba8_len(width, height, rgba_f32.len())?;
        Ok(Self {
            width,
            height,
            rgba_f32,
            headroom: if headroom.is_finite() && headroom > 1.0 {
                headroom
            } else {
                1.0
            },
        })
    }

    /// Frame width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Row-major extended linear Display P3 `f32` RGBA channels
    /// (`width * height * 4` floats).
    #[must_use]
    pub const fn rgba_f32(&self) -> &[f32] {
        self.rgba_f32.as_slice()
    }

    /// Configured capture target headroom ceiling (`>= 1.0`).
    #[must_use]
    pub const fn headroom(&self) -> f32 {
        self.headroom
    }

    /// Consumes the snapshot and returns `(width, height, rgba_f32)`.
    #[must_use]
    pub fn into_parts(self) -> (u32, u32, Vec<f32>) {
        (self.width, self.height, self.rgba_f32)
    }

    /// Returns the `[R, G, B, A]` linear Display P3 pixel at `(x, y)`.
    ///
    /// # Panics
    ///
    /// Panics if `x >= width` or `y >= height`.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 4] {
        assert!(
            x < self.width && y < self.height,
            "pixel ({x}, {y}) out of bounds for {}x{}",
            self.width,
            self.height
        );
        let offset = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        [
            self.rgba_f32[offset],
            self.rgba_f32[offset + 1],
            self.rgba_f32[offset + 2],
            self.rgba_f32[offset + 3],
        ]
    }

    /// Maximum finite RGB channel value across the entire frame (`0.0` if
    /// empty).
    #[must_use]
    pub fn max_channel(&self) -> f32 {
        self.rgba_f32
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| [px[0], px[1], px[2]])
            .filter(|v| v.is_finite())
            .fold(0.0_f32, f32::max)
    }

    /// Peak EDR headroom observed in this frame relative to SDR white
    /// (`>= 1.0`).
    #[must_use]
    pub fn max_edr_headroom(&self) -> f32 {
        self.max_channel().max(1.0)
    }

    /// Encodes this HDR snapshot as a 16-bit big-endian RGBA PNG tagged with
    /// Display P3 SMPTE ST 2084 PQ `cICP` `(12, 16, 0, 1)` at ITU-R BT.2408
    /// `203`-nit reference white.
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::InvalidImageBuffer`] or
    /// [`VisualDiffError::EncodePng`] if PNG encoding fails.
    pub fn to_hdr_png(&self) -> Result<Vec<u8>, VisualDiffError> {
        encode_png_display_p3_pq_rgba16(self.width, self.height, &self.rgba_f32)
    }

    /// Writes this HDR snapshot to `path` as a 16-bit Display P3 PQ PNG
    /// (`cICP (12, 16, 0, 1)`), creating parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::Io`] or [`VisualDiffError::EncodePng`] if
    /// writing or PNG encoding fails.
    pub fn save_hdr_png(&self, path: impl AsRef<Path>) -> Result<(), VisualDiffError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| VisualDiffError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let encoded = self.to_hdr_png()?;
        fs::write(path, encoded).map_err(|source| VisualDiffError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Converts this extended linear Display P3 snapshot into an 8-bit SDR
    /// Display P3 [`Snapshot`] by clamping/tone-mapping highlights to SDR white
    /// and applying the sRGB transfer function (Display P3's OETF).
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "sRGB OETF output is clamped to [0.0, 1.0] before scaling to 0..=255"
    )]
    pub fn to_sdr_snapshot(&self) -> Snapshot {
        let mut rgba8 = Vec::with_capacity(self.rgba_f32.len());
        for px in self.rgba_f32.as_chunks::<4>().0 {
            let max_rgb = px[0].max(px[1]).max(px[2]);
            let scale = if max_rgb > 1.0 { 1.0 / max_rgb } else { 1.0 };
            for ch in [px[0] * scale, px[1] * scale, px[2] * scale] {
                let encoded = linear_to_srgb(ch.clamp(0.0, 1.0));
                rgba8.push((encoded * 255.0).round() as u8);
            }
            let alpha = if px[3].is_finite() {
                px[3].clamp(0.0, 1.0)
            } else {
                0.0
            };
            rgba8.push((alpha * 255.0).round() as u8);
        }
        Snapshot::new(self.width, self.height, rgba8)
    }
}

/// IEC 61966-2-1 sRGB OETF (also used by Display P3).
fn linear_to_srgb(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055_f32.mul_add(linear.powf(1.0 / 2.4), -0.055)
    }
}
