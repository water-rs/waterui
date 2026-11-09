//! `vello-hybrid` adapter: `vello_gpu` (CPU strip rasterizer + GPU
//! colour pass) on `wgpu`.
//!
//! Route: strips and coverage are computed on the CPU into GPU-accessible
//! geometry, then a wgpu colour pass writes into the target texture (format
//! chosen from queried adapter capabilities). GPU time is unavailable:
//! the renderer does not expose timestamps inside its render passes.
//! Independent queue markers would include CPU submission gaps.

use std::collections::BTreeSet;
use std::time::Instant;

use cherenkov_scene::Feature;
use kurbo::{Affine, BezPath, Rect, Stroke};
use peniko::{BlendMode as PBlendMode, Fill, FontData, ImageBrush};
use vello_common::paint::{ImageSource, PaintType};
use vello_gpu::{
    RenderSize, RenderTargetConfig, Renderer, Resources, Scene as HybridScene, TargetInit,
    TextureBindings,
};

use crate::convert::{self, Prepared};
use crate::memory::{
    AdapterMemory, Reading, allocator_report, wgpu_metal_allocated_bytes, wgpu_vk_memory_budget,
};
use crate::vello_like::{
    Lowered, Op, VelloLikeCtx, lower, replay, vello_features, vello_missing_api,
};
use crate::wgpu_ctx::{Gpu, Target, readback};
use crate::{BenchError, Counters, DeviceInfo, EncodeInput, Engine, EngineInfo, GpuSample, Submit};

/// `vello_gpu` adapter (the `vello-hybrid` baseline).
pub struct VelloHybrid {
    info: EngineInfo,
    gpu: Gpu,
    renderer: Option<Renderer>,
    renderer_config: Option<RenderTargetConfig>,
    res: Option<Resources>,
    scene: Option<HybridScene>,
    target: Option<Target>,
    depth_view: Option<wgpu::TextureView>,
    prepared: Option<Prepared>,
    lowered: Option<Lowered<PaintType>>,
    /// Fonts whose runs draw bitmap glyphs, from `bitmap_glyph_fonts`.
    bitmap_fonts: BitmapFonts,
    counters: Counters,
    /// Whether a submitted frame awaits [`Engine::completion`].
    awaiting_completion: bool,
}

/// The GPU and renderer state do not format.
impl std::fmt::Debug for VelloHybrid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VelloHybrid")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl VelloHybrid {
    /// Adapter key.
    pub const NAME: &'static str = "vello-hybrid";

    /// Creates the adapter, initializing wgpu.
    ///
    /// # Errors
    /// [`BenchError::Gpu`] when no adapter exists.
    pub fn new() -> Result<Self, BenchError> {
        let gpu = Gpu::new(wgpu::Features::empty())?;
        Ok(Self {
            info: EngineInfo {
                name: Self::NAME,
                engine_crate: "vello_gpu",
                crate_version: env!("DEP_VELLO_GPU_VERSION"),
                source_rev: option_env!("DEP_VELLO_GPU_SOURCE_REV").map(String::from),
                output_format: format!("wgpu {:?} texture", gpu.target_format),
                precision: "vello_common strip rasterizer in f32; GPU colour pass writes rgba8unorm",
                route: "hybrid (cpu strips + gpu colour pass)",
                color_note: "peniko brushes (sRGB-tagged); output quantized to rgba8unorm sRGB; \
                             readback decoded sRGB→linear P3",
                encode_scope: "records `vello_gpu::Scene` calls (set_paint/fill_path/\
                               push_layer/glyph_run) against fonts and atlas-resident images \
                               prepared once",
            },
            gpu,
            renderer: None,
            renderer_config: None,
            res: None,
            scene: None,
            target: None,
            depth_view: None,
            prepared: None,
            lowered: None,
            bitmap_fonts: BitmapFonts::new(),
            counters: Counters::default(),
            awaiting_completion: false,
        })
    }
}

/// The `VelloLikeCtx` view over a `vello_gpu::Scene` + its `Resources`.
///
/// `vello_gpu` 0.3 renders images only through
/// [`ImageSource::OpaqueId`] — inline [`ImageSource::Pixmap`] paints panic
/// inside the GPU paint encoder by design (`vello_gpu::Scene::set_paint`
/// documents pixmap sources as out of scope). The adapter therefore uploads
/// every image to the atlas via [`Renderer::upload_image`] once in
/// `prepare`, and [`Prepared::paint_type`] already resolves image paints
/// to their `OpaqueId` — `set_paint` needs no interception.
///
/// Bitmap glyphs (CBDT, sbix) follow the same rule: `glifo` draws one
/// through `vello_gpu`'s glyph atlas, an `OpaqueId` source, only when the
/// run enables the atlas cache; otherwise it paints the decoded strike as
/// an inline pixmap. Runs in a font that draws bitmap glyphs therefore
/// enable the atlas, and `prepare` reports the bitmap glyphs the atlas
/// cannot take as unsupported (`bitmap_glyph_fonts`). Other runs keep
/// the uncached path.
pub struct HybridCtx<'a> {
    scene: &'a mut HybridScene,
    res: &'a mut Resources,
    bitmap_fonts: &'a BitmapFonts,
}

