//! Rendering scene content into pixels through a Cherenkov offscreen target.
//!
//! Previews, exports and tests want a drawing as an image rather than on a
//! window. An [`OffscreenRenderer`] owns an `Engine<Gpu>`, records the content
//! into the root layer of an [`Offscreen`] surface, renders one frame and reads
//! the target back; the readback is premultiplied linear Display P3 floats,
//! which [`OffscreenImage`] unpremultiplies and encodes as sRGB8 for PNGs.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::fmt;
use core::num::NonZeroU32;
use std::path::Path;

use cherenkov::kurbo::Affine;
use cherenkov::{
    Backend, Draw as _, Engine, EngineError, FrameTime, Offscreen, OffscreenFormat, Readback,
    Recorder, RenderError, SurfaceError,
};
#[cfg(feature = "cpu")]
use cherenkov_cpu::{Raster, RasterConfig};
#[cfg(feature = "gpu")]
use cherenkov_gpu::{Gpu, GpuConfig};
use image::ImageEncoder as _;

use crate::scene::resources::{ImageUploads, Scene, SceneResources};
use crate::scene::scene_view::SceneContent;

/// A non-empty pixel size for an offscreen target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OffscreenSize {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl OffscreenSize {
    /// A size from pixel dimensions; `None` when either axis is zero.
    #[must_use]
    pub const fn try_from_pixels(width: u32, height: u32) -> Option<Self> {
        match (NonZeroU32::new(width), NonZeroU32::new(height)) {
            (Some(width), Some(height)) => Some(Self { width, height }),
            _ => None,
        }
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width.get()
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height.get()
    }
}

/// Why an offscreen render did not produce pixels.
#[derive(Debug, thiserror::Error)]
pub enum OffscreenError {
    /// No GPU engine could be created.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The offscreen surface was refused.
    #[error(transparent)]
    Surface(#[from] SurfaceError),
    /// The frame did not render or could not be read back.
    #[error(transparent)]
    Render(#[from] RenderError),
}

/// The pixels of an offscreen render: straight-alpha sRGB8, row-major.
#[derive(Debug, Clone)]
pub struct OffscreenImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of straight-alpha sRGB8.
    pub rgba8: Vec<u8>,
}

impl OffscreenImage {
    /// Encodes the readback's premultiplied linear P3 as straight sRGB8,
    /// mapping P3 primaries to sRGB through XYZ and clipping what falls
    /// outside the sRGB gamut.
    #[must_use]
    pub fn from_readback(readback: &Readback) -> Self {
        let rgba8 = readback
            .pixels
            .iter()
            .flat_map(|&[r, g, b, a]| {
                let straight = if a > 0.0 {
                    [r / a, g / a, b / a]
                } else {
                    [0.0; 3]
                };
                let [r, g, b] = p3_to_srgb(straight);
                [encode(r), encode(g), encode(b), encode_alpha(a)]
            })
            .collect();
        Self {
            width: readback.width,
            height: readback.height,
            rgba8,
        }
    }

    /// The image as premultiplied sRGB8: each channel is its straight value
    /// scaled by the pixel's alpha, the layout image views composite directly.
    #[must_use]
    pub fn premultiplied_rgba8(&self) -> Vec<u8> {
        self.rgba8
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| {
                let alpha = f32::from(px[3]) / 255.0;
                let mul = |c: u8| encode_alpha(f32::from(c) / 255.0 * alpha);
                [mul(px[0]), mul(px[1]), mul(px[2]), px[3]]
            })
            .collect()
    }

    /// The pixel at `(x, y)`.
    ///
    /// # Panics
    /// When `(x, y)` lies outside the image.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(x < self.width && y < self.height, "pixel outside the image");
        let offset = ((y * self.width + x) * 4) as usize;
        self.rgba8[offset..offset + 4]
            .try_into()
            .expect("a pixel is four bytes")
    }

    /// Writes the image as a PNG.
    ///
    /// # Errors
    /// When the file cannot be created or the encoder rejects the data.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), image::ImageError> {
        let file = std::fs::File::create(path)?;
        image::codecs::png::PngEncoder::new(std::io::BufWriter::new(file)).write_image(
            &self.rgba8,
            self.width,
            self.height,
            image::ExtendedColorType::Rgba8,
        )
    }
}

fn encode(linear: f32) -> u8 {
    let clipped = linear.clamp(0.0, 1.0);
    let encoded = if clipped <= 0.003_130_8 {
        clipped * 12.92
    } else {
        1.055_f32.mul_add(clipped.powf(1.0 / 2.4), -0.055)
    };
    encode_alpha(encoded)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to [0, 255] before the cast"
)]
fn encode_alpha(unit: f32) -> u8 {
    (unit.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Linear Display P3 to linear sRGB, via XYZ D65.
fn p3_to_srgb([r, g, b]: [f32; 3]) -> [f32; 3] {
    const P3_TO_SRGB: [[f32; 3]; 3] = [
        [1.224_940_2, -0.224_940_2, 0.0],
        [-0.042_056_96, 1.042_056_9, 0.0],
        [-0.019_637_55, -0.078_636_05, 1.098_273_6],
    ];
    let row = |m: [f32; 3]| m[2].mul_add(b, m[0].mul_add(r, m[1] * g));
    [row(P3_TO_SRGB[0]), row(P3_TO_SRGB[1]), row(P3_TO_SRGB[2])]
}

#[expect(
    clippy::cast_precision_loss,
    reason = "a texture side is far below f32's exact integer range"
)]
fn pixels_to_points(pixels: u32, scale: f32) -> f32 {
    pixels as f32 / scale
}

