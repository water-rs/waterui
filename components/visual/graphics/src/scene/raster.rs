//! CPU rasterisation of a [`Picture`] through `cherenkov_cpu`.
//!
//! The FFI backends rasterise a [`Picture`] here to hand the pixels to a
//! platform image view. It needs no GPU device, which is the point — a static
//! drawing must not cost a wgpu runtime.
//!
//! [`Picture`]: crate::picture::Picture

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::fmt;

use cherenkov::kurbo::Affine;
use cherenkov::{
    Engine, FrameTime, LayerContent, Offscreen, OffscreenFormat, Recorder, RenderError, Surface,
    SurfaceError,
};
use cherenkov_cpu::{Raster, RasterConfig, present_srgb8};

use crate::scene::picture::PictureRecording;
use crate::scene::resources::{HeldResources, SceneResources};
use crate::scene::scene_view::SceneContent;

/// A premultiplied sRGB8 raster, rows top to bottom with no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaBitmap {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl RgbaBitmap {
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

    /// The pixels, `width * height * 4` bytes of premultiplied RGBA.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Takes the pixel buffer.
    #[must_use]
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }
}

/// Rasterisation failure: engine or surface construction, render, or
/// readback.
#[derive(Debug)]
pub enum RasterizeError {
    /// The engine or the offscreen target could not be created.
    Surface(SurfaceError),
    /// The frame failed to render or be read back.
    Render(RenderError),
}

impl fmt::Display for RasterizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Surface(e) => e.fmt(f),
            Self::Render(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RasterizeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Surface(e) => Some(e),
            Self::Render(e) => Some(e),
        }
    }
}

impl From<SurfaceError> for RasterizeError {
    fn from(e: SurfaceError) -> Self {
        Self::Surface(e)
    }
}

impl From<RenderError> for RasterizeError {
    fn from(e: RenderError) -> Self {
        Self::Render(e)
    }
}

/// A CPU rasteriser for one pixel size, reusable across drawings.
///
/// Keeping one of these per picture means a drawing that changes often (a
/// tint following an animation, say) re-renders into the same engine and
/// surface — glyph masks and uploaded images stay cached — instead of
/// building a fresh raster pipeline for every frame.
///
/// The engine behind a `Rasterizer` is real, so pictures that name engine
/// resources — fonts, images, shader paints — must have recorded those
/// resources against this one: record them with
/// [`Picture::record_with`](crate::picture::Picture::record_with) over
/// [`Rasterizer::resources`], not another engine.
///
/// Every drawing it takes carries the registrations it names — a
/// [`PictureRecording`] holds them, and scene content names them while it
/// records — and the rasteriser keeps the drawing it last installed holding
/// them until the next drawing replaces it, so nothing a drawing names can be
/// released while it can still be drawn.
pub struct Rasterizer {
    engine: Rc<Engine<Raster>>,
    resources: SceneResources,
    surface: Surface<Raster>,
    installed: HeldResources,
    width: u32,
    height: u32,
}

