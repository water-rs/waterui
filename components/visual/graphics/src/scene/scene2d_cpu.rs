//! CPU rasterisation of a [`SceneRecording`] through `vello_cpu`.
//!
//! Two users share this: the FFI backends rasterise a [`Picture`] here to hand
//! the pixels to a platform image view, and Dew paints every scene with it.
//! Neither needs a GPU device, which is the point — a static drawing must not
//! cost a wgpu runtime.
//!
//! [`Picture`]: crate::picture::Picture

use alloc::vec::Vec;
use core::fmt;

use kurbo::{Affine, BezPath, Rect, Shape, Stroke};
use peniko::{BlendMode, Brush, Fill, ImageBrush, StyleRef};
use vello_cpu::{Image, ImageSource, RenderContext, RenderMode, RenderSettings, Resources};

use crate::scene2d::{GlyphRun, Scene2D, SceneRecording};

/// Flattening tolerance for the rectangles this module turns into paths.
const BEZIER_TOLERANCE: f64 = 0.05;

/// Image sources uploaded to `vello_cpu`, reused across scenes that draw the
/// same image data.
#[derive(Debug, Default)]
pub struct CpuImageCache {
    images: Vec<CachedImage>,
}

#[derive(Debug)]
struct CachedImage {
    data: peniko::ImageData,
    source: ImageSource,
}

impl CpuImageCache {
    /// An empty cache.
    #[must_use]
    pub const fn new() -> Self {
        Self { images: Vec::new() }
    }

    /// How many distinct images have been uploaded.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.images.len()
    }

    /// Whether no image has been uploaded yet.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    fn source_for(&mut self, image: &ImageBrush) -> ImageSource {
        if let Some(cached) = self.images.iter().find(|cached| cached.data == image.image) {
            return cached.source.clone();
        }
        let source = ImageSource::from_peniko_image_data(&image.image);
        self.images.push(CachedImage {
            data: image.image.clone(),
            source: source.clone(),
        });
        source
    }

    /// Selects `brush` as the paint of `ctx`, uploading an image brush once.
    pub fn set_brush(&mut self, ctx: &mut RenderContext, brush: &Brush) {
        match brush {
            Brush::Solid(color) => ctx.set_paint(*color),
            Brush::Gradient(gradient) => ctx.set_paint(gradient.clone()),
            Brush::Image(image) => {
                let source = self.source_for(image);
                ctx.set_paint(Image {
                    image: source,
                    sampler: image.sampler,
                });
            }
        }
    }
}

/// A [`Scene2D`] that paints straight into a `vello_cpu` render context.
pub struct CpuScene<'a> {
    ctx: &'a mut RenderContext,
    resources: &'a mut Resources,
    images: &'a mut CpuImageCache,
    depth: usize,
}

impl fmt::Debug for CpuScene<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CpuScene")
            .field("depth", &self.depth)
            .finish_non_exhaustive()
    }
}

impl<'a> CpuScene<'a> {
    /// Paints into `ctx`, resolving glyphs through `resources` and image
    /// brushes through `images`.
    pub const fn new(
        ctx: &'a mut RenderContext,
        resources: &'a mut Resources,
        images: &'a mut CpuImageCache,
    ) -> Self {
        Self {
            ctx,
            resources,
            images,
            depth: 0,
        }
    }

    /// How many layers are pushed and not yet popped.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    fn paint(&mut self, brush: &Brush, brush_transform: Option<Affine>) {
        self.images.set_brush(self.ctx, brush);
        match brush_transform {
            Some(transform) => self.ctx.set_paint_transform(transform),
            None => self.ctx.reset_paint_transform(),
        }
    }
}

