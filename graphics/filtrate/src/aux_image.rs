//! Auxiliary images: the second inputs of blends, masks, maps, LUTs and
//! transitions.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;

use filtrate_core::{AuxData, AuxFormat, AuxImage};
use waterui_contract::contract_violation;

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
    /// `rgba8` must contain exactly `width * height * 4` bytes; otherwise the
    /// contract is broken and the texels are truncated or zero-padded to that
    /// length.
    #[must_use]
    pub fn from_rgba8(width: u32, height: u32, rgba8: Vec<u8>) -> Self {
        Self::from_data(width, height, AuxFormat::Rgba8, rgba8)
    }

    /// Creates an image from f16 texels, row-major.
    ///
    /// `rgba16f` must contain exactly `width * height * 4` texels; otherwise
    /// the contract is broken and the texels are truncated or zero-padded to
    /// that length.
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
    /// `rgba32f` must contain exactly `width * height * 4` texels; otherwise
    /// the contract is broken and the texels are truncated or zero-padded to
    /// that length.
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
    /// `texels` must be exactly `width * height * format.texel_size()` bytes;
    /// otherwise the contract is broken and they are truncated or zero-padded
    /// to that length.
    #[must_use]
    fn from_data(width: u32, height: u32, format: AuxFormat, mut texels: Vec<u8>) -> Self {
        let expected_len = width as usize * height as usize * format.texel_size();
        if texels.len() != expected_len {
            contract_violation!(
                "FilterImage: expected {expected_len} bytes for a {width}x{height} {format:?} image, got {}; \
                 the texels are truncated or zero-padded to fit",
                texels.len()
            );
            texels.resize(expected_len, 0);
        }
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
    /// `size` must be in `2..=65535` and `image` must be `size * size` by
    /// `size` texels; otherwise the contract is broken. The size is clamped
    /// into that range and the strip is sampled as given, with lookups
    /// beyond it clamped to its edge.
    #[must_use]
    pub fn new(image: FilterImage, size: u32) -> Self {
        let clamped = size.clamp(2, Self::MAX_SIZE);
        if clamped != size {
            contract_violation!(
                "LutImage::new: lut size must be in 2..={}, got {size}; the size is clamped to {clamped}",
                Self::MAX_SIZE
            );
        }
        // `MAX_SIZE` squared fits u32.
        let expected_width = clamped * clamped;
        if image.width() != expected_width || image.height() != clamped {
            contract_violation!(
                "LutImage::new: expected a {expected_width}x{clamped} strip for size {clamped}, got {}x{}; \
                 the strip is sampled as given, with lookups beyond it clamped to its edge",
                image.width(),
                image.height()
            );
        }
        Self {
            image,
            size: clamped,
        }
    }

    /// The largest cube size: its strip is `size * size` texels wide, which
    /// must fit a texture dimension.
    const MAX_SIZE: u32 = u16::MAX as u32;

    /// Creates a LUT from RGBA8 strip bytes.
    ///
    /// The bytes must form a `size * size` by `size` strip and `size` must be
    /// in `2..=65535`; see [`new`](Self::new) for a broken contract.
    #[must_use]
    pub fn from_rgba8(size: u32, rgba8: Vec<u8>) -> Self {
        let clamped = size.clamp(2, Self::MAX_SIZE);
        Self::new(
            FilterImage::from_rgba8(clamped * clamped, clamped, rgba8),
            size,
        )
    }

    /// Decodes a LUT strip from encoded image bytes.
    ///
    /// # Errors
    ///
    /// Returns the decode error when the bytes cannot be parsed as an image.
    ///
    /// The decoded image must be a `size * size` by `size` strip and `size`
    /// must be in `2..=65535`; see [`new`](Self::new) for a broken contract.
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
    #[cfg_attr(debug_assertions, should_panic(expected = "expected 16 bytes"))]
    fn filter_image_pads_or_truncates_invalid_buffer_len() {
        let short = FilterImage::from_rgba8(2, 2, vec![7; 3]);
        let data = short.data().expect("CPU image");
        assert_eq!(data.bytes, [7, 7, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let long = FilterImage::from_rgba8(1, 1, vec![9; 6]);
        assert_eq!(long.data().expect("CPU image").bytes, [9, 9, 9, 9]);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "expected a 16x4 strip"))]
    fn lut_image_samples_an_invalid_strip_as_given() {
        let bad_image = FilterImage::from_rgba8(16, 15, vec![0; 16 * 15 * 4]);
        let lut = LutImage::new(bad_image, 4);
        assert_eq!((lut.size(), lut.image().height()), (4, 15));
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "lut size must be in 2..=65535")
    )]
    fn lut_image_clamps_an_out_of_range_size() {
        let lut = LutImage::new(FilterImage::from_rgba8(4, 2, vec![0; 32]), 1);
        assert_eq!(lut.size(), 2);
    }
}