impl fmt::Debug for Rasterizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rasterizer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Rasterizer {
    /// A rasteriser producing `width × height` bitmaps.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::Engine`] when the engine's render thread fails to
    /// start, [`SurfaceError`] itself when the offscreen target cannot be
    /// created.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(width: u32, height: u32) -> Result<Self, SurfaceError> {
        let engine = Rc::new(Engine::<Raster>::new(RasterConfig::default())?);
        let surface =
            engine.surface(Offscreen::new((width, height), OffscreenFormat::LinearF16))?;
        Ok(Self {
            resources: SceneResources::new(Rc::clone(&engine)),
            engine,
            surface,
            installed: HeldResources::empty(),
            width,
            height,
        })
    }

    /// A rasteriser producing `width × height` bitmaps.
    ///
    /// # Errors
    ///
    /// [`SurfaceError`] when the offscreen target cannot be created.
    #[cfg(target_arch = "wasm32")]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
        )
    )]
    pub async fn new(width: u32, height: u32) -> Result<Self, SurfaceError> {
        let engine = Rc::new(Engine::<Raster>::new(RasterConfig::default()).await?);
        let surface = engine
            .surface(Offscreen::new((width, height), OffscreenFormat::LinearF16))
            .await?;
        Ok(Self {
            resources: SceneResources::new(Rc::clone(&engine)),
            engine,
            surface,
            installed: HeldResources::empty(),
            width,
            height,
        })
    }

    /// Resource registration over this rasteriser's engine, for pictures that
    /// record fonts, images or shader paints: handles minted here are the
    /// only ones the engine owns.
    #[must_use]
    pub const fn resources(&self) -> &SceneResources {
        &self.resources
    }

    /// Rasterises `recording` drawn under `transform` (the caller maps its
    /// points onto these pixels there).
    ///
    /// # Errors
    ///
    /// [`RenderError`] when the render or the readback fails.
    ///
    /// # Panics
    ///
    /// When `recording` names resources registered on another engine.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn rasterize(
        &mut self,
        recording: &PictureRecording,
        transform: Affine,
    ) -> Result<RgbaBitmap, RenderError> {
        let mut resources = self.resources.recording();
        resources.hold(recording.held());
        let held = resources.finish();
        self.show(recording.picture().clone(), held, transform)
    }

    /// Rasterises `recording` drawn under `transform` (the caller maps its
    /// points onto these pixels there).
    ///
    /// # Errors
    ///
    /// [`RenderError`] when the render or the readback fails.
    ///
    /// # Panics
    ///
    /// When `recording` names resources registered on another engine.
    #[cfg(target_arch = "wasm32")]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
        )
    )]
    pub async fn rasterize(
        &mut self,
        recording: &PictureRecording,
        transform: Affine,
    ) -> Result<RgbaBitmap, RenderError> {
        let mut resources = self.resources.recording();
        resources.hold(recording.held());
        let held = resources.finish();
        self.show(recording.picture().clone(), held, transform)
            .await
    }

    /// Records `content` laid out at this rasteriser's pixel size divided by
    /// `scale`, in points, and rasterises it at `scale` pixels per point.
    ///
    /// The content records against [`Rasterizer::resources`], so what it
    /// registers while drawing is this engine's; it keeps those handles
    /// across calls the way it would on a window.
    ///
    /// # Errors
    ///
    /// [`RenderError`] when the render or the readback fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn rasterize_scene(
        &mut self,
        content: &mut dyn SceneContent,
        scale: f32,
    ) -> Result<RgbaBitmap, RenderError> {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a rasteriser's sides are at most 65535 pixels, which f32 holds exactly"
        )]
        let (width, height) = (self.width as f32 / scale, self.height as f32 / scale);
        let mut resources = self.resources.recording();
        let recorded = self.surface.record(|recorder: &mut Recorder| {
            content.build_scene(recorder, &mut resources, width, height);
        });
        let held = resources.finish();
        self.show(recorded, held, Affine::scale(f64::from(scale)))
    }

    /// Records `content` laid out at this rasteriser's pixel size divided by
    /// `scale`, in points, and rasterises it at `scale` pixels per point.
    ///
    /// The content records against [`Rasterizer::resources`], so what it
    /// registers while drawing is this engine's; it keeps those handles
    /// across calls the way it would on a window.
    ///
    /// # Errors
    ///
    /// [`RenderError`] when the render or the readback fails.
    #[cfg(target_arch = "wasm32")]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
        )
    )]
    pub async fn rasterize_scene(
        &mut self,
        content: &mut dyn SceneContent,
        scale: f32,
    ) -> Result<RgbaBitmap, RenderError> {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a rasteriser's sides are at most 65535 pixels, which f32 holds exactly"
        )]
        let (width, height) = (self.width as f32 / scale, self.height as f32 / scale);
        let mut resources = self.resources.recording();
        let recorded = self.surface.record(|recorder: &mut Recorder| {
            content.build_scene(recorder, &mut resources, width, height);
        });
        let held = resources.finish();
        self.show(recorded, held, Affine::scale(f64::from(scale)))
            .await
    }

    /// Installs `content` on the root under `transform`, then lets go of what
    /// the drawing it replaces held, renders and reads the pixels back.
    #[cfg(not(target_arch = "wasm32"))]
    fn show(
        &mut self,
        content: impl Into<LayerContent<Raster>>,
        held: HeldResources,
        transform: Affine,
    ) -> Result<RgbaBitmap, RenderError> {
        let root = self.surface.root();
        self.surface.update(|tx| {
            tx[root].transform(transform);
            tx[root].content(content.into());
        });
        self.installed = held;
        self.engine.render(FrameTime::now())?;
        let readback = self.surface.readback()?;
        Ok(RgbaBitmap {
            width: self.width,
            height: self.height,
            // An image view in a platform surface is an SDR sRGB destination:
            // the presentation pass tone-maps to its ceiling.
            data: present_srgb8(1.0, &readback.pixels),
        })
    }

    /// Installs `content` on the root under `transform`, then lets go of what
    /// the drawing it replaces held, renders and reads the pixels back.
    #[cfg(target_arch = "wasm32")]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
        )
    )]
    async fn show(
        &mut self,
        content: impl Into<LayerContent<Raster>>,
        held: HeldResources,
        transform: Affine,
    ) -> Result<RgbaBitmap, RenderError> {
        let root = self.surface.root();
        self.surface.update(|tx| {
            tx[root].transform(transform);
            tx[root].content(content.into());
        });
        self.installed = held;
        self.engine.render(FrameTime::now()).await?;
        let readback = self.surface.readback().await?;
        Ok(RgbaBitmap {
            width: self.width,
            height: self.height,
            // An image view in a platform surface is an SDR sRGB destination:
            // the presentation pass tone-maps to its ceiling.
            data: present_srgb8(1.0, &readback.pixels),
        })
    }
}