impl Scene2D for CpuScene<'_> {
    fn fill(
        &mut self,
        fill: Fill,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        self.ctx.set_transform(transform);
        self.ctx.set_fill_rule(fill);
        self.paint(brush, brush_transform);
        self.ctx.fill_path(shape);
    }

    fn stroke(
        &mut self,
        stroke: &Stroke,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        self.ctx.set_transform(transform);
        self.ctx.set_stroke(stroke.clone());
        self.paint(brush, brush_transform);
        self.ctx.stroke_path(shape);
    }

    fn push_layer(
        &mut self,
        fill: Fill,
        blend: BlendMode,
        alpha: f32,
        transform: Affine,
        clip: &BezPath,
    ) {
        self.ctx.set_transform(transform);
        self.ctx.set_fill_rule(fill);
        self.ctx
            .push_layer(Some(clip), Some(blend), Some(alpha), None, None);
        self.depth += 1;
    }

    fn push_clip_layer(&mut self, fill: Fill, transform: Affine, clip: &BezPath) {
        self.ctx.set_transform(transform);
        self.ctx.set_fill_rule(fill);
        self.ctx.push_layer(Some(clip), None, None, None, None);
        self.depth += 1;
    }

    fn pop_layer(&mut self) {
        assert!(
            self.depth > 0,
            "scene content popped a layer it never pushed"
        );
        self.depth -= 1;
        self.ctx.pop_layer();
    }

    fn draw_image(&mut self, image: &ImageBrush, transform: Affine) {
        let bounds = Rect::new(
            0.0,
            0.0,
            f64::from(image.image.width),
            f64::from(image.image.height),
        );
        self.fill(
            Fill::NonZero,
            transform,
            &Brush::Image(image.clone()),
            None,
            &bounds.to_path(BEZIER_TOLERANCE),
        );
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_>) {
        self.ctx.set_transform(run.transform);
        let brush = if run.brush_alpha < 1.0 {
            run.brush.clone().multiply_alpha(run.brush_alpha)
        } else {
            run.brush.clone()
        };
        self.paint(&brush, None);
        match run.style {
            StyleRef::Fill(fill) => self.ctx.set_fill_rule(fill),
            StyleRef::Stroke(stroke) => self.ctx.set_stroke(stroke.clone()),
        }
        let glyphs = run.glyphs.iter().map(|glyph| vello_cpu::Glyph {
            id: glyph.id,
            x: glyph.x,
            y: glyph.y,
        });
        let builder = self
            .ctx
            .glyph_run(self.resources, run.font)
            .font_size(run.font_size)
            .normalized_coords(run.normalized_coords)
            .hint(true);
        match run.style {
            StyleRef::Fill(_) => builder.fill_glyphs(glyphs),
            StyleRef::Stroke(_) => builder.stroke_glyphs(glyphs),
        }
    }

    fn reset(&mut self) {
        panic!("a CPU scene paints commands as they arrive and has no recording to reset");
    }
}

/// Replays `recording` into `ctx` under `transform`, leaving the context's
/// state as it found it.
///
/// # Panics
///
/// When the recording pushes a layer it never pops: that is a defect in the
/// content, not a condition to paint around.
pub fn replay_recording(
    ctx: &mut RenderContext,
    resources: &mut Resources,
    images: &mut CpuImageCache,
    recording: &SceneRecording,
    transform: Affine,
) {
    let saved = ctx.save_current_state();
    let depth = {
        let mut scene = CpuScene::new(ctx, resources, images);
        recording.replay(&mut scene, Some(transform));
        scene.depth()
    };
    assert_eq!(
        depth, 0,
        "scene content left {depth} layer(s) unpopped: every push must have a matching pop"
    );
    ctx.restore_state(saved);
}

/// A premultiplied RGBA8 raster, rows top to bottom with no padding.
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

/// A CPU rasteriser for one pixel size, reusable across drawings.
///
/// Keeping one of these per picture means a drawing that changes often (a
/// tint following an animation, say) re-encodes into the same render context
/// and image registry instead of allocating a fresh set of strip buffers for
/// every frame.
pub struct Rasterizer {
    ctx: RenderContext,
    resources: Resources,
    images: CpuImageCache,
    width: u32,
    height: u32,
}

impl fmt::Debug for Rasterizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rasterizer")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("images", &self.images.len())
            .finish_non_exhaustive()
    }
}

