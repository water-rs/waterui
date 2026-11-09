//! `vello-classic` adapter: `vello` (classic) on `wgpu`.
//!
//! Route: wgpu backend (Vulkan/Metal/D3D12/GL — whichever the system
//! provides; lavapipe on a headless Linux box). The target format is
//! chosen from the adapter's queried texture-format capabilities (see
//! [`crate::wgpu_ctx::Gpu::new`]). GPU time uses Vello's existing encoder
//! profiler where supported. Metal reports GPU timing unavailable; external
//! queue markers would include CPU submission gaps.

use std::collections::BTreeSet;
use std::time::Instant;

use cherenkov_scene::Feature;
use peniko::Brush;
use vello::kurbo::{Affine, Rect};
use vello::peniko::Fill;
use vello::{
    AaConfig, AaSupport, BumpReadback, RenderParams, Renderer, RendererOptions, Scene as VelloScene,
};

use crate::convert::{self, Prepared};
use crate::memory::{
    AdapterMemory, Reading, allocator_report, wgpu_metal_allocated_bytes, wgpu_vk_memory_budget,
};
use crate::vello_like::{Lowered, Op, lower, scrolled, vello_features};
use crate::wgpu_ctx::{Gpu, Target, readback, wait};
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
    /// When the last submit's final completion wait returned.
    completed: crate::ObservedCompletion,
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
        let gpu = Gpu::new(
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
        )?;
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
            completed: crate::ObservedCompletion::default(),
        })
    }
}

