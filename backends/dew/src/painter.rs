//! The `vello_cpu` bridge: rasterizes a display list into a region-sized
//! scratch pixmap.
//!
//! This is the only module that touches `vello_cpu` directly; its 0.0.x API
//! is expected to change and the churn must stay contained here. Region
//! rendering works by translating every command by the region origin and
//! rasterizing into a context exactly the size of the region — sparse-strip
//! rasterization only pays for covered pixels, so this is cheap even though
//! the full scene is replayed.

use kurbo::{Affine, Rect, Shape};
use vello_cpu::{Image, ImageSource, Pixmap, RenderContext, RenderMode, RenderSettings, Resources};

use crate::compositor::DeviceRegion;
use crate::display_list::{BEZIER_TOLERANCE, DisplayList, DrawCommand};
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
}

#[derive(Debug)]
struct CachedImage {
    data: peniko::ImageData,
    source: ImageSource,
}

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
        }
    }

    /// Rasterizes the window-coordinate `list` clipped to `region`,
    /// returning a `region.width × region.height` premultiplied-RGBA8
    /// pixmap.
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
    ) -> Pixmap {
        let width = u16::try_from(region.width).expect("region width exceeds u16::MAX");
        let height = u16::try_from(region.height).expect("region height exceeds u16::MAX");
        let mut ctx = RenderContext::new_with(width, height, self.settings);
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
            let clip = command.clip();
            if let Some(clip) = clip {
                if clip.width() <= 0.0 || clip.height() <= 0.0 {
                    continue;
                }
                // The clip rectangle is in window coordinates, so it only
                // needs the region shift, not the command transform.
                ctx.set_transform(shift);
                ctx.push_clip_path(&clip.to_path(BEZIER_TOLERANCE));
            }
            match command {
                DrawCommand::FillPath {
                    path,
                    transform,
                    brush,
                    ..
                } => {
                    ctx.set_transform(shift * *transform);
                    self.set_brush(&mut ctx, brush);
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
                    self.set_brush(&mut ctx, brush);
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
                    self.set_brush(&mut ctx, brush);
                    ctx.glyph_run(&mut self.resources, font)
                        .font_size(*font_size)
                        .hint(true)
                        .fill_glyphs(
                            glyphs
                                .iter()
                                .zip(glyph_bounds)
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
            }
            if clip.is_some() {
                ctx.pop_clip_path();
            }
        }
        ctx.flush();
        let mut pixmap = Pixmap::new(width, height);
        ctx.render_to_pixmap(&mut self.resources, &mut pixmap);
        pixmap
    }

    fn set_brush(&mut self, ctx: &mut RenderContext, brush: &peniko::Brush) {
        match brush {
            peniko::Brush::Solid(color) => ctx.set_paint(*color),
            peniko::Brush::Gradient(gradient) => ctx.set_paint(gradient.clone()),
            peniko::Brush::Image(image) => {
                let cached = self
                    .images
                    .iter()
                    .find(|cached| cached.data == image.image)
                    .map(|cached| cached.source.clone());
                let source = if let Some(cached) = cached {
                    cached
                } else {
                    let source = ImageSource::from_peniko_image_data(&image.image);
                    self.images.push(CachedImage {
                        data: image.image.clone(),
                        source: source.clone(),
                    });
                    source
                };
                ctx.set_paint(Image {
                    image: source,
                    sampler: image.sampler,
                });
            }
        }
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
        painter.rasterize_region(list, region, &all_candidates(list), &mut work)
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
        for band in scheduler.schedule(&[Rect::new(0.0, 0.0, 64.0, 64.0)]) {
            let pixmap = rasterize(&list, band);
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
}
