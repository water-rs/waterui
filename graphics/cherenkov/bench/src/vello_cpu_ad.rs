//! `vello-cpu` adapter: `vello_cpu::RenderContext` CPU rasterizer.
//!
//! Colour: `vello_cpu` composites in premultiplied `f32` over an rgba8
//! pixmap; paints take `peniko` colours (sRGB-tagged). Readback is the
//! pixmap's premultiplied rgba8, decoded to the suite working space.

use std::collections::BTreeSet;
use std::time::Instant;

use cherenkov_scene::Feature;
use vello_common::paint::PaintType;
use vello_cpu::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_cpu::peniko::{BlendMode as PBlendMode, Fill, FontData, ImageBrush};
use vello_cpu::{Pixmap, PixmapMut, RenderContext, Resources};

use crate::convert::{self, Prepared};
use crate::memory::{AdapterMemory, Reading};
use crate::vello_like::{Lowered, VelloLikeCtx, lower, replay, vello_features};
use crate::{
    BenchError, Counters, DeviceInfo, EncodeInput, Engine, EngineInfo, Submit, cpu_model,
    thermal_celsius,
};

/// `vello_cpu` adapter.
pub struct VelloCpu {
    info: EngineInfo,
    ctx: Option<RenderContext>,
    res: Resources,
    prepared: Option<Prepared>,
    lowered: Option<Lowered<PaintType>>,
    counters: Counters,
}

/// The `RenderContext` and prepared scene state do not format.
impl std::fmt::Debug for VelloCpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VelloCpu")
            .field("info", &self.info)
            .field("counters", &self.counters)
            .finish_non_exhaustive()
    }
}

impl VelloCpu {
    /// Adapter key.
    pub const NAME: &'static str = "vello-cpu";

    /// Creates the adapter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            info: EngineInfo {
                name: Self::NAME,
                engine_crate: "vello_cpu",
                crate_version: env!("DEP_VELLO_CPU_VERSION"),
                source_rev: option_env!("DEP_VELLO_CPU_SOURCE_REV").map(String::from),
                output_format: "premultiplied rgba8 Pixmap (straight sRGB quantization)".into(),
                precision: "f32 premultiplied pipeline (vello_cpu `f32_pipeline` feature)",
                route: "cpu-raster",
                color_note: "peniko brushes (sRGB-tagged); scene colours quantized to rgba8 sRGB; \
                             readback decoded sRGB→linear P3",
                encode_scope: "records `vello_cpu::RenderContext` calls (set_paint/fill_path/\
                               push_layer/glyph_run) against fonts/images prepared once",
            },
            ctx: None,
            res: Resources::new(),
            prepared: None,
            lowered: None,
            counters: Counters::default(),
        }
    }
}

impl Default for VelloCpu {
    fn default() -> Self {
        Self::new()
    }
}

impl VelloLikeCtx for VelloCpu {
    fn set_transform(&mut self, t: Affine) {
        self.ctx
            .as_mut()
            .expect("encode before submit")
            .set_transform(t);
    }

    fn set_paint(&mut self, p: PaintType) {
        self.ctx
            .as_mut()
            .expect("encode before submit")
            .set_paint(p);
    }

    fn set_paint_transform(&mut self, t: Affine) {
        self.ctx
            .as_mut()
            .expect("encode before submit")
            .set_paint_transform(t);
    }

    fn reset_paint_transform(&mut self) {
        self.ctx
            .as_mut()
            .expect("encode before submit")
            .reset_paint_transform();
    }

    fn set_fill_rule(&mut self, f: Fill) {
        self.ctx
            .as_mut()
            .expect("encode before submit")
            .set_fill_rule(f);
    }

    fn set_stroke(&mut self, s: Stroke) {
        self.ctx
            .as_mut()
            .expect("encode before submit")
            .set_stroke(s);
    }

    fn fill_path(&mut self, p: &BezPath) {
        self.ctx
            .as_mut()
            .expect("encode before submit")
            .fill_path(p);
    }

    fn stroke_path(&mut self, p: &BezPath) {
        self.ctx
            .as_mut()
            .expect("encode before submit")
            .stroke_path(p);
    }

    fn fill_rect(&mut self, r: &Rect) {
        self.ctx
            .as_mut()
            .expect("encode before submit")
            .fill_rect(r);
    }

    fn push_layer(
        &mut self,
        clip: Option<&BezPath>,
        blend: Option<PBlendMode>,
        opacity: Option<f32>,
    ) {
        self.ctx
            .as_mut()
            .expect("encode before submit")
            .push_layer(clip, blend, opacity, None, None);
    }

    fn pop_layer(&mut self) {
        self.ctx.as_mut().expect("encode before submit").pop_layer();
    }

