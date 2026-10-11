//! `vello-classic` adapter: `vello` (classic) on `wgpu`.
//!
//! Route: wgpu backend (Vulkan/Metal/D3D12/GL — whichever the system
//! provides; lavapipe on a headless Linux box). The target format is
//! chosen from the adapter's queried texture-format capabilities (see
//! [`crate::wgpu_ctx::Gpu::new`]). GPU time comes from drained standalone
//! timestamp submissions bracketing `render_to_texture` (see
//! [`crate::wgpu_ctx::drain_and_stamp`]) — real device timestamps that
//! serialize CPU and GPU for the measured frame, never wall clock.

use std::collections::BTreeSet;
use std::time::Instant;

use cherenkov_scene::Feature;
use peniko::Brush;
use vello::kurbo::{Affine, Rect};
use vello::peniko::Fill;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene as VelloScene};

use crate::convert::{self, Prepared};
use crate::memory::{AdapterMemory, Reading, wgpu29_allocator, wgpu29_vk_memory_budget};
use crate::vello_like::{Lowered, Op, lower, vello_features};
use crate::wgpu_ctx::{Gpu, Target, drain_and_stamp, readback, resolve_timestamps};
use crate::{BenchError, Counters, DeviceInfo, EncodeInput, Engine, EngineInfo, GpuSample, Submit};

/// `vello` classic adapter.
pub struct VelloClassic {
    info: EngineInfo,
    gpu: Gpu,
    renderer: Option<Renderer>,
    scene: Option<VelloScene>,
    target: Option<Target>,
    prepared: Option<Prepared>,
    lowered: Option<Lowered<Brush>>,
    counters: Counters,
}

/// The GPU and renderer state do not format.
impl std::fmt::Debug for VelloClassic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VelloClassic")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl VelloClassic {
    /// Adapter key.
    pub const NAME: &'static str = "vello-classic";

    /// Creates the adapter, initializing wgpu.
    ///
    /// # Errors
    /// [`BenchError::Gpu`] when no adapter exists.
    pub fn new() -> Result<Self, BenchError> {
        let gpu = Gpu::new()?;
        Ok(Self {
            info: EngineInfo {
                name: Self::NAME,
                engine_crate: "vello",
                crate_version: env!("DEP_VELLO_VERSION"),
                source_rev: option_env!("DEP_VELLO_SOURCE_REV").map(String::from),
                output_format: format!("wgpu {:?} texture", gpu.target_format),
                precision: "vello fine stage computes in f32 and writes rgba8unorm",
                route: "gpu (wgpu)",
                color_note: "peniko brushes quantized to sRGB rgba8 at the fine stage; readback \
                             decoded sRGB→linear P3",
                encode_scope: "records `vello::Scene` (fill/stroke/push_layer/draw_glyphs/\
                               draw_blurred_rounded_rect) against fonts/images prepared once",
            },
            gpu,
            renderer: None,
            scene: None,
            target: None,
            prepared: None,
            lowered: None,
            counters: Counters::default(),
        })
    }
}

/// Replays a lowered op into `vello::Scene` — the timed `encode` is a
/// single loop of these recording calls.
fn replay_op(scene: &mut VelloScene, op: &Op<Brush>, counters: &mut Counters) {
    match op {
        Op::PushLayer {
            transform,
            clip,
            blend,
            opacity,
        } => {
            let clip = clip.as_ref().expect("classic lowering fills opaque clips");
            scene.push_layer(Fill::NonZero, *blend, *opacity, *transform, clip);
        }
        Op::PopLayer => scene.pop_layer(),
        Op::Fill {
            transform,
            rule,
            paint,
            paint_transform,
            path,
        } => {
            counters.draw_commands += 1;
            scene.fill(*rule, *transform, paint, *paint_transform, path);
        }
        Op::Stroke {
            transform,
            stroke,
            paint,
            paint_transform,
            path,
        } => {
            counters.draw_commands += 1;
            scene.stroke(stroke, *transform, paint, *paint_transform, path);
        }
        Op::ImageFill {
            transform,
            paint,
            paint_transform,
            dst_path,
            ..
        } => {
            counters.draw_commands += 1;
            scene.fill(
                Fill::NonZero,
                *transform,
                paint,
                Some(*paint_transform),
                dst_path,
            );
        }
        Op::Glyphs {
            transform,
            paint,
            font,
            size,
            coords,
            glyphs,
            ..
        } => {
            counters.draw_commands += 1;
            scene
                .draw_glyphs(font)
                .font_size(*size)
                .normalized_coords(coords)
                .hint(false)
                .brush(paint)
                .transform(*transform)
                .draw(
                    Fill::NonZero,
                    glyphs.iter().map(|g| vello::Glyph {
                        id: g.id,
                        x: g.x,
                        y: g.y,
                    }),
                );
        }
        Op::Shadow {
            transform,
            rect,
            radius,
            std_dev,
            color,
        } => {
            counters.draw_commands += 1;
            scene.draw_blurred_rounded_rect(*transform, *rect, *color, *radius, *std_dev);
        }
    }
}