/// An engine that renders scene content into images through backend `B`.
pub struct OffscreenRenderer<B: Backend> {
    engine: Rc<Engine<B>>,
}

impl<B: Backend> fmt::Debug for OffscreenRenderer<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OffscreenRenderer").finish_non_exhaustive()
    }
}

#[cfg(feature = "gpu")]
impl OffscreenRenderer<Gpu> {
    /// Creates a GPU engine on the default adapter.
    ///
    /// # Errors
    /// [`EngineError`] when no adapter is available.
    pub fn new() -> Result<Self, OffscreenError> {
        Self::with_config(GpuConfig::default())
    }
}

#[cfg(feature = "cpu")]
impl OffscreenRenderer<Raster> {
    /// Creates a CPU raster engine.
    ///
    /// # Errors
    /// [`EngineError`] when the raster backend cannot initialise.
    pub fn cpu() -> Result<Self, OffscreenError> {
        Self::with_config(RasterConfig::default())
    }
}

impl<B: ImageUploads> OffscreenRenderer<B> {
    /// Creates an engine with an explicit backend configuration.
    ///
    /// # Errors
    /// [`EngineError`] when the backend cannot initialise.
    pub fn with_config(config: B::Config) -> Result<Self, OffscreenError> {
        Ok(Self {
            engine: Rc::new(Engine::<B>::new(config)?),
        })
    }

    /// The engine's resources, for content that mints handles ahead of a render.
    #[must_use]
    pub fn resources(&self) -> Rc<dyn SceneResources> {
        Rc::clone(&self.engine) as Rc<dyn SceneResources>
    }

    /// Records `content` laid out at `size / scale` points, renders one frame
    /// at `size` pixels and reads it back.
    ///
    /// # Errors
    /// [`OffscreenError`] when the surface cannot be created, the frame does
    /// not render, or the target cannot be read back.
    pub fn render(
        &self,
        content: &mut dyn SceneContent,
        size: OffscreenSize,
        scale: f32,
    ) -> Result<OffscreenImage, OffscreenError> {
        let width = pixels_to_points(size.width(), scale);
        let height = pixels_to_points(size.height(), scale);
        let surface = self.engine.surface(Offscreen::new(
            (size.width(), size.height()),
            OffscreenFormat::LinearF16,
        ))?;
        let resources = self.resources();
        let recorded = surface.record(|recorder: &mut Recorder| {
            recorder.transform(Affine::scale(f64::from(scale)), |recorder| {
                let mut scene = Scene::new(recorder, &resources, width, height);
                content.record(&mut scene);
            });
        });
        surface.update(|tx| {
            tx[surface.root()].content(recorded);
        });
        self.engine.render(FrameTime::now())?;
        Ok(OffscreenImage::from_readback(&surface.readback()?))
    }

    /// Renders a recorded `picture` under `transform` into `size` pixels and
    /// reads it back.
    ///
    /// # Errors
    /// [`OffscreenError`] when the surface cannot be created, the frame does
    /// not render, or the target cannot be read back.
    pub fn render_picture(
        &self,
        picture: &cherenkov::Picture,
        size: OffscreenSize,
        transform: Affine,
    ) -> Result<OffscreenImage, OffscreenError> {
        let surface = self.engine.surface(Offscreen::new(
            (size.width(), size.height()),
            OffscreenFormat::LinearF16,
        ))?;
        let recorded = surface.record(|recorder: &mut Recorder| {
            recorder.picture(picture, transform);
        });
        surface.update(|tx| {
            tx[surface.root()].content(recorded);
        });
        self.engine.render(FrameTime::now())?;
        Ok(OffscreenImage::from_readback(&surface.readback()?))
    }
}

#[cfg(test)]
mod tests {
    use super::{OffscreenImage, OffscreenSize};
    use alloc::vec;
    use cherenkov::Readback;

    #[test]
    fn a_zero_axis_is_not_a_size() {
        assert!(OffscreenSize::try_from_pixels(0, 4).is_none());
        assert!(OffscreenSize::try_from_pixels(4, 0).is_none());
        let size = OffscreenSize::try_from_pixels(4, 2).expect("non-zero");
        assert_eq!((size.width(), size.height()), (4, 2));
    }

    #[test]
    fn readback_unpremultiplies_and_encodes_srgb() {
        let readback = Readback {
            width: 2,
            height: 1,
            pixels: vec![[0.5, 0.5, 0.5, 0.5], [0.0, 0.0, 0.0, 0.0]],
        };
        let image = OffscreenImage::from_readback(&readback);
        assert_eq!(image.pixel(0, 0), [255, 255, 255, 128]);
        assert_eq!(image.pixel(1, 0), [0, 0, 0, 0]);
    }
}