/// Rasterises `recording` once into a `width × height` pixel bitmap, drawing
/// it under `transform`; see [`Rasterizer`] for repeated drawings.
///
/// A fresh engine draws it, so the recording can name no engine resource:
/// one that does was recorded against another engine, and panics.
///
/// # Errors
///
/// As [`Rasterizer::new`] and [`Rasterizer::rasterize`].
#[cfg(not(target_arch = "wasm32"))]
pub fn rasterize_picture(
    recording: &PictureRecording,
    width: u32,
    height: u32,
    transform: Affine,
) -> Result<RgbaBitmap, RasterizeError> {
    Rasterizer::new(width, height)?
        .rasterize(recording, transform)
        .map_err(RasterizeError::Render)
}

/// Rasterises `recording` once into a `width × height` pixel bitmap, drawing
/// it under `transform`; see [`Rasterizer`] for repeated drawings.
///
/// A fresh engine draws it, so the recording can name no engine resource:
/// one that does was recorded against another engine, and panics.
///
/// # Errors
///
/// As [`Rasterizer::new`] and [`Rasterizer::rasterize`].
#[cfg(target_arch = "wasm32")]
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::future_not_send,
        reason = "holds the engine's Rc-based wasm32 backend handles across an await, so the future is !Send; every future on wasm32 runs on the browser's single-threaded executor"
    )
)]
pub async fn rasterize_picture(
    recording: &PictureRecording,
    width: u32,
    height: u32,
    transform: Affine,
) -> Result<RgbaBitmap, RasterizeError> {
    Rasterizer::new(width, height)
        .await?
        .rasterize(recording, transform)
        .await
        .map_err(RasterizeError::Render)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cherenkov::kurbo::{Rect, Shape};
    use cherenkov::{Color, Draw, Srgb, StaticRecorder, WorkingColor};

    use crate::picture::Picture;

    /// Linear-working red is Display P3's red, which sRGB cannot hold: the
    /// expectations below come from naming an sRGB colour instead of a
    /// working-space triplet, not from hoping the rasteriser clips for us.
    fn square(color: WorkingColor) -> PictureRecording {
        Picture::record(move |scene: &mut StaticRecorder| {
            scene.fill(Rect::new(0.0, 0.0, 4.0, 4.0).to_path(0.05), color);
        })
    }

    fn srgb_red() -> WorkingColor {
        Color::<Srgb>::new([1.0, 0.0, 0.0, 1.0]).into()
    }

    #[test]
    fn rasterises_a_picture_at_the_requested_scale() {
        let bitmap = rasterize_picture(&square(srgb_red()), 8, 8, Affine::scale(2.0))
            .expect("CPU rasterisation failed");
        assert_eq!((bitmap.width(), bitmap.height()), (8, 8));
        let pixel = |x: usize, y: usize| {
            let i = (y * 8 + x) * 4;
            [
                bitmap.data()[i],
                bitmap.data()[i + 1],
                bitmap.data()[i + 2],
                bitmap.data()[i + 3],
            ]
        };
        let px = pixel(7, 7);
        assert!(
            px[0] > 250 && px[1] < 5 && px[2] < 5 && px[3] == 255,
            "{px:?}"
        );
    }

    #[test]
    fn a_reused_rasteriser_starts_every_drawing_from_a_clean_scene() {
        let mut rasterizer = Rasterizer::new(4, 4).expect("engine failed to start");
        let red = rasterizer
            .rasterize(&square(srgb_red()), Affine::IDENTITY)
            .expect("rasterise failed");
        assert!(red.data()[0] > 250 && red.data()[1] < 5 && red.data()[2] < 5);
        let empty = rasterizer
            .rasterize(&Picture::record(|_| {}), Affine::IDENTITY)
            .expect("rasterise failed");
        assert!(
            empty.data().iter().all(|byte| *byte == 0),
            "the first drawing leaked into the second"
        );
    }
}