impl Engine for VelloClassic {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn supported(&self) -> BTreeSet<Feature> {
        vello_features().into_iter().collect()
    }

    fn prepare(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError> {
        convert::check_features(
            Self::NAME,
            input.scene,
            &vello_features(),
            crate::vello_like::vello_missing_api,
        )?;
        if self.renderer.is_none() {
            self.renderer = Some(
                Renderer::new(
                    &self.gpu.device,
                    RendererOptions {
                        use_cpu: false,
                        antialiasing_support: AaSupport::area_only(),
                        num_init_threads: std::num::NonZeroUsize::new(1),
                        pipeline_cache: None,
                        // The pinned fork grows the bump buffers on overflow;
                        // `None` lets it size from the scene each render.
                        buffer_sizes: None,
                    },
                )
                .map_err(|e| BenchError::Gpu(format!("vello renderer init: {e}")))?,
            );
        }
        let target = Target::new(
            &self.gpu.device,
            input.scene.width,
            input.scene.height,
            self.gpu.target_format,
        );
        self.target = Some(target);
        let prepared = Prepared::build(input.scene, input.blobs)?;
        // `push_layer` requires a clip argument, so `lower` fills opaque
        // clips here (`opaque_clip = true`); vello classic's brushes are
        // `peniko::Brush`.
        self.lowered = Some(lower(
            input.scene,
            &prepared,
            Self::NAME,
            true,
            &|p| prepared.brush(Self::NAME, p),
            &|hash, sampling| {
                Ok(Brush::Image(peniko::ImageBrush {
                    image: prepared.image(hash)?.clone(),
                    sampler: convert::image_sampler(sampling),
                }))
            },
        )?);
        self.prepared = Some(prepared);
        Ok(())
    }

    fn encode(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError> {
        let lowered = self
            .lowered
            .as_ref()
            .ok_or_else(|| BenchError::Engine("vello-classic: encode before prepare".into()))?;
        self.counters = Counters::default();
        self.counters.layers = lowered.layers;
        let mut scene = VelloScene::new();
        let clear = convert::peniko_solid(&input.scene.clear);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            clear,
            None,
            &Rect::new(
                0.0,
                0.0,
                f64::from(input.scene.width),
                f64::from(input.scene.height),
            ),
        );
        self.counters.draw_commands += 1;
        for op in &lowered.ops {
            replay_op(&mut scene, op, &mut self.counters);
        }
        self.scene = Some(scene);
        // `bytes_uploaded` counts the decoded texels the GPU receives, not
        // the compressed PNG sizes.
        self.counters.bytes_uploaded = self.prepared.as_ref().map(Prepared::texel_bytes);
        Ok(())
    }

    fn submit(&mut self, frame: u64, readback_flag: bool) -> Result<Submit, BenchError> {
        let (Some(scene), Some(target), Some(renderer)) =
            (&self.scene, &self.target, &mut self.renderer)
        else {
            return Err(BenchError::Engine(
                "vello-classic: submit before encode".into(),
            ));
        };
        let params = RenderParams {
            base_color: convert::peniko_solid(&cherenkov_scene::Color::srgb(0.0, 0.0, 0.0)),
            width: target.width,
            height: target.height,
            antialiasing_method: AaConfig::Area,
        };
        // GPU time: bracket the render with timestamps written in
        // standalone submissions after a full queue drain — on
        // job-scheduled tiled GPUs a timestamp inside the same submission
        // runs concurrently with the render and brackets an empty
        // interval. This serializes CPU and GPU for the measured frame.
        drain_and_stamp(&self.gpu, 0)?;
        let render_at = Instant::now();
        renderer
            .render_to_texture(
                &self.gpu.device,
                &self.gpu.queue,
                scene,
                &target.view,
                &params,
            )
            .map_err(|e| BenchError::Gpu(format!("vello render: {e}")))?;
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
            engine: Reading::unavailable("Vello classic does not expose engine memory"),
            wgpu_allocator: wgpu29_allocator(&self.gpu.device, self.gpu.info.backend),
            skia_budgeted: Reading::unavailable("Vello classic has no Skia budget"),
            vk_memory_budget: wgpu29_vk_memory_budget(
                &self.gpu.device,
                self.gpu.info.backend,
                &self.gpu.info.name,
            ),
        }
    }
}
