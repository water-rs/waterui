//! The `vello_cpu` bridge: rasterizes a display list into a region-sized
//! scratch pixmap.
//!
//! This is the only module that touches `vello_cpu` directly; its 0.0.x API
//! is expected to change and the churn must stay contained here. Region
//! rendering works by translating every command by the region origin and
//! rasterizing into a context exactly the size of the region — sparse-strip
//! rasterization only pays for covered pixels, so this is cheap even though
//! the full scene is replayed.
//!
//! It is also where dew implements [`Scene2D`]: [`CpuScene`] projects the
//! engine-neutral scene contract straight onto the rasterizer, which is what
//! lets `Canvas` drawings and SVG documents render here with no engine of
//! their own. Confining it to this module is the same rule as everything else
//! about `vello_cpu`, and the reason a scene reaches the painter as an opaque
//! recording rather than as display-list commands.

use kurbo::{Affine, BezPath, Rect, Shape, Stroke};
use peniko::{BlendMode, Brush, Fill, ImageBrush, StyleRef};
use vello_cpu::{Image, ImageSource, Pixmap, RenderContext, RenderMode, RenderSettings, Resources};
use waterui_graphics::{GlyphRun, Scene2D, SceneRecording};

use crate::compositor::DeviceRegion;
use crate::display_list::{BEZIER_TOLERANCE, Clip, ClipRegion, DisplayList, DrawCommand};
use crate::stats::FrameWork;

/// The rasterizer: owns the persistent `vello_cpu` resources (glyph atlas,
/// image registry) that must survive across bands and frames.
///
/// One painter per screen; create it once and reuse it for every region.
#[derive(Debug)]
pub struct Painter {
    resources: Resources,
    settings: RenderSettings,
    images: Vec<CachedImage>,
    scratch: Vec<ScratchSlot>,
}

#[derive(Debug)]
struct CachedImage {
    data: peniko::ImageData,
    source: ImageSource,
}

/// A reusable render context and pixmap for one region size.
///
/// A `RenderContext` is fixed-size, and steady-state dirty regions repeat the
/// same handful of sizes frame after frame (an animating progress bar dirties
/// identical bands every frame). Recreating the context and pixmap per band
/// was the single largest source of per-frame heap churn the work simulation
/// measured, and on an RTOS heap churn is fragmentation pressure — so the
/// painter keeps one slot per recent size and `reset()`s it instead.
#[derive(Debug)]
struct ScratchSlot {
    width: u16,
    height: u16,
    context: RenderContext,
    pixmap: Pixmap,
}

/// Distinct region sizes kept alive for reuse, least recently used evicted.
///
/// Steady-state frames cycle through only a few sizes; the bound exists so a
/// pathological size storm cannot hoard band-sized buffers.
const SCRATCH_SLOTS: usize = 8;

impl Default for Painter {
    fn default() -> Self {
        Self::new(target_render_settings())
    }
}

impl Painter {
    /// Creates a painter with empty caches and explicit target settings.
    #[must_use]
    pub fn new(settings: RenderSettings) -> Self {
        Self {
            resources: Resources::new(),
            settings,
            images: Vec::new(),
            scratch: Vec::new(),
        }
    }

    /// The reusable scratch slot for a `width` × `height` region, creating it
    /// (evicting the least recently used) when absent. The returned slot's
    /// context is reset and ready to encode.
    fn scratch_slot(&mut self, width: u16, height: u16) -> &mut ScratchSlot {
        if let Some(index) = self
            .scratch
            .iter()
            .position(|slot| slot.width == width && slot.height == height)
        {
            // Move to the back: the back is the most recently used.
            let slot = self.scratch.remove(index);
            self.scratch.push(slot);
        } else {
            if self.scratch.len() == SCRATCH_SLOTS {
                self.scratch.remove(0);
            }
            self.scratch.push(ScratchSlot {
                width,
                height,
                context: RenderContext::new_with(width, height, self.settings),
                pixmap: Pixmap::new(width, height),
            });
        }
        let slot = self
            .scratch
            .last_mut()
            .expect("a scratch slot was just ensured");
        slot.context.reset();
        slot
    }