    fn fill_blurred_rrect(&mut self, rect: &Rect, radius: f32, std_dev: f32) {
        self.ctx
            .as_mut()
            .expect("encode before submit")
            .fill_blurred_rounded_rect(rect, radius, std_dev, false);
    }

    fn draw_glyphs_fill(
        &mut self,
        font: &FontData,
        size: f32,
        coords: &[i16],
        glyphs: Vec<cherenkov_scene::Glyph>,
    ) {
        let ctx = self.ctx.as_mut().expect("encode before submit");
        ctx.glyph_run(&mut self.res, font)
            .font_size(size)
            .normalized_coords(coords)
            .fill_glyphs(glyphs.iter().map(|g| vello_cpu::Glyph {
                id: g.id,
                x: g.x,
                y: g.y,
            }));
    }
}

impl Engine for VelloCpu {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn supported(&self) -> BTreeSet<Feature> {
        vello_features().into_iter().collect()
    }

    fn prepare(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError> {
        crate::convert::check_features(
            Self::NAME,
            input.scene,
            &vello_features(),
            crate::vello_like::vello_missing_api,
        )?;
        let prepared = Prepared::build(input.scene, input.blobs)?;
        self.lowered = Some(lower(
            input.scene,
            &prepared,
            Self::NAME,
            false,
            &|p| prepared.paint_type(Self::NAME, p),
            &|hash, sampling| {
                Ok(PaintType::Image(ImageBrush {
                    image: prepared.image_source(hash)?,
                    sampler: convert::image_sampler(sampling),
                }))
            },
        )?);
        self.prepared = Some(prepared);
        Ok(())
    }

    fn encode(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError> {
        self.counters = Counters::default();
        let mut ctx = RenderContext::new(
            u16::try_from(input.scene.width)
                .map_err(|_| BenchError::Engine("vello-cpu: scene exceeds u16 dims".into()))?,
            u16::try_from(input.scene.height)
                .map_err(|_| BenchError::Engine("vello-cpu: scene exceeds u16 dims".into()))?,
        );
        // Clear to scene clear colour.
        ctx.set_paint(PaintType::Solid(crate::convert::peniko_solid(
            &input.scene.clear,
        )));
        ctx.set_transform(Affine::IDENTITY);
        ctx.set_fill_rule(Fill::NonZero);
        ctx.fill_rect(&Rect::new(
            0.0,
            0.0,
            f64::from(input.scene.width),
            f64::from(input.scene.height),
        ));
        self.ctx = Some(ctx);
        // `replay` needs `&mut self` (as the ctx), `&mut self.counters`
        // and `&self.lowered` — take both fields out for the call.
        let mut counters = std::mem::take(&mut self.counters);
        let lowered = self
            .lowered
            .take()
            .ok_or_else(|| BenchError::Engine("vello-cpu: encode before prepare".into()))?;
        replay(self, &lowered, &mut counters);
        self.lowered = Some(lowered);
        self.counters = counters;
        Ok(())
    }

    fn submit(&mut self, _frame: u64, readback: bool) -> Result<Submit, BenchError> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| BenchError::Engine("vello-cpu: submit before encode".into()))?;
        // Always rasterize — `measure` must execute the real render even
        // without readback (`ctx.flush()` alone only drains the dispatcher).
        let render_at = Instant::now();
        let mut pixmap = Pixmap::new(ctx.width(), ctx.height());
        ctx.render(PixmapMut::from(&mut pixmap), &mut self.res);
        let render_seconds = render_at.elapsed().as_secs_f64();
        let readback_at = Instant::now();
        let image = readback.then(|| {
            let rgba8: &[u8] = bytemuck::cast_slice(pixmap.data());
            crate::convert::rgba8_to_working(
                u32::from(pixmap.width()),
                u32::from(pixmap.height()),
                rgba8,
            )
        });
        Ok(Submit {
            image,
            gpu: Vec::new(),
            phases: None,
            render_seconds: Some(render_seconds),
            readback_seconds: readback.then(|| readback_at.elapsed().as_secs_f64()),
        })
    }

    fn counters(&self) -> Counters {
        self.counters.clone()
    }

    fn device(&self) -> DeviceInfo {
        DeviceInfo {
            cpu: cpu_model(),
            thermal_celsius: thermal_celsius(),
            ..Default::default()
        }
    }

    fn memory(&self) -> AdapterMemory {
        AdapterMemory {
            engine: Reading::unavailable("Vello CPU does not expose engine memory"),
            wgpu_allocator: Reading::unavailable("Vello CPU has no wgpu allocator"),
            skia_budgeted: Reading::unavailable("Vello CPU has no Skia budget"),
            vk_memory_budget: Reading::unavailable("Vello CPU has no Vulkan device"),
        }
    }
}
