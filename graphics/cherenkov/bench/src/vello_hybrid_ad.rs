//! `vello-hybrid` adapter: `vello_hybrid` (CPU strip rasterizer + GPU
//! colour pass) on `wgpu`.
//!
//! Route: strips and coverage are computed on the CPU into GPU-accessible
//! geometry, then a wgpu colour pass writes into the target texture (format
//! chosen from queried adapter capabilities). GPU time is a real `wgpu`
//! timestamp pair written in standalone submissions after a full queue
//! drain — never estimated, and serialized CPU+GPU by construction (see
//! [`crate::wgpu_ctx::drain_and_stamp`]).

use std::collections::BTreeSet;
use std::time::Instant;

use cherenkov_scene::Feature;
use kurbo::{Affine, BezPath, Rect, Stroke};
use peniko::{BlendMode as PBlendMode, Fill, FontData, ImageBrush};
use vello_common::paint::{ImageSource, PaintType};
use vello_hybrid::{
    RenderSize, RenderTargetConfig, Renderer, Resources, Scene as HybridScene, TextureBindings,
};

use crate::convert::{self, Prepared};
use crate::memory::{AdapterMemory, Reading, wgpu29_allocator, wgpu29_vk_memory_budget};
use crate::vello_like::{Lowered, VelloLikeCtx, lower, replay, vello_features, vello_missing_api};
use crate::wgpu_ctx::{Gpu, Target, drain_and_stamp, readback, resolve_timestamps};
use crate::{BenchError, Counters, DeviceInfo, EncodeInput, Engine, EngineInfo, GpuSample, Submit};

/// `vello_hybrid` adapter.
pub struct VelloHybrid {
    info: EngineInfo,
    gpu: Gpu,
    renderer: Option<Renderer>,
    renderer_config: Option<RenderTargetConfig>,
    res: Option<Resources>,
    scene: Option<HybridScene>,
    target: Option<Target>,
    prepared: Option<Prepared>,
    lowered: Option<Lowered<PaintType>>,
    counters: Counters,
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
        let gpu = Gpu::new()?;
        Ok(Self {
            info: EngineInfo {
                name: Self::NAME,
                engine_crate: "vello_hybrid",
                crate_version: env!("DEP_VELLO_HYBRID_VERSION"),
                source_rev: option_env!("DEP_VELLO_HYBRID_SOURCE_REV").map(String::from),
                output_format: format!("wgpu {:?} texture", gpu.target_format),
                precision: "vello_common strip rasterizer in f32; GPU colour pass writes rgba8unorm",
                route: "hybrid (cpu strips + gpu colour pass)",
                color_note: "peniko brushes (sRGB-tagged); output quantized to rgba8unorm sRGB; \
                             readback decoded sRGB→linear P3",
                encode_scope: "records `vello_hybrid::Scene` calls (set_paint/fill_path/\
                               push_layer/glyph_run) against fonts and atlas-resident images \
                               prepared once",
            },
            gpu,
            renderer: None,
            renderer_config: None,
            res: None,
            scene: None,
            target: None,
            prepared: None,
            lowered: None,
            counters: Counters::default(),
        })
    }
}

/// The `VelloLikeCtx` view over a `vello_hybrid::Scene` + its `Resources`.
///
/// `vello_hybrid` 0.2 renders images only through
/// [`ImageSource::OpaqueId`] — inline [`ImageSource::Pixmap`] paints panic
/// inside the GPU paint encoder by design (`vello_hybrid::Scene::set_paint`
/// documents pixmap sources as out of scope). The adapter therefore uploads
/// every image to the atlas via [`Renderer::upload_image`] once in
/// `prepare`, and [`Prepared::paint_type`] already resolves image paints
/// to their `OpaqueId` — `set_paint` needs no interception.
pub struct HybridCtx<'a> {
    scene: &'a mut HybridScene,
    res: &'a mut Resources,
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
            .font_size(size)
            .normalized_coords(coords)
            .fill_glyphs(glyphs.iter().map(|g| glifo::Glyph {
                id: g.id,
                x: g.x,
                y: g.y,
            }));
    }
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
        let config = RenderTargetConfig {
            format: self.gpu.target_format,
            width: input.scene.width,
            height: input.scene.height,
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
                    .create_command_encoder(&wgpu29::CommandEncoderDescriptor {
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
        };
        let lowered = self
            .lowered
            .as_ref()
            .ok_or_else(|| BenchError::Engine("vello-hybrid: encode before prepare".into()))?;
        replay(&mut ctx, lowered, &mut self.counters);
        // `bytes_uploaded` counts the decoded texels uploaded to the
        // atlas in `prepare` — the same quantity vello-classic reports
        // for its texture uploads.
        self.counters.bytes_uploaded = self.prepared.as_ref().map(Prepared::texel_bytes);
        Ok(())
    }

    fn submit(&mut self, frame: u64, readback_flag: bool) -> Result<Submit, BenchError> {
        let (Some(scene), Some(res), Some(renderer), Some(target)) =
            (&self.scene, &mut self.res, &mut self.renderer, &self.target)
        else {
            return Err(BenchError::Engine(
                "vello-hybrid: submit before encode".into(),
            ));
        };
        // Bracket the engine submission with timestamps written in their
        // own submissions, each after a full queue drain: on job-scheduled
        // tiled GPUs a timestamp in the same encoder shares no hazard with
        // the render and would bracket an empty interval. The drain
        // serializes CPU and GPU for the measured frame by design.
        drain_and_stamp(&self.gpu, 0)?;
        let render_at = Instant::now();
        let mut encoder =
            self.gpu
                .device
                .create_command_encoder(&wgpu29::CommandEncoderDescriptor {
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
                    width: target.width,
                    height: target.height,
                },
                &target.view,
                &TextureBindings::new(),
            )
            .map_err(|e| BenchError::Gpu(format!("vello-hybrid render: {e}")))?;
        self.gpu.queue.submit([encoder.finish()]);
        drain_and_stamp(&self.gpu, 1)?;
        let gpu_seconds = resolve_timestamps(&self.gpu)?;
        let render_seconds = render_at.elapsed().as_secs_f64();
        let readback_at = Instant::now();
        let image = if readback_flag {
            Some(readback(&self.gpu, target)?)
        } else {
            None
        };
        Ok(Submit {
            image,
            gpu: GpuSample::whole_frame(frame, gpu_seconds),
            phases: None,
            render_seconds: Some(render_seconds),
            readback_seconds: readback_flag.then(|| readback_at.elapsed().as_secs_f64()),
        })
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
            wgpu_allocator: wgpu29_allocator(&self.gpu.device, self.gpu.info.backend),
            skia_budgeted: Reading::unavailable("Vello hybrid has no Skia budget"),
            vk_memory_budget: wgpu29_vk_memory_budget(
                &self.gpu.device,
                self.gpu.info.backend,
                &self.gpu.info.name,
            ),
        }
    }
}