    /// Rasterizes the window-coordinate `list` clipped to `region`,
    /// returning a `region.width × region.height` premultiplied-RGBA8
    /// pixmap.
    ///
    /// The pixmap borrows the painter's reusable scratch slot for this
    /// region size and is valid until the next `rasterize_region` call —
    /// callers stream it out immediately, which is also the only usage the
    /// banded flush model permits.
    ///
    /// `candidates` are indices into `list.commands()` that a spatial index
    /// has already established *may* touch this region's band row; the
    /// painter still tests each one against the exact region. Passing every
    /// index is correct but reduces the pass to the quadratic scan the index
    /// exists to avoid.
    ///
    /// # Panics
    ///
    /// Panics when the region exceeds `u16::MAX` in either dimension, far
    /// beyond any target panel, or when a candidate index is out of range.
    #[must_use]
    pub fn rasterize_region(
        &mut self,
        list: &DisplayList,
        region: DeviceRegion,
        candidates: &[u32],
        work: &mut FrameWork,
    ) -> &Pixmap {
        let width = u16::try_from(region.width).expect("region width exceeds u16::MAX");
        let height = u16::try_from(region.height).expect("region height exceeds u16::MAX");
        // Ensure the slot exists and is reset, then split borrows so the
        // brush cache and the slot can be used simultaneously.
        let _ = self.scratch_slot(width, height);
        let Self {
            resources,
            images,
            scratch,
            ..
        } = self;
        let slot = scratch
            .last_mut()
            .expect("scratch_slot just ensured a slot");
        let ctx = &mut slot.context;
        let shift = Affine::translate((-f64::from(region.x), -f64::from(region.y)));
        let region_bounds = Rect::new(
            f64::from(region.x),
            f64::from(region.y),
            f64::from(region.x + region.width),
            f64::from(region.y + region.height),
        );
        let commands = list.commands();
        work.command_band_visits += candidates.len() as u64;
        work.pixels_rasterized += region.area();
        for index in candidates {
            let placed = &commands[usize::try_from(*index)
                .expect("display-list command index must fit a pointer-sized value")];
            if !placed.intersects(region_bounds) {
                continue;
            }
            work.command_band_draws += 1;
            let command = placed.command();
            let Some(clip_depth) = push_clip_layers(ctx, command.clip(), shift) else {
                continue;
            };
            match command {
                DrawCommand::FillPath {
                    path,
                    transform,
                    brush,
                    ..
                } => {
                    ctx.set_transform(shift * *transform);
                    set_brush(images, ctx, brush);
                    ctx.fill_path(path);
                }
                DrawCommand::StrokePath {
                    path,
                    transform,
                    stroke,
                    brush,
                    ..
                } => {
                    ctx.set_transform(shift * *transform);
                    ctx.set_stroke(stroke.clone());
                    set_brush(images, ctx, brush);
                    ctx.stroke_path(path);
                }
                DrawCommand::GlyphRun {
                    font,
                    font_size,
                    glyphs,
                    glyph_bounds,
                    transform,
                    brush,
                    ..
                } => {
                    ctx.set_transform(shift * *transform);
                    set_brush(images, ctx, brush);
                    ctx.glyph_run(resources, font)
                        .font_size(*font_size)
                        .hint(true)
                        .fill_glyphs(
                            glyphs
                                .iter()
                                .zip(glyph_bounds.iter())
                                .filter(|(_, bounds)| {
                                    transform
                                        .transform_rect_bbox(**bounds)
                                        .intersect(region_bounds)
                                        .area()
                                        > 0.0
                                })
                                .map(|(glyph, _)| *glyph),
                        );
                }
                DrawCommand::Scene {
                    recording,
                    transform,
                    ..
                } => {
                    replay_scene(ctx, resources, images, recording, shift * *transform);
                }
            }
            for _ in 0..clip_depth {
                ctx.pop_clip_path();
            }
        }
        ctx.flush();
        slot.context.render_to_pixmap(resources, &mut slot.pixmap);
        &slot.pixmap
    }
}

