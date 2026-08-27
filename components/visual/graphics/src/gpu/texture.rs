//! Row geometry for texture uploads and readbacks.
//!
//! `wgpu` requires every buffer row of a texture copy to be a multiple of
//! [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`], so any code moving pixels between CPU
//! memory and a texture has to derive a padded row stride, size the staging
//! buffer from it, and put the padding back on or take it off again. That
//! arithmetic lives here once instead of in every renderer that uploads an
//! image or reads a frame back.

use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;

/// Row stride arithmetic for one tightly packed CPU image and its texture.
///
/// Construct it from the image's dimensions and its bytes-per-pixel, then use
/// [`padded_bytes_per_row`](Self::padded_bytes_per_row) for the copy layout and
/// [`pad_rows`](Self::pad_rows) / [`unpad_rows`](Self::unpad_rows) to convert
/// between the packed and padded representations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureRowLayout {
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
}

impl TextureRowLayout {
    /// Describes an image of `width` × `height` pixels of `bytes_per_pixel` each.
    #[must_use]
    pub const fn new(width: u32, height: u32, bytes_per_pixel: u32) -> Self {
        Self {
            width,
            height,
            bytes_per_pixel,
        }
    }

    /// Describes an 8-bit RGBA image, the format every readback path uses.
    #[must_use]
    pub const fn rgba8(width: u32, height: u32) -> Self {
        Self::new(width, height, 4)
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Row stride of the tightly packed CPU image.
    #[must_use]
    pub const fn unpadded_bytes_per_row(&self) -> u32 {
        self.width * self.bytes_per_pixel
    }

    /// Row stride `wgpu` requires for a buffer taking part in a texture copy.
    #[must_use]
    pub const fn padded_bytes_per_row(&self) -> u32 {
        const ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        self.unpadded_bytes_per_row().div_ceil(ALIGNMENT) * ALIGNMENT
    }

    /// Size of a staging buffer holding every padded row.
    #[must_use]
    pub const fn padded_buffer_size(&self) -> u64 {
        self.padded_bytes_per_row() as u64 * self.height as u64
    }

    /// Copy layout describing the padded rows, for `copy_texture_to_buffer`,
    /// `copy_buffer_to_texture`, and `write_texture`.
    #[must_use]
    pub const fn buffer_layout(&self) -> wgpu::TexelCopyBufferLayout {
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(self.padded_bytes_per_row()),
            rows_per_image: Some(self.height),
        }
    }

    /// Extent covering the whole image, for the copy's `copy_size`.
    #[must_use]
    pub const fn extent(&self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        }
    }

    /// Rewrites tightly packed rows onto the padded stride.
    ///
    /// Borrows `pixels` unchanged when the packed stride is already aligned,
    /// which is the common case for wide images.
    ///
    /// # Panics
    ///
    /// Panics when `pixels` is not exactly one tightly packed image.
    #[must_use]
    pub fn pad_rows<'a>(&self, pixels: &'a [u8]) -> Cow<'a, [u8]> {
        let unpadded = self.unpadded_bytes_per_row() as usize;
        let padded = self.padded_bytes_per_row() as usize;
        let height = self.height as usize;
        assert_eq!(
            pixels.len(),
            unpadded * height,
            "pixel buffer must hold exactly {height} tightly packed rows of {unpadded} bytes"
        );
        if padded == unpadded {
            return Cow::Borrowed(pixels);
        }

        let mut out = vec![0u8; padded * height];
        for row in 0..height {
            let src = row * unpadded;
            let dst = row * padded;
            out[dst..dst + unpadded].copy_from_slice(&pixels[src..src + unpadded]);
        }
        Cow::Owned(out)
    }

    /// Strips the row padding a texture readback wrote into `padded`.
    ///
    /// # Panics
    ///
    /// Panics when `padded` is shorter than the padded image it should hold.
    #[must_use]
    pub fn unpad_rows(&self, padded: &[u8]) -> Vec<u8> {
        let unpadded_stride = self.unpadded_bytes_per_row() as usize;
        let padded_stride = self.padded_bytes_per_row() as usize;
        let height = self.height as usize;
        assert!(
            padded.len() >= padded_stride * height,
            "readback buffer holds {} bytes, short of the {} a padded image needs",
            padded.len(),
            padded_stride * height
        );

        let mut out = Vec::with_capacity(unpadded_stride * height);
        for row in 0..height {
            let start = row * padded_stride;
            out.extend_from_slice(&padded[start..start + unpadded_stride]);
        }
        out
    }
}

/// Uploads one tightly packed CPU image into `texture`, padding rows as `wgpu`
/// requires.
///
/// # Panics
///
/// Panics when `pixels` is not exactly one tightly packed image of `layout`.
pub fn upload_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    pixels: &[u8],
    layout: TextureRowLayout,
) {
    queue.write_texture(
        texture.as_image_copy(),
        &layout.pad_rows(pixels),
        layout.buffer_layout(),
        layout.extent(),
    );
}
