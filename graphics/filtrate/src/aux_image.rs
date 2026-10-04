//! Auxiliary images: the second inputs of blends, masks, maps, LUTs and
//! transitions.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;

use filtrate_core::{AuxData, AuxFormat, AuxImage};

/// A caller-provided GPU texture, bound to a stage's `aux` argument at its
/// native format.
///
/// The executor binds the texture in place — no upload, no conversion — so
/// its format must sample as `texture_2d<f32>`; setup fails otherwise. A
/// stage binds one through [`AuxSource::Texture`](filtrate_core::AuxSource::Texture),
/// which rejects CPU images, or through `AuxSource::Image`, which accepts
/// either.
///
/// Available with the `gpu` feature.
#[cfg(feature = "gpu")]
#[derive(Clone, Debug)]
pub struct TextureImage {
    texture: wgpu::Texture,
}

#[cfg(feature = "gpu")]
impl TextureImage {
    /// Wraps a texture the caller already uploaded.
    ///
    /// The texture must be 2D and float-sampleable; setup fails otherwise.
    /// Binding it needs [`wgpu::TextureUsages::TEXTURE_BINDING`].
    #[must_use]
    pub const fn new(texture: wgpu::Texture) -> Self {
        Self { texture }
    }

    /// The texture.
    #[must_use]
    pub const fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }
}

#[cfg(feature = "gpu")]
impl AuxImage for TextureImage {
    fn width(&self) -> u32 {
        self.texture.width()
    }

    fn height(&self) -> u32 {
        self.texture.height()
    }

    fn data(&self) -> Option<AuxData<'_>> {
        None
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }
}

/// An immutable image bound to a stage's `aux` argument.
///
/// CPU images are uploaded once, at their native precision: RGBA8, RGBA16F
/// or RGBA32F texels exactly as given — a float map keeps its precision
/// ([`FilterImage::from_rgba16f`], [`FilterImage::from_rgba32f`]). With the
/// `gpu` feature a texture binds in place, also at its native format
/// (`FilterImage::from_texture`).
#[derive(Clone, Debug)]
pub struct FilterImage(Source);

/// What a [`FilterImage`] holds.
#[derive(Clone, Debug)]
enum Source {
    /// CPU texels the executor uploads.
    Cpu {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// The texel format of `texels`.
        format: AuxFormat,
        /// `width * height * format.texel_size()` bytes, row-major.
        texels: Arc<[u8]>,
    },
    /// A texture bound in place.
    #[cfg(feature = "gpu")]
    Texture(TextureImage),
}

impl FilterImage {
    /// Creates an image from raw RGBA8 bytes, row-major.
    ///
    /// # Panics
    ///
    /// Panics when `rgba8` does not contain exactly `width * height * 4` bytes.
    #[must_use]
    pub fn from_rgba8(width: u32, height: u32, rgba8: Vec<u8>) -> Self {
        Self::from_data(width, height, AuxFormat::Rgba8, rgba8)
    }

    /// Creates an image from f16 texels, row-major.
    ///
    /// # Panics
    ///
    /// Panics when `rgba16f` does not contain exactly `width * height * 4`
    /// texels.
    #[must_use]
    pub fn from_rgba16f(width: u32, height: u32, rgba16f: &[half::f16]) -> Self {
        Self::from_data(
            width,
            height,
            AuxFormat::Rgba16Float,
            rgba16f
                .iter()
                .flat_map(|texel| texel.to_bits().to_le_bytes())
                .collect(),
        )
    }

    /// Creates an image from f32 texels, row-major.
    ///
    /// # Panics
    ///
    /// Panics when `rgba32f` does not contain exactly `width * height * 4`
    /// texels.
    #[must_use]
    pub fn from_rgba32f(width: u32, height: u32, rgba32f: &[f32]) -> Self {
        Self::from_data(
            width,
            height,
            AuxFormat::Rgba32Float,
            bytemuck::cast_slice(rgba32f).to_vec(),
        )
    }

    /// Wraps a texture the caller already uploaded — bound in place at its
    /// native format.
    ///
    /// Available with the `gpu` feature.
    #[cfg(feature = "gpu")]
    #[must_use]
    pub const fn from_texture(texture: TextureImage) -> Self {
        Self(Source::Texture(texture))
    }