/// Replays a lowered op into `vello::Scene` — the timed `encode` is a
/// single loop of these recording calls.
fn replay_op(
    scene: &mut VelloScene,
    op: &Op<Brush>,
    counters: &mut Counters,
    scroll: Option<Affine>,
) {
    match op {
        Op::PushLayer {
            transform,
            clip,
            blend,
            opacity,
        } => {
            let transform = scrolled(*transform, scroll);
            let clip = clip.as_ref().expect("classic lowering fills opaque clips");
            scene.push_layer(Fill::NonZero, *blend, *opacity, transform, clip);
        }
        Op::PopLayer => scene.pop_layer(),
        Op::Fill {
            transform,
            rule,
            paint,
            paint_transform,
            path,
        } => {
            let transform = scrolled(*transform, scroll);
            counters.draw_commands += 1;
            scene.fill(*rule, transform, paint, *paint_transform, path);
        }
        Op::Stroke {
            transform,
            stroke,
            paint,
            paint_transform,
            path,
        } => {
            let transform = scrolled(*transform, scroll);
            counters.draw_commands += 1;
            scene.stroke(stroke, transform, paint, *paint_transform, path);
        }
        Op::ImageFill {
            transform,
            paint,
            paint_transform,
            dst_path,
            ..
        } => {
            let transform = scrolled(*transform, scroll);
            counters.draw_commands += 1;
            scene.fill(
                Fill::NonZero,
                transform,
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
            let transform = scrolled(*transform, scroll);
            counters.draw_commands += 1;
            scene
                .draw_glyphs(font)
                .font_size(*size)
                .normalized_coords(coords)
                .hint(false)
                .brush(paint)
                .transform(transform)
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
            let transform = scrolled(*transform, scroll);
            counters.draw_commands += 1;
            scene.draw_blurred_rounded_rect(transform, *rect, *color, *radius, *std_dev);
        }
    }
}

/// Upper bound on growth rounds (re-renders) a scene needs to stop
/// overflowing.
///
/// The fork's pipeline gates each stage on the failure bits of earlier
/// ones: `tile_alloc.wgsl` bails when `STAGE_BINNING | STAGE_FLATTEN` is
/// set, and `coarse.wgsl` skips on `STAGE_BINNING | STAGE_TILE_ALLOC |
/// STAGE_FLATTEN`, flagging `STAGE_PATH_COUNT`. A failed render therefore
/// reports exact demand only up to its first failing stage, and
/// `Renderer::verify_bump_readbacks` grows just that. Grown sizes persist
/// in the renderer, so a satisfied stage stays satisfied for the same
/// scene and each round clears at least one of the five stages
/// (`STAGE_BINNING`, `STAGE_TILE_ALLOC`, `STAGE_FLATTEN`,
/// `STAGE_PATH_COUNT`, `STAGE_COARSE`, see `shader/shared/bump.wgsl`).
/// The stage bits are WGSL constants, not exported by `vello_encoding`,
/// so the count is declared here.
const GROWTH_ROUNDS: u32 = 5;

/// One attempt, including the completion wait needed before checking overflow.
struct RenderAttempt {
    ticket: Option<BumpReadback>,
    gpu_seconds: Option<f64>,
    wait_seconds: f64,
    /// When the completion wait returned: the attempt's rendering is done.
    completed: Instant,
}

fn render_measured(
    gpu: &Gpu,
    renderer: &mut Renderer,
    scene: &VelloScene,
    target: &Target,
    params: &RenderParams,
) -> Result<RenderAttempt, BenchError> {
    let ticket = renderer
        .render_to_texture(&gpu.device, &gpu.queue, scene, &target.view, params)
        .map_err(|e| BenchError::Gpu(format!("vello render: {e}")))?;
    let wait_at = Instant::now();
    wait(gpu, None, "the Vello classic render and overflow feedback")?;
    let completed = Instant::now();
    let wait_seconds = completed.duration_since(wait_at).as_secs_f64();
    // Each attempt is drained here, so at most this attempt's profiler frame
    // is pending. render_to_texture may already have collected it.
    let profile = renderer.profile_result.take().or_else(|| {
        renderer
            .profiler
            .process_finished_frame(gpu.queue.get_timestamp_period())
    });
    let gpu_seconds = if gpu
        .device
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
    {
        let profile = profile.ok_or_else(|| {
            BenchError::Gpu("Vello profiler did not resolve the completed render".into())
        })?;
        let scope = profile
            .iter()
            .find(|scope| scope.label == "render_to_texture")
            .and_then(|scope| scope.time.as_ref())
            .ok_or_else(|| {
                BenchError::Gpu("Vello profiler omitted the render_to_texture interval".into())
            })?;
        if scope.end <= scope.start {
            return Err(BenchError::Gpu(
                "Vello profiler returned a non-increasing render interval".into(),
            ));
        }
        Some(scope.end - scope.start)
    } else {
        None
    };
    Ok(RenderAttempt {
        ticket,
        gpu_seconds,
        wait_seconds,
        completed,
    })
}

/// Whether `ticket`'s render overflowed its bump buffers — the feedback
/// [`Renderer::render_to_texture`] hands the caller to resolve. Dropping
/// the ticket unresolved is not an option here: this build's vello fork
/// `unwrap`s the map callback's send, so a discarded ticket panics on the
/// next device poll, and an unverified overflow would pass a truncated
/// frame off as rendered. `None` issues no feedback (the CPU pipeline)
/// and cannot overflow.
fn bump_overflowed(
    gpu: &Gpu,
    renderer: &mut Renderer,
    ticket: Option<BumpReadback>,
) -> Result<bool, BenchError> {
    let Some(ticket) = ticket else {
        return Ok(false);
    };
    let overflowed = renderer
        .verify_bump_readbacks(&gpu.device, vec![ticket])
        .map_err(|e| BenchError::Gpu(format!("vello bump readback: {e}")))?;
    Ok(!overflowed.is_empty())
}

impl Engine for VelloClassic {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn gpu_timing_unavailable(&self) -> Option<&'static str> {
        (!self
            .gpu
            .device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS))
        .then_some("Vello classic's encoder profiler is unavailable on this backend")
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
        let scroll = input.scroll_transform();
        for op in &lowered.ops {
            replay_op(&mut scene, op, &mut self.counters, scroll);
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
        // Verify every attempt and include all work needed to obtain a complete
        // frame. Overflow growth is part of rendering, not discarded warmup.
        let render_at = Instant::now();
        let mut attempt = render_measured(&self.gpu, renderer, scene, target, &params)?;
        let mut gpu_seconds = attempt.gpu_seconds;
        let mut gpu_wait_seconds = attempt.wait_seconds;
        let mut round = 0;
        while bump_overflowed(&self.gpu, renderer, attempt.ticket)? {
            round += 1;
            if round > GROWTH_ROUNDS {
                return Err(BenchError::Engine(format!(
                    "vello-classic: the scene still overflows the bump buffers after \
                     {GROWTH_ROUNDS} growth rounds, one per failure stage ({}x{})",
                    target.width, target.height
                )));
            }
            tracing::debug!(
                round,
                "vello-classic: re-rendering after bump buffer growth"
            );
            attempt = render_measured(&self.gpu, renderer, scene, target, &params)?;
            gpu_seconds = gpu_seconds
                .zip(attempt.gpu_seconds)
                .map(|(total, seconds)| total + seconds);
            gpu_wait_seconds += attempt.wait_seconds;
        }
        self.completed.record(attempt.completed);
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
            phases: Vec::new(),
            composition: None,
            gpu_wait_seconds,
            render_seconds: Some(render_seconds),
            readback_seconds: readback_flag.then(|| readback_at.elapsed().as_secs_f64()),
        })
    }

    fn completion_signal(&self) -> crate::CompletionSignal {
        crate::CompletionSignal::Measured(
            "the host instant the device wait inside submit returned for the frame's final render \
             attempt; submit drains the GPU to verify overflow feedback, so frames never overlap",
        )
    }

    fn completion(&mut self) -> Result<Option<crate::Completion>, BenchError> {
        self.completed.take(Self::NAME)
    }

    fn wait_idle(&mut self) -> Result<(), BenchError> {
        // submit waits for every render attempt before verifying its overflow feedback.
        Ok(())
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
            wgpu_allocator: allocator_report(&self.gpu.device, self.gpu.info.backend),
            skia_budgeted: Reading::unavailable("Vello classic has no Skia budget"),
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