/// The borrowed scene and resources do not format.
impl std::fmt::Debug for HybridCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HybridCtx").finish_non_exhaustive()
    }
}

impl VelloLikeCtx for HybridCtx<'_> {
    fn set_transform(&mut self, t: Affine) {
        self.scene.set_transform(t);
    }

    fn set_paint(&mut self, p: PaintType) {
        self.scene.set_paint(p);
    }

    fn set_paint_transform(&mut self, t: Affine) {
        self.scene.set_paint_transform(t);
    }

    fn reset_paint_transform(&mut self) {
        self.scene.reset_paint_transform();
    }

    fn set_fill_rule(&mut self, f: Fill) {
        self.scene.set_fill_rule(f);
    }

    fn set_stroke(&mut self, s: Stroke) {
        self.scene.set_stroke(s);
    }

    fn fill_path(&mut self, p: &BezPath) {
        self.scene.fill_path(p);
    }

    fn stroke_path(&mut self, p: &BezPath) {
        self.scene.stroke_path(p);
    }

    fn fill_rect(&mut self, r: &Rect) {
        self.scene.fill_rect(r);
    }

    fn push_layer(
        &mut self,
        clip: Option<&BezPath>,
        blend: Option<PBlendMode>,
        opacity: Option<f32>,
    ) {
        self.scene.push_layer(clip, blend, opacity, None, None);
    }

    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }

    fn fill_blurred_rrect(&mut self, rect: &Rect, radius: f32, std_dev: f32) {
        self.scene
            .fill_blurred_rounded_rect(rect, radius, std_dev, false);
    }

    fn draw_glyphs_fill(
        &mut self,
        font: &FontData,
        size: f32,
        coords: &[i16],
        glyphs: Vec<cherenkov_scene::Glyph>,
    ) {
        self.scene
            .glyph_run(self.res, font)
            .atlas_cache(self.bitmap_fonts.contains(&(font.data.id(), font.index)))
            .font_size(size)
            .normalized_coords(coords)
            .fill_glyphs(glyphs.iter().map(|g| glifo::Glyph {
                id: g.id,
                x: g.x,
                y: g.y,
            }))
            .expect("vello_gpu glyph run renders");
    }
}

/// Fonts, as (`FontData` blob id, collection index), whose runs draw
/// bitmap glyphs through `vello_gpu`'s glyph atlas.
type BitmapFonts = BTreeSet<(u64, u32)>;

/// The `vello_gpu` limit a `bitmap_glyph_fonts` rejection names.
const BITMAP_GLYPH_API: &str = "vello_gpu draws bitmap glyphs only from its glyph atlas, which \
                                takes no rotation, skew or mirroring and no size above \
                                GlyphCacheConfig::max_cached_font_size";

/// The fonts whose runs draw bitmap glyphs, which `encode` draws through
/// the glyph atlas.
///
/// `glifo` resolves each glyph COLR first, then a PNG bitmap strike at the
/// draw size, then the outline. A bitmap glyph reaches `vello_gpu` through
/// the glyph atlas only when the run's transform has no rotation or skew
/// and positive axis scales, and its draw size — the run size times a
/// positive uniform scale, which `glifo` absorbs — is within the default
/// [`glifo::GlyphCacheConfig`] that `vello_gpu::Resources` uses. Any other
/// bitmap glyph is painted as an inline pixmap, which `vello_gpu`'s paint
/// encoder rejects by panicking.
///
/// # Errors
/// [`BenchError::Unsupported`] for a run with a bitmap glyph the atlas
/// cannot take; [`BenchError::Engine`] for a font `skrifa` cannot read.
fn bitmap_glyph_fonts(lowered: &Lowered<PaintType>) -> Result<BitmapFonts, BenchError> {
    use skrifa::MetadataProvider;
    use skrifa::bitmap::BitmapData;
    use skrifa::instance::Size;

    // `glifo`'s tolerance for a zero transform coefficient.
    const NEARLY_ZERO: f64 = 1.0 / 4096.0;
    let max_cached_size = glifo::GlyphCacheConfig::default().max_cached_font_size;
    let mut fonts = BitmapFonts::new();
    for op in &lowered.ops {
        let Op::Glyphs {
            transform,
            font,
            size,
            glyphs,
            ..
        } = op
        else {
            continue;
        };
        let [a, b, c, d, _, _] = transform.as_coeffs();
        let axis_aligned = b.abs() <= NEARLY_ZERO && c.abs() <= NEARLY_ZERO && a > 0.0 && d > 0.0;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "glifo absorbs the scale into its f32 font size the same way"
        )]
        let draw_size = if axis_aligned && (a - d).abs() <= NEARLY_ZERO {
            size * d as f32
        } else {
            *size
        };
        let font_ref = skrifa::FontRef::from_index(font.data.data(), font.index)
            .map_err(|e| BenchError::Engine(format!("vello-hybrid: unreadable font: {e}")))?;
        let colr = font_ref.color_glyphs();
        let strikes = font_ref.bitmap_strikes();
        let draws_bitmap = glyphs.iter().any(|g| {
            let id = skrifa::GlyphId::new(g.id);
            colr.get(id).is_none()
                && strikes
                    .glyph_for_size(Size::new(draw_size), id)
                    .is_some_and(|bitmap| matches!(bitmap.data, BitmapData::Png(_)))
        });
        if !draws_bitmap {
            continue;
        }
        if !axis_aligned || draw_size > max_cached_size {
            return Err(BenchError::Unsupported {
                engine: VelloHybrid::NAME,
                feature: Feature::Glyphs,
                api: Some(BITMAP_GLYPH_API),
            });
        }
        fonts.insert((font.data.id(), font.index));
    }
    Ok(fonts)
}