/// Pushes every clip layer in force for a command, returning how many were
/// pushed, or [`None`] when the clip admits no pixel at all.
///
/// Clips are in window coordinates, so they only need the region shift, not
/// the command transform. Each region becomes one layer: their intersection is
/// the mask, and the rasterizer computes it exactly rather than approximating
/// it with a boolean path operation.
fn push_clip_layers(ctx: &mut RenderContext, clip: Option<&Clip>, shift: Affine) -> Option<usize> {
    let Some(clip) = clip else {
        return Some(0);
    };
    let bounds = clip.bounds();
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return None;
    }
    ctx.set_transform(shift);
    for region in clip.regions() {
        match region {
            ClipRegion::Rect(rect) => ctx.push_clip_path(&rect.to_path(BEZIER_TOLERANCE)),
            ClipRegion::Shape { path, .. } => ctx.push_clip_path(path),
        }
    }
    Some(clip.regions().len())
}

/// Sets the context paint, converting and caching image brushes once.
fn set_brush(images: &mut Vec<CachedImage>, ctx: &mut RenderContext, brush: &peniko::Brush) {
    match brush {
        peniko::Brush::Solid(color) => ctx.set_paint(*color),
        peniko::Brush::Gradient(gradient) => ctx.set_paint(gradient.clone()),
        peniko::Brush::Image(image) => {
            let cached = images
                .iter()
                .find(|cached| cached.data == image.image)
                .map(|cached| cached.source.clone());
            let source = cached.unwrap_or_else(|| {
                let source = ImageSource::from_peniko_image_data(&image.image);
                images.push(CachedImage {
                    data: image.image.clone(),
                    source: source.clone(),
                });
                source
            });
            ctx.set_paint(Image {
                image: source,
                sampler: image.sampler,
            });
        }
    }
}

/// Replays a scene recording into the rasterizer, positioned by `transform`.
///
/// The recording is opaque to the display list, so the whole of it is replayed
/// whenever a band it touches is rasterized — exactly like every other
/// command. The context's drawing state is saved and restored around the
/// replay because a scene sets fill rules, strokes and paint transforms that
/// no other command sets, and a later command inheriting them would be painted
/// wrong.
///
/// # Panics
///
/// Panics when the recording leaves layers unpopped. Scene content owns its
/// layer stack, and an unbalanced one would corrupt every command rasterized
/// after it in the same band.
fn replay_scene(
    ctx: &mut RenderContext,
    resources: &mut Resources,
    images: &mut Vec<CachedImage>,
    recording: &SceneRecording,
    transform: Affine,
) {
    let saved = ctx.save_current_state();
    let depth = {
        let mut scene = CpuScene {
            ctx,
            resources,
            images,
            depth: 0,
        };
        recording.replay(&mut scene, Some(transform));
        scene.depth
    };
    assert_eq!(
        depth, 0,
        "dew scene content left {depth} layer(s) unpopped: every push must have a matching pop"
    );
    ctx.restore_state(saved);
}

/// Dew's [`Scene2D`]: engine-neutral scene commands painted directly into the
/// `vello_cpu` rasterizer.
///
/// This is the whole of dew's scene support. Everything a scene can express —
/// filled and stroked paths, gradient and image brushes, clip and compositing
/// layers, glyph runs — maps onto a rasterizer primitive, so content written
/// against the contract draws here exactly as it draws on a GPU engine.
struct CpuScene<'a> {
    ctx: &'a mut RenderContext,
    resources: &'a mut Resources,
    images: &'a mut Vec<CachedImage>,
    /// Layers pushed and not yet popped, checked at the end of a replay.
    depth: usize,
}

