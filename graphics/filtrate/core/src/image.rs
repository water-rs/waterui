//! Auxiliary images a filter provides to its stages.

use core::any::Any;

/// The texel format of an auxiliary image's CPU data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuxFormat {
    /// Four unorm bytes per texel, RGBA.
    Rgba8,
    /// Four half-precision floats per texel, RGBA.
    Rgba16Float,
    /// Four single-precision floats per texel, RGBA.
    Rgba32Float,
}

impl AuxFormat {
    /// The byte count of one texel.
    #[must_use]
    pub const fn texel_size(self) -> usize {
        match self {
            Self::Rgba8 => 4,
            Self::Rgba16Float => 8,
            Self::Rgba32Float => 16,
        }
    }
}

/// The CPU texels of an auxiliary image, and their format.
#[derive(Debug, Clone, Copy)]
pub struct AuxData<'a> {
    /// The texel format.
    pub format: AuxFormat,
    /// `width * height * format.texel_size()` bytes, row-major.
    pub bytes: &'a [u8],
}

/// An image a filter binds to one of its stages' `aux` arguments.
///
/// The executor reads the bytes once, uploads them at their native format
/// (no colour conversion and no premultiplication) and samples them with
/// nearest filtering. A source that is not CPU data — a caller-provided GPU
/// texture — returns `None` from [`AuxImage::data`] and is recognized
/// through [`AuxImage::as_any`] instead.
pub trait AuxImage {
    /// Width in pixels.
    fn width(&self) -> u32;

    /// Height in pixels.
    fn height(&self) -> u32;

    /// The image's texels, or `None` when the image is not CPU data.
    fn data(&self) -> Option<AuxData<'_>>;

    /// The image as [`Any`], for executors that recognize a GPU source by
    /// downcasting (a texture the caller already uploaded). `None` for a
    /// plain CPU image.
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
}

/// Receives each image of a filter, in image-index order.
pub trait ImageVisitor {
    /// Visits image `index`.
    fn visit<I: AuxImage + ?Sized>(&mut self, index: usize, image: &I);
}