impl Rasterizer {
    /// A rasteriser producing `width × height` bitmaps.
    ///
    /// # Panics
    ///
    /// `vello_cpu` addresses pixels with 16-bit coordinates; a bitmap larger
    /// than that is not a picture any image view would show, so it is a panic.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let (width_px, height_px) = (
            u16::try_from(width).expect("a rasterised picture is at most 65535 pixels wide"),
            u16::try_from(height).expect("a rasterised picture is at most 65535 pixels tall"),
        );
        Self {
            ctx: RenderContext::new_with(width_px, height_px, RenderSettings::default()),
            resources: Resources::new(),
            images: CpuImageCache::new(),
            width,
            height,
        }
    }

    /// Rasterises `recording`, drawn under `transform` (the caller maps its
    /// points onto these pixels there).
    ///
    /// # Panics
    ///
    /// A recording that leaves a layer open is a content defect and panics.
    #[must_use]
    pub fn rasterize(&mut self, recording: &SceneRecording, transform: Affine) -> RgbaBitmap {
        self.ctx.reset();
        replay_recording(
            &mut self.ctx,
            &mut self.resources,
            &mut self.images,
            recording,
            transform,
        );
        self.ctx.flush();
        let mut data = alloc::vec![0; self.width as usize * self.height as usize * 4];
        #[expect(
            clippy::cast_possible_truncation,
            reason = "`new` already proved both sides fit in a u16"
        )]
        self.ctx.render_to_buffer(
            &mut self.resources,
            &mut data,
            self.width as u16,
            self.height as u16,
            RenderMode::OptimizeSpeed,
        );
        RgbaBitmap {
            width: self.width,
            height: self.height,
            data,
        }
    }
}

/// Rasterises `recording` once into a `width × height` pixel bitmap, drawing
/// it under `transform`; see [`Rasterizer`] for repeated drawings.
///
/// # Panics
///
/// As [`Rasterizer::new`] and [`Rasterizer::rasterize`].
#[must_use]
pub fn rasterize_recording(
    recording: &SceneRecording,
    width: u32,
    height: u32,
    transform: Affine,
) -> RgbaBitmap {
    Rasterizer::new(width, height).rasterize(recording, transform)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Rect;
    use peniko::Color;

    #[test]
    fn rasterises_a_recording_at_the_requested_scale() {
        let mut recording = SceneRecording::new();
        recording.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &Brush::Solid(Color::from_rgb8(255, 0, 0)),
            None,
            &Rect::new(0.0, 0.0, 4.0, 4.0).to_path(BEZIER_TOLERANCE),
        );
        let bitmap = rasterize_recording(&recording, 8, 8, Affine::scale(2.0));
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
        assert_eq!(pixel(7, 7), [255, 0, 0, 255]);
    }

    #[test]
    fn a_reused_rasteriser_starts_every_drawing_from_a_clean_scene() {
        let square = |color: Color| {
            let mut recording = SceneRecording::new();
            recording.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                &Brush::Solid(color),
                None,
                &Rect::new(0.0, 0.0, 2.0, 2.0).to_path(BEZIER_TOLERANCE),
            );
            recording
        };
        let mut rasterizer = Rasterizer::new(4, 4);
        let red = rasterizer.rasterize(&square(Color::from_rgb8(255, 0, 0)), Affine::IDENTITY);
        assert_eq!(&red.data()[..4], [255, 0, 0, 255]);
        let empty = rasterizer.rasterize(&SceneRecording::new(), Affine::IDENTITY);
        assert!(
            empty.data().iter().all(|byte| *byte == 0),
            "the first drawing leaked into the second"
        );
    }

    #[test]
    #[should_panic(expected = "layer(s) unpopped")]
    fn a_layer_left_open_is_a_content_defect() {
        let mut recording = SceneRecording::new();
        recording.push_clip_layer(
            Fill::NonZero,
            Affine::IDENTITY,
            &Rect::new(0.0, 0.0, 1.0, 1.0).to_path(BEZIER_TOLERANCE),
        );
        let _ = rasterize_recording(&recording, 2, 2, Affine::IDENTITY);
    }
}