impl CpuScene<'_> {
    /// Sets the paint for the next draw, including the brush-relative
    /// transform that positions a gradient independently of its shape.
    fn paint(&mut self, brush: &Brush, brush_transform: Option<Affine>) {
        set_brush(self.images, self.ctx, brush);
        // `vello_cpu` encodes a paint against `transform * paint_transform`,
        // which is exactly what `Scene2D` means by a brush transform.
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
            "dew scene content popped a layer it never pushed"
        );
        self.depth -= 1;
        self.ctx.pop_layer();
    }

    fn draw_image(&mut self, image: &ImageBrush, transform: Affine) {
        // An image is a rectangle of the image's own pixel size painted with
        // the image as its brush — the lowering every `Scene2D` engine uses.
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
        // The style has to be in force before the builder borrows the context.
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
        // `reset` clears a recording's commands; this scene has none, because
        // it paints each command into the rasterizer as it arrives. Content
        // that resets mid-draw would be asking for pixels already rasterized
        // to be taken back, which no immediate-mode target can do.
        panic!("dew paints scene commands as they arrive and has no recording to reset");
    }
}

/// Render settings for this target.
///
/// The Xtensa LLVM backend currently miscompiles `vello_cpu`'s u8/u16 fine
/// kernels regardless of opt-level (corrupted strip indices surfacing as
/// `bytemuck` cast panics or `LoadProhibited` faults), so ESP32-S3 builds use
/// the f32 pipeline, which also maps onto the chip's hardware FPU. All
/// other targets keep the faster u8 pipeline.
pub(crate) fn target_render_settings() -> RenderSettings {
    RenderSettings {
        render_mode: if cfg!(target_arch = "xtensa") {
            RenderMode::OptimizeQuality
        } else {
            RenderMode::OptimizeSpeed
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::BandScheduler;
    use kurbo::Rect;
    use peniko::{Color, ImageAlphaType, ImageBrush, ImageData, ImageFormat};

    /// Every command index, i.e. the un-indexed scan the band index replaces.
    fn all_candidates(list: &DisplayList) -> Vec<u32> {
        (0..u32::try_from(list.commands().len()).expect("test scenes stay small")).collect()
    }

    fn rasterize(list: &DisplayList, region: DeviceRegion) -> Pixmap {
        rasterize_with(&mut Painter::default(), list, region)
    }

    fn rasterize_with(painter: &mut Painter, list: &DisplayList, region: DeviceRegion) -> Pixmap {
        let mut work = FrameWork::ZERO;
        painter
            .rasterize_region(list, region, &all_candidates(list), &mut work)
            .clone()
    }

    fn checker_scene() -> DisplayList {
        let mut list = DisplayList::new();
        list.fill(
            &Rect::new(0.0, 0.0, 64.0, 64.0),
            Affine::IDENTITY,
            Color::from_rgb8(20, 40, 80),
        );
        list.fill(
            &Rect::new(8.5, 8.5, 31.5, 31.5),
            Affine::IDENTITY,
            Color::from_rgb8(220, 60, 40),
        );
        list.fill(
            &kurbo::Circle::new((44.0, 44.0), 14.0),
            Affine::IDENTITY,
            Color::from_rgb8(60, 200, 120),
        );
        list
    }

    fn image_scene() -> DisplayList {
        let image = ImageData {
            data: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ]
            .into(),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        };
        let mut list = DisplayList::new();
        list.fill(
            &Rect::new(0.0, 0.0, 2.0, 2.0),
            Affine::IDENTITY,
            peniko::Brush::Image(ImageBrush::new(image)),
        );
        list
    }

    #[test]
    fn full_region_renders_expected_colors() {
        let pixmap = rasterize(
            &checker_scene(),
            DeviceRegion {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );
        let pixel = |x: usize, y: usize| {
            let i = (y * 64 + x) * 4;
            let data = pixmap.data_as_u8_slice();
            [data[i], data[i + 1], data[i + 2], data[i + 3]]
        };
        assert_eq!(pixel(2, 2), [20, 40, 80, 255]);
        assert_eq!(pixel(16, 16), [220, 60, 40, 255]);
        assert_eq!(pixel(44, 44), [60, 200, 120, 255]);
    }

    /// A retained clip must mask fills during rasterization: pixels inside
    /// the clip render, pixels outside stay untouched.
    #[test]
    fn clipped_command_renders_only_inside_the_clip() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(0.0, 0.0, 32.0, 32.0));
        list.fill(
            &Rect::new(0.0, 0.0, 64.0, 64.0),
            Affine::IDENTITY,
            Color::from_rgb8(220, 60, 40),
        );
        list.pop_clip();
        let pixmap = rasterize(
            &list,
            DeviceRegion {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );
        let pixel = |x: usize, y: usize| {
            let i = (y * 64 + x) * 4;
            let data = pixmap.data_as_u8_slice();
            [data[i], data[i + 1], data[i + 2], data[i + 3]]
        };
        assert_eq!(pixel(16, 16), [220, 60, 40, 255]);
        assert_eq!(pixel(48, 16), [0, 0, 0, 0]);
        assert_eq!(pixel(16, 48), [0, 0, 0, 0]);
    }

    /// Band-by-band rendering must be byte-identical to rendering the same
    /// area in one pass — otherwise band seams would be visible.
    #[test]
    fn banded_render_matches_single_pass() {
        let list = checker_scene();
        let full = rasterize(
            &list,
            DeviceRegion {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );
        let scheduler = BandScheduler::new(64, 64, 16);
        let full_data = full.data_as_u8_slice();
        // One painter across all bands: same-size bands share one reset
        // scratch slot, so this also proves reuse renders like fresh state.
        let mut painter = Painter::default();
        for band in scheduler.schedule(&[Rect::new(0.0, 0.0, 64.0, 64.0)]) {
            let pixmap = rasterize_with(&mut painter, &list, band);
            let band_data = pixmap.data_as_u8_slice();
            for row in 0..band.height as usize {
                let band_row =
                    &band_data[row * band.width as usize * 4..(row + 1) * band.width as usize * 4];
                let full_start = ((band.y as usize + row) * 64 + band.x as usize) * 4;
                let full_row = &full_data[full_start..full_start + band.width as usize * 4];
                assert_eq!(band_row, full_row, "band seam mismatch at row {row}");
            }
        }
    }

    #[test]
    fn image_brush_matches_across_bands_and_is_converted_once() {
        let list = image_scene();
        let full = rasterize(
            &list,
            DeviceRegion {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
        );
        let mut painter = Painter::default();
        let top = rasterize_with(
            &mut painter,
            &list,
            DeviceRegion {
                x: 0,
                y: 0,
                width: 2,
                height: 1,
            },
        );
        let bottom = rasterize_with(
            &mut painter,
            &list,
            DeviceRegion {
                x: 0,
                y: 1,
                width: 2,
                height: 1,
            },
        );

        assert_eq!(
            [top.data_as_u8_slice(), bottom.data_as_u8_slice()].concat(),
            full.data_as_u8_slice()
        );
        assert_eq!(painter.images.len(), 1);
        assert_eq!(
            full.data_as_u8_slice(),
            [
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ]
        );
    }

    /// An opaque backdrop with a half-transparent scene layer over it: the
    /// layer is a real compositing layer, so the pixels under it are the blend
    /// of the two, not either one alone.
    fn layered_scene() -> DisplayList {
        let mut recording = SceneRecording::new();
        let cover = Rect::new(0.0, 0.0, 64.0, 64.0).to_path(BEZIER_TOLERANCE);
        recording.push_layer(
            Fill::NonZero,
            BlendMode::default(),
            0.5,
            Affine::IDENTITY,
            &cover,
        );
        recording.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &Color::from_rgb8(255, 255, 255).into(),
            None,
            &Rect::new(16.0, 16.0, 48.0, 48.0).to_path(BEZIER_TOLERANCE),
        );
        recording.pop_layer();

        let mut list = DisplayList::new();
        list.fill(
            &Rect::new(0.0, 0.0, 64.0, 64.0),
            Affine::IDENTITY,
            Color::from_rgb8(0, 0, 0),
        );
        list.push(DrawCommand::Scene {
            recording: std::sync::Arc::new(recording),
            transform: Affine::IDENTITY,
            bounds: Rect::new(0.0, 0.0, 64.0, 64.0),
            clip: None,
        });
        list
    }

    /// A scene's compositing layer is honoured, not flattened into a per-shape
    /// alpha — a white square at half opacity over black is mid grey.
    #[test]
    fn a_scene_layer_composites_its_opacity() {
        let pixmap = rasterize(
            &layered_scene(),
            DeviceRegion {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );
        let data = pixmap.data_as_u8_slice();
        let pixel = |x: usize, y: usize| {
            let i = (y * 64 + x) * 4;
            [data[i], data[i + 1], data[i + 2], data[i + 3]]
        };
        assert_eq!(pixel(4, 4), [0, 0, 0, 255], "outside the layer stays black");
        let inside = pixel(32, 32);
        assert_eq!(inside[3], 255);
        assert!(
            (120..=136).contains(&inside[0]),
            "half-opacity white over black is mid grey, got {inside:?}"
        );
    }

    /// A scene rasterized band by band must match a single pass exactly:
    /// its layers composite within each band over the same backdrop, so no
    /// seam may appear where a band boundary crosses one.
    #[test]
    fn banded_scene_render_matches_single_pass() {
        let list = layered_scene();
        let full = rasterize(
            &list,
            DeviceRegion {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );
        let full_data = full.data_as_u8_slice();
        let scheduler = BandScheduler::new(64, 64, 16);
        let mut painter = Painter::default();
        for band in scheduler.schedule(&[Rect::new(0.0, 0.0, 64.0, 64.0)]) {
            let pixmap = rasterize_with(&mut painter, &list, band);
            let band_data = pixmap.data_as_u8_slice();
            for row in 0..band.height as usize {
                let band_row =
                    &band_data[row * band.width as usize * 4..(row + 1) * band.width as usize * 4];
                let full_start = ((band.y as usize + row) * 64 + band.x as usize) * 4;
                let full_row = &full_data[full_start..full_start + band.width as usize * 4];
                assert_eq!(band_row, full_row, "scene band seam mismatch at row {row}");
            }
        }
    }

    /// A scene must not leak its drawing state into the commands rasterized
    /// after it: the even-odd rule it leaves behind would punch a hole in the
    /// next fill, which is drawn from a self-overlapping path here precisely so
    /// the two rules disagree.
    #[test]
    fn a_scene_does_not_leak_its_state_into_later_commands() {
        let mut recording = SceneRecording::new();
        recording.fill(
            Fill::EvenOdd,
            Affine::IDENTITY,
            &Color::from_rgb8(10, 10, 10).into(),
            None,
            &Rect::new(0.0, 0.0, 4.0, 4.0).to_path(BEZIER_TOLERANCE),
        );

        let mut overlapping = Rect::new(0.0, 0.0, 32.0, 32.0).to_path(BEZIER_TOLERANCE);
        overlapping.extend(Rect::new(8.0, 8.0, 24.0, 24.0).to_path(BEZIER_TOLERANCE));

        let mut list = DisplayList::new();
        list.push(DrawCommand::Scene {
            recording: std::sync::Arc::new(recording),
            transform: Affine::IDENTITY,
            bounds: Rect::new(0.0, 0.0, 4.0, 4.0),
            clip: None,
        });
        list.fill(
            &overlapping,
            Affine::IDENTITY,
            Color::from_rgb8(200, 60, 40),
        );

        let pixmap = rasterize(
            &list,
            DeviceRegion {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            },
        );
        let data = pixmap.data_as_u8_slice();
        let index = (16 * 32 + 16) * 4;
        assert_eq!(
            [
                data[index],
                data[index + 1],
                data[index + 2],
                data[index + 3]
            ],
            [200, 60, 40, 255],
            "the fill after a scene is painted non-zero, as every dew fill is"
        );
    }
}