    /// Creates an image from CPU texels.
    ///
    /// # Panics
    ///
    /// Panics when `texels` is not exactly `width * height *
    /// format.texel_size()` bytes.
    #[must_use]
    fn from_data(width: u32, height: u32, format: AuxFormat, texels: Vec<u8>) -> Self {
        let expected_len = width as usize * height as usize * format.texel_size();
        assert_eq!(
            texels.len(),
            expected_len,
            "FilterImage: expected {expected_len} bytes for a {width}x{height} {format:?} image, got {}",
            texels.len()
        );
        Self(Source::Cpu {
            width,
            height,
            format,
            texels: Arc::from(texels),
        })
    }

    /// Decodes an encoded image and converts it to RGBA8 pixels.
    ///
    /// # Errors
    ///
    /// Returns the decode error when the bytes cannot be parsed as an image.
    pub fn from_encoded(bytes: &[u8]) -> Result<Self, image::ImageError> {
        let decoded = image::load_from_memory(bytes)?;
        Ok(Self::from_dynamic_image(&decoded))
    }

    /// Converts a dynamic image into a filter image.
    #[must_use]
    pub fn from_dynamic_image(image: &image::DynamicImage) -> Self {
        let rgba = image.to_rgba8();
        Self::from_rgba8(rgba.width(), rgba.height(), rgba.into_raw())
    }
}

impl AuxImage for FilterImage {
    fn width(&self) -> u32 {
        match &self.0 {
            Source::Cpu { width, .. } => *width,
            #[cfg(feature = "gpu")]
            Source::Texture(texture) => texture.texture.width(),
        }
    }

    fn height(&self) -> u32 {
        match &self.0 {
            Source::Cpu { height, .. } => *height,
            #[cfg(feature = "gpu")]
            Source::Texture(texture) => texture.texture.height(),
        }
    }

    fn data(&self) -> Option<AuxData<'_>> {
        match &self.0 {
            Source::Cpu { format, texels, .. } => Some(AuxData {
                format: *format,
                bytes: texels.as_ref(),
            }),
            #[cfg(feature = "gpu")]
            Source::Texture(_) => None,
        }
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        match &self.0 {
            Source::Cpu { .. } => None,
            #[cfg(feature = "gpu")]
            Source::Texture(texture) => Some(texture),
        }
    }
}

/// A 3D LUT packed into the common 2D strip layout: `size` slices of
/// `size` x `size` texels side by side, blue selecting the slice.
#[derive(Clone, Debug)]
pub struct LutImage {
    image: FilterImage,
    size: u32,
}

impl LutImage {
    /// Creates a LUT from a strip image.
    ///
    /// # Panics
    ///
    /// Panics when `size < 2`, when `image` is a GPU texture, or when `image`
    /// is not `size * size` by `size` texels.
    #[must_use]
    pub fn new(image: FilterImage, size: u32) -> Self {
        assert!(
            size >= 2,
            "LutImage::new: lut size must be >= 2, got {size}"
        );
        let expected_width = size * size;
        assert_eq!(
            image.width(),
            expected_width,
            "LutImage::new: expected width {expected_width} for size {size}, got {}",
            image.width()
        );
        assert_eq!(
            image.height(),
            size,
            "LutImage::new: expected height {size} for size {size}, got {}",
            image.height()
        );
        Self { image, size }
    }

    /// Creates a LUT from RGBA8 strip bytes.
    ///
    /// # Panics
    ///
    /// Panics when the bytes do not form a `size * size` by `size` strip, or
    /// when `size < 2`.
    #[must_use]
    pub fn from_rgba8(size: u32, rgba8: Vec<u8>) -> Self {
        Self::new(FilterImage::from_rgba8(size * size, size, rgba8), size)
    }

    /// Decodes a LUT strip from encoded image bytes.
    ///
    /// # Errors
    ///
    /// Returns the decode error when the bytes cannot be parsed as an image.
    ///
    /// # Panics
    ///
    /// Panics when the decoded image is not a `size * size` by `size` strip,
    /// or when `size < 2`.
    pub fn from_encoded(size: u32, encoded: &[u8]) -> Result<Self, image::ImageError> {
        Ok(Self::new(FilterImage::from_encoded(encoded)?, size))
    }

    /// The LUT cube size.
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }

    /// The strip image.
    #[must_use]
    pub const fn image(&self) -> &FilterImage {
        &self.image
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_image_rejects_invalid_buffer_len() {
        let result = std::panic::catch_unwind(|| FilterImage::from_rgba8(2, 2, vec![0; 3]));
        assert!(result.is_err());
    }

    #[test]
    fn lut_image_rejects_invalid_dimensions() {
        let bad_image = FilterImage::from_rgba8(16, 15, vec![0; 16 * 15 * 4]);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = LutImage::new(bad_image, 4);
        }));
        assert!(result.is_err());
    }
}
