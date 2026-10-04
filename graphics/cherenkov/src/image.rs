//! Typed image storage formats and the data a commit hands to the render
//! thread for [`Image`](crate::Image) registration.

use std::sync::Arc;

use crate::error::ResourceError;

mod sealed {
    pub trait Sealed {}
}

/// An image storage format. The `F` parameter of
/// [`ImageData`] and [`Image`](crate::Image) — typed because which formats a
/// backend accepts is a static fact (the [`Uploads`](crate::Uploads)
/// capability).
pub trait Format: sealed::Sealed + 'static {
    /// The untyped tag carried across the channel.
    const FORMAT: ImageFormat;
    /// The encoded byte length of a `width` × `height` image.
    fn bytes(width: u32, height: u32) -> usize;
}

/// Uncompressed premultiplied RGBA8.
#[derive(Clone, Copy, Debug)]
pub struct Rgba8;
impl sealed::Sealed for Rgba8 {}
impl Format for Rgba8 {
    const FORMAT: ImageFormat = ImageFormat::Rgba8;
    fn bytes(width: u32, height: u32) -> usize {
        (width as usize) * (height as usize) * 4
    }
}

/// Uncompressed `Rgba16Float` (8 bytes per texel).
#[derive(Clone, Copy, Debug)]
pub struct Rgba16F;
impl sealed::Sealed for Rgba16F {}
impl Format for Rgba16F {
    const FORMAT: ImageFormat = ImageFormat::Rgba16F;
    fn bytes(width: u32, height: u32) -> usize {
        (width as usize) * (height as usize) * 8
    }
}

/// ASTC 4×4 block-compressed data.
#[derive(Clone, Copy, Debug)]
pub struct Astc4x4;
impl sealed::Sealed for Astc4x4 {}
impl Format for Astc4x4 {
    const FORMAT: ImageFormat = ImageFormat::Astc4x4;
    fn bytes(width: u32, height: u32) -> usize {
        (width as usize).div_ceil(4) * (height as usize).div_ceil(4) * 16
    }
}

/// ETC2 RGBA block-compressed data.
#[derive(Clone, Copy, Debug)]
pub struct Etc2Rgba;
impl sealed::Sealed for Etc2Rgba {}
impl Format for Etc2Rgba {
    const FORMAT: ImageFormat = ImageFormat::Etc2Rgba;
    fn bytes(width: u32, height: u32) -> usize {
        (width as usize).div_ceil(4) * (height as usize).div_ceil(4) * 16
    }
}

/// BC7 block-compressed data.
#[derive(Clone, Copy, Debug)]
pub struct Bc7;
impl sealed::Sealed for Bc7 {}
impl Format for Bc7 {
    const FORMAT: ImageFormat = ImageFormat::Bc7;
    fn bytes(width: u32, height: u32) -> usize {
        (width as usize).div_ceil(4) * (height as usize).div_ceil(4) * 16
    }
}

/// The untyped tag of a [`Format`], carried across the channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// Uncompressed RGBA8.
    Rgba8,
    /// Uncompressed `Rgba16Float`.
    Rgba16F,
    /// ASTC 4×4.
    Astc4x4,
    /// ETC2 RGBA.
    Etc2Rgba,
    /// BC7.
    Bc7,
}

/// The colour space an image's texels are encoded in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageColorSpace {
    /// sRGB primaries and transfer function.
    Srgb,
    /// Display P3 primaries with the sRGB transfer function.
    DisplayP3,
    /// Linear sRGB (extended values allowed).
    LinearSrgb,
    /// Linear Display P3 — the working space; decoding is the identity.
    LinearP3,
}

/// The data of an image to register, typed by its storage [`Format`].
///
/// The byte length is validated at construction, so a well-formed
/// `ImageData` always matches its dimensions.
pub struct ImageData<F: Format> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The texel data in `F`'s encoding.
    pub data: Arc<[u8]>,
    /// The encoded colour space.
    pub color_space: ImageColorSpace,
    /// Whether the texels carry premultiplied alpha.
    pub premultiplied: bool,
    /// The format tag.
    format: std::marker::PhantomData<F>,
}

impl<F: Format> std::fmt::Debug for ImageData<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageData")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("len", &self.data.len())
            .field("color_space", &self.color_space)
            .field("premultiplied", &self.premultiplied)
            .field("format", &F::FORMAT)
            .finish()
    }
}

impl<F: Format> ImageData<F> {
    /// A `width` × `height` image in `F`'s encoding.
    ///
    /// # Errors
    /// [`ResourceError::Image`] for a zero dimension or a byte-length
    /// mismatch.
    pub fn new(width: u32, height: u32, data: impl Into<Arc<[u8]>>) -> Result<Self, ResourceError> {
        let data = data.into();
        if width == 0 || height == 0 {
            return Err(ResourceError::Image("zero-size image".into()));
        }
        let want = F::bytes(width, height);
        if data.len() != want {
            return Err(ResourceError::Image(format!(
                "{width}x{height} {:?} needs {want} bytes, got {}",
                F::FORMAT,
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            data,
            color_space: ImageColorSpace::Srgb,
            premultiplied: false,
            format: std::marker::PhantomData,
        })
    }

    /// Sets the encoded colour space (default [`ImageColorSpace::Srgb`]).
    #[must_use]
    pub fn color_space(self, color_space: ImageColorSpace) -> Self {
        Self {
            color_space,
            ..self
        }
    }

    /// Marks the texels as premultiplied rather than straight alpha.
    #[must_use]
    pub fn premultiplied(self) -> Self {
        Self {
            premultiplied: true,
            ..self
        }
    }

    /// Erases the format type for the channel.
    #[must_use]
    pub(crate) fn into_upload(self) -> ImageUpload {
        ImageUpload {
            width: self.width,
            height: self.height,
            data: self.data,
            color_space: self.color_space,
            premultiplied: self.premultiplied,
            format: F::FORMAT,
        }
    }
}

/// An image crossing to the render thread: the erased, `Send` form of
/// [`ImageData`].
#[derive(Debug)]
pub struct ImageUpload {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The texel data in `format`'s encoding.
    pub data: Arc<[u8]>,
    /// The encoded colour space.
    pub color_space: ImageColorSpace,
    /// Whether the texels carry premultiplied alpha.
    pub premultiplied: bool,
    /// The storage format.
    pub format: ImageFormat,
}