impl Engine for VelloHybrid {
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
            vello_missing_api,
        )?;
        let dim = |v: u32| {
            u16::try_from(v)
                .map_err(|_| BenchError::Engine("vello-hybrid: scene exceeds u16 dims".into()))
        };
        let (width, height) = (dim(input.scene.width)?, dim(input.scene.height)?);
        let config = RenderTargetConfig {
            format: self.gpu.target_format,
            width,
            height,
        };
        // The renderer is built for one target size and format —
        // recreate it when they change rather than reusing the first
        // scene's config.
        if self.renderer_config.as_ref().is_none_or(|c| {
            c.format != config.format || c.width != config.width || c.height != config.height
        }) {
            self.renderer = None;
            self.res = None;
        }
        if self.renderer.is_none() {
            let (renderer, res) = Renderer::new(&self.gpu.device, &config);
            self.renderer = Some(renderer);
            self.res = Some(res);
            self.renderer_config = Some(config);
        }
        let mut prepared = Prepared::build(input.scene, input.blobs)?;
        // Upload every image to the atlas once — `OpaqueId` paints resolve
        // against it when `encode` records and `submit` renders. The
        // uploads land in their own submission so the atlas is populated
        // before the first frame's render submission.
        let hashes: Vec<cherenkov_scene::ResourceHash> =
            prepared.image_entries().map(|(h, _)| h).collect();
        if !hashes.is_empty() {
            let mut encoder =
                self.gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("vello-hybrid image uploads"),
                    });
            for hash in hashes {
                let source = prepared.image_source(hash)?;
                let ImageSource::Pixmap(pixmap) = source else {
                    return Err(BenchError::Engine(
                        "vello-hybrid: image not decoded in prepare".into(),
                    ));
                };
                let id = self.renderer.as_mut().expect("created above").upload_image(
                    self.res.as_mut().expect("created above"),
                    &self.gpu.device,
                    &self.gpu.queue,
                    &mut encoder,
                    pixmap.as_ref(),
                );
                prepared.set_gpu_image(hash, id.as_u32(), pixmap.may_have_transparency());
            }
            self.gpu.queue.submit([encoder.finish()]);
        }
        // Lower after the atlas uploads so `image_fn` resolves OpaqueId
        // sources — `prepare` produces the pre-resolved op list `encode`
        // replays.
        let lowered = lower(
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
        )?;
        self.bitmap_fonts = bitmap_glyph_fonts(&lowered)?;
        self.lowered = Some(lowered);
        self.prepared = Some(prepared);
        // The depth buffer the 0.2 renderer owned internally is now
        // caller-owned; passing it keeps the front-to-back opaque pass.
        self.depth_view = Some(Renderer::create_depth_texture_view(
            &self.gpu.device,
            &RenderSize { width, height },
        ));
        self.target = Some(Target::new(
            &self.gpu.device,
            input.scene.width,
            input.scene.height,
            self.gpu.target_format,
        ));
        Ok(())
    }

    fn encode(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError> {
        self.counters = Counters::default();
        let mut scene = HybridScene::new(
            u16::try_from(input.scene.width)
                .map_err(|_| BenchError::Engine("vello-hybrid: scene exceeds u16 dims".into()))?,
            u16::try_from(input.scene.height)
                .map_err(|_| BenchError::Engine("vello-hybrid: scene exceeds u16 dims".into()))?,
        );
        scene.set_transform(Affine::IDENTITY);
        scene.set_fill_rule(Fill::NonZero);
        scene.set_paint(PaintType::Solid(crate::convert::peniko_solid(
            &input.scene.clear,
        )));
        scene.fill_rect(&Rect::new(
            0.0,
            0.0,
            f64::from(input.scene.width),
            f64::from(input.scene.height),
        ));
        self.counters.draw_commands += 1;
        self.scene = Some(scene);
        let mut ctx = HybridCtx {
            scene: self.scene.as_mut().expect("set above"),
            res: self.res.as_mut().expect("renderer created in prepare"),
            bitmap_fonts: &self.bitmap_fonts,
        };
        let lowered = self
            .lowered
            .as_ref()
            .ok_or_else(|| BenchError::Engine("vello-hybrid: encode before prepare".into()))?;
        replay(
            &mut ctx,
            lowered,
            &mut self.counters,
            input.scroll_transform(),
        );
        // `bytes_uploaded` counts the decoded texels uploaded to the
        // atlas in `prepare` — the same quantity vello-classic reports
        // for its texture uploads.
        self.counters.bytes_uploaded = self.prepared.as_ref().map(Prepared::texel_bytes);
        Ok(())
    }

    fn submit(&mut self, _frame: u64, readback_flag: bool) -> Result<Submit, BenchError> {
        let (Some(scene), Some(res), Some(renderer), Some(target), Some(depth_view), Some(config)) = (
            &self.scene,
            &mut self.res,
            &mut self.renderer,
            &self.target,
            &self.depth_view,
            &self.renderer_config,
        ) else {
            return Err(BenchError::Engine(
                "vello-hybrid: submit before encode".into(),
            ));
        };
        let render_at = Instant::now();
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vello-hybrid frame"),
            });
        renderer
            .render(
                scene,
                res,
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                &RenderSize {
                    width: config.width,
                    height: config.height,
                },
                &target.view,
                Some(depth_view),
                &TextureBindings::new(),
                // The scene's first draw is the clear-colour rect, over
                // the target as it stands — the 0.2 renderer's load-op.
                TargetInit::SrcOver,
            )
            .map_err(|e| BenchError::Gpu(format!("vello-hybrid render: {e}")))?;
        self.gpu.queue.submit([encoder.finish()]);
        let render_seconds = render_at.elapsed().as_secs_f64();
        let readback_at = Instant::now();
        let image = if readback_flag {
            Some(readback(&self.gpu, target)?)
        } else {
            None
        };
        self.awaiting_completion = true;
        Ok(Submit {
            image,
            gpu: Vec::new(),
            phases: Vec::new(),
            composition: None,
            gpu_wait_seconds: 0.0,
            render_seconds: Some(render_seconds),
            readback_seconds: readback_flag.then(|| readback_at.elapsed().as_secs_f64()),
        })
    }

    fn completion_signal(&self) -> crate::CompletionSignal {
        crate::CompletionSignal::Measured(crate::Completion::WGPU_SIGNAL)
    }

    fn completion(&mut self) -> Result<Option<crate::Completion>, BenchError> {
        if !std::mem::take(&mut self.awaiting_completion) {
            return Err(BenchError::Engine(format!(
                "{}: completion requested without a submitted frame",
                Self::NAME
            )));
        }
        // Nothing has been submitted on this queue since the frame's submit
        // returned, so its latest submission is the frame's.
        Ok(Some(crate::Completion::on_submitted_work_done(
            &self.gpu.queue,
        )))
    }

    fn gpu_timing_unavailable(&self) -> Option<&'static str> {
        Some("Vello hybrid does not expose render-pass GPU timestamps")
    }

    fn finish_gpu(&mut self) -> Result<Vec<GpuSample>, BenchError> {
        self.wait_idle()?;
        Ok(Vec::new())
    }

    fn wait_idle(&mut self) -> Result<(), BenchError> {
        crate::wgpu_ctx::wait(&self.gpu, None, "the Vello hybrid frames")
    }

    fn counters(&self) -> Counters {
        self.counters.clone()
    }

    fn device(&self) -> DeviceInfo {
        self.gpu.device_info()
    }

    fn memory(&self) -> AdapterMemory {
        AdapterMemory {
            engine: Reading::unavailable("Vello hybrid does not expose engine memory"),
            wgpu_allocator: allocator_report(&self.gpu.device, self.gpu.info.backend),
            skia_budgeted: Reading::unavailable("Vello hybrid has no Skia budget"),
            vk_memory_budget: wgpu_vk_memory_budget(
                &self.gpu.device,
                self.gpu.info.backend,
                &self.gpu.info.name,
            ),
            metal_allocated_bytes: wgpu_metal_allocated_bytes(
                &self.gpu.device,
                self.gpu.info.backend,
            ),
        }
    }
}
