//! Generic GPU runtime for compiling a typed [`Filter`] graph into an [`Effect`].
//!
//! This module owns filter-stage fusion, deterministic parameter animation,
//! HDR intermediate selection, scratch-texture reuse, and command encoding.
//! It has no `WaterUI` dependency and can process images, decoded video frames,
//! or arbitrary wgpu textures in any Rust application.

extern crate alloc;

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::{fmt, time::Duration};
use std::sync::{
    OnceLock,
    mpsc::{self, Receiver, Sender},
};

use filtrate_core::{
    AnimationTrack, Chain, Filter, FilterParam, Interpolator, ParamArray, SignalVisitor,
    StageCollector, WatchGuard,
};
use num_traits::ToPrimitive;

#[cfg(test)]
use crate::effect::EffectFrameTiming;
use crate::effect::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRedrawCallback, EffectRenderResult,
    EffectSetupResult,
};

const MAX_FILTER_PARAMS: usize = 64;
const FILTER_UNIFORM_WORDS: usize = 4 + MAX_FILTER_PARAMS;
const SPATIAL_OUTPUT_FORMAT_TOKEN: &str = "OUTPUT_STORAGE_FORMAT";

/// Workgroup shape shared by every spatial compute shader. The WGSL sources
/// reference the `WORKGROUP_X` / `WORKGROUP_Y` tokens, which
/// [`specialize_spatial_shader`] substitutes with these values so shader
/// declarations and `dispatch_workgroups` can never drift apart.
/// 256 threads benchmarks at the memory-bandwidth floor on tiler GPUs.
const SPATIAL_WORKGROUP_X: u32 = 16;
const SPATIAL_WORKGROUP_Y: u32 = 16;
const SPATIAL_WORKGROUP_X_TOKEN: &str = "WORKGROUP_X";
const SPATIAL_WORKGROUP_Y_TOKEN: &str = "WORKGROUP_Y";

/// Policy for HDR behavior in filter pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HdrPolicy {
    /// Require HDR-capable intermediate pipeline; fail setup if unavailable.
    RequireHdr,
    /// Prefer HDR intermediates and automatically downgrade to LDR when unsupported.
    #[default]
    PreferHdr,
    /// Force LDR intermediates even on HDR-capable devices.
    ForceLdr,
}

const fn is_hdr_texture_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    )
}

const fn preferred_scratch_format(
    input_format: wgpu::TextureFormat,
    output_format: wgpu::TextureFormat,
) -> wgpu::TextureFormat {
    if is_hdr_texture_format(input_format) || is_hdr_texture_format(output_format) {
        wgpu::TextureFormat::Rgba16Float
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    }
}

fn scratch_texture_usage() -> wgpu::TextureUsages {
    wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::STORAGE_BINDING
        | wgpu::TextureUsages::RENDER_ATTACHMENT
}

const fn storage_format_to_wgsl(format: wgpu::TextureFormat) -> Result<&'static str, &'static str> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => Ok("rgba8unorm"),
        wgpu::TextureFormat::Rgba16Float => Ok("rgba16float"),
        wgpu::TextureFormat::Rgba32Float => Ok("rgba32float"),
        _ => Err("unsupported storage texture format for spatial filter"),
    }
}

fn specialize_spatial_shader(
    shader_source: &str,
    storage_format: wgpu::TextureFormat,
) -> Result<alloc::string::String, &'static str> {
    let storage_ty = storage_format_to_wgsl(storage_format)?;
    Ok(shader_source
        .replace(SPATIAL_OUTPUT_FORMAT_TOKEN, storage_ty)
        .replace(SPATIAL_WORKGROUP_X_TOKEN, &SPATIAL_WORKGROUP_X.to_string())
        .replace(SPATIAL_WORKGROUP_Y_TOKEN, &SPATIAL_WORKGROUP_Y.to_string()))
}

#[derive(Debug, Clone, Copy)]
enum AtomicStageKind {
    ColorFragment(&'static str),
    SpatialShader(&'static str),
    SpatialShaderWithOriginal(&'static str),
}

#[derive(Debug, Clone, Copy)]
struct AtomicStage {
    kind: AtomicStageKind,
    param_count: usize,
}

#[derive(Debug, Clone)]
enum PlannedPassKind {
    Color {
        fragments: alloc::string::String,
    },
    Spatial {
        shader: &'static str,
        original_input: bool,
    },
}

#[derive(Debug, Clone)]
struct PlannedPass {
    kind: PlannedPassKind,
    param_offset: usize,
    param_count: usize,
}

struct FilterSetupPlan {
    passes: Vec<PlannedPass>,
    scratch_formats: Vec<wgpu::TextureFormat>,
}

enum CompiledPassKind {
    Color {
        pipeline: wgpu::RenderPipeline,
        bind_group_layout: wgpu::BindGroupLayout,
    },
    Spatial {
        pipeline: wgpu::ComputePipeline,
        bind_group_layout: wgpu::BindGroupLayout,
        original_input: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PassTextureSource {
    Input,
    Scratch(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorTarget {
    Output,
    Scratch(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PassBindingPlan {
    Color {
        source: PassTextureSource,
        target: ColorTarget,
    },
    Spatial {
        source: PassTextureSource,
        target_scratch: usize,
    },
}

struct CompiledPass {
    kind: CompiledPassKind,
    param_offset: usize,
    param_count: usize,
    binding_plan: PassBindingPlan,
    uniform_buffer: wgpu::Buffer,
    last_uniform_data: Option<[f32; FILTER_UNIFORM_WORDS]>,
    cached_bind_group: Option<wgpu::BindGroup>,
    cached_dynamic_bind_group: Option<CachedDynamicBindGroup>,
}

struct FinalSpatialOutputPipeline {
    pass_index: usize,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

struct CachedDynamicBindGroup {
    source_view: wgpu::TextureView,
    target_view: Option<wgpu::TextureView>,
    original_view: Option<wgpu::TextureView>,
    bind_group: wgpu::BindGroup,
}

impl CachedDynamicBindGroup {
    fn matches(
        &self,
        source_view: &wgpu::TextureView,
        target_view: Option<&wgpu::TextureView>,
        original_view: Option<&wgpu::TextureView>,
    ) -> bool {
        self.source_view == *source_view
            && match (&self.target_view, target_view) {
                (Some(cached), Some(target)) => cached == target,
                (None, None) => true,
                _ => false,
            }
            && match (&self.original_view, original_view) {
                (Some(cached), Some(original)) => cached == original,
                (None, None) => true,
                _ => false,
            }
    }
}

fn fuse_stages(stages: &[AtomicStage]) -> Result<Vec<PlannedPass>, &'static str> {
    if stages.is_empty() {
        return Err("filter graph produced no stages");
    }

    let mut passes: Vec<PlannedPass> = Vec::with_capacity(stages.len());
    let mut param_offset = 0usize;

    for stage in stages {
        match stage.kind {
            AtomicStageKind::ColorFragment(fragment) => {
                if let Some(last) = passes.last_mut() {
                    if let PlannedPassKind::Color { fragments } = &mut last.kind {
                        fragments.push_str(fragment);
                        fragments.push('\n');
                        last.param_count += stage.param_count;
                    } else {
                        let mut fused = alloc::string::String::new();
                        fused.push_str(fragment);
                        fused.push('\n');
                        passes.push(PlannedPass {
                            kind: PlannedPassKind::Color { fragments: fused },
                            param_offset,
                            param_count: stage.param_count,
                        });
                    }
                } else {
                    let mut fused = alloc::string::String::new();
                    fused.push_str(fragment);
                    fused.push('\n');
                    passes.push(PlannedPass {
                        kind: PlannedPassKind::Color { fragments: fused },
                        param_offset,
                        param_count: stage.param_count,
                    });
                }
            }
            AtomicStageKind::SpatialShader(shader) => {
                passes.push(PlannedPass {
                    kind: PlannedPassKind::Spatial {
                        shader,
                        original_input: false,
                    },
                    param_offset,
                    param_count: stage.param_count,
                });
            }
            AtomicStageKind::SpatialShaderWithOriginal(shader) => {
                passes.push(PlannedPass {
                    kind: PlannedPassKind::Spatial {
                        shader,
                        original_input: true,
                    },
                    param_offset,
                    param_count: stage.param_count,
                });
            }
        }
        param_offset += stage.param_count;
    }

    Ok(passes)
}

fn create_pass_uniform_buffer(device: &wgpu::Device, label: &'static str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (FILTER_UNIFORM_WORDS * core::mem::size_of::<f32>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn upload_uniform_if_changed(
    queue: &wgpu::Queue,
    uniform_buffer: &wgpu::Buffer,
    last_uniform_data: &mut Option<[f32; FILTER_UNIFORM_WORDS]>,
    uniform_data: &[f32; FILTER_UNIFORM_WORDS],
) {
    let needs_upload = last_uniform_data.as_ref() != Some(uniform_data);
    if needs_upload {
        queue.write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&uniform_data[..]));
        *last_uniform_data = Some(*uniform_data);
    }
}

fn plan_runtime_bindings(
    planned: &[PlannedPass],
) -> Result<(Vec<PassBindingPlan>, Option<usize>), &'static str> {
    if planned.is_empty() {
        return Err("filter planner produced no passes");
    }

    let mut plans = Vec::with_capacity(planned.len());
    let mut source = PassTextureSource::Input;
    let mut next_scratch = 0usize;

    for (idx, pass) in planned.iter().enumerate() {
        let is_last = idx + 1 == planned.len();
        match &pass.kind {
            PlannedPassKind::Color { .. } => {
                let pass_source = source;
                let target = if is_last {
                    ColorTarget::Output
                } else {
                    let slot = next_scratch;
                    next_scratch ^= 1;
                    source = PassTextureSource::Scratch(slot);
                    ColorTarget::Scratch(slot)
                };
                plans.push(PassBindingPlan::Color {
                    source: pass_source,
                    target,
                });
            }
            PlannedPassKind::Spatial { .. } => {
                let target_scratch = next_scratch;
                plans.push(PassBindingPlan::Spatial {
                    source,
                    target_scratch,
                });
                source = PassTextureSource::Scratch(target_scratch);
                next_scratch ^= 1;
            }
        }
    }

    let blit_source_scratch = match planned.last().map(|pass| &pass.kind) {
        Some(PlannedPassKind::Spatial { .. }) => match source {
            PassTextureSource::Scratch(slot) => Some(slot),
            PassTextureSource::Input => {
                return Err("spatial pipeline planner produced invalid blit source");
            }
        },
        Some(PlannedPassKind::Color { .. }) | None => None,
    };

    Ok((plans, blit_source_scratch))
}

const PARAM_EPSILON: f32 = 0.000_01;

#[derive(Debug)]
struct ParamTrackState {
    track: AnimationTrack,
    animated_target: Option<f32>,
}

/// Shared animation state that can be updated from watcher callbacks.
#[derive(Debug)]
struct SharedAnimationState {
    /// Animation timeline for each parameter index.
    tracks: Vec<ParamTrackState>,
    /// Current values for each parameter (either animated or direct).
    current_values: Vec<f32>,
    /// Whether any animation is active.
    has_active_animation: bool,
}

const fn approx_param_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= PARAM_EPSILON
}

struct ParamAnimationEvent {
    param_index: usize,
    target_value: f32,
    interpolator: Option<Box<dyn Interpolator>>,
}

impl core::fmt::Debug for ParamAnimationEvent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ParamAnimationEvent")
            .field("param_index", &self.param_index)
            .field("target_value", &self.target_value)
            .field("animated", &self.interpolator.is_some())
            .finish()
    }
}

// ============================================================================
// Stage and signal visitors used by the planner / animation watcher install.
// ============================================================================

#[derive(Default)]
struct StageBuffer {
    stages: Vec<AtomicStage>,
}

impl StageBuffer {
    fn into_inner(self) -> Vec<AtomicStage> {
        self.stages
    }
}

impl StageCollector for StageBuffer {
    fn color_fragment(&mut self, source: &'static str, param_count: usize) {
        self.stages.push(AtomicStage {
            kind: AtomicStageKind::ColorFragment(source),
            param_count,
        });
    }
    fn spatial_shader(&mut self, source: &'static str, param_count: usize) {
        self.stages.push(AtomicStage {
            kind: AtomicStageKind::SpatialShader(source),
            param_count,
        });
    }

    fn spatial_shader_with_original(&mut self, source: &'static str, param_count: usize) {
        self.stages.push(AtomicStage {
            kind: AtomicStageKind::SpatialShaderWithOriginal(source),
            param_count,
        });
    }
}

fn collect_filter_stages<F: Filter>(filter: &F) -> Vec<AtomicStage> {
    let mut buffer = StageBuffer::default();
    filter.collect_stages(&mut buffer);
    buffer.into_inner()
}

struct WatcherInstaller<'a> {
    sender: Sender<ParamAnimationEvent>,
    redraw_callback: Arc<OnceLock<EffectRedrawCallback>>,
    guards: &'a mut Vec<WatchGuard>,
}

impl SignalVisitor for WatcherInstaller<'_> {
    fn visit<P: FilterParam + ?Sized>(&mut self, param_index: usize, param: &P) {
        let sender = self.sender.clone();
        let redraw_callback = self.redraw_callback.clone();
        let guard = param.watch_animated(Box::new(move |target| {
            sender
                .send(ParamAnimationEvent {
                    param_index,
                    target_value: target.value,
                    interpolator: target.interpolator,
                })
                .expect("FilterAdapter parameter event receiver dropped while watcher is active");
            if let Some(callback) = redraw_callback.get() {
                callback();
            }
        }));
        self.guards.push(guard);
    }
}

// ============================================================================
// Filter trait adapter - converts Filter to Effect with animation support
// ============================================================================

/// Adapter that wraps a `Filter` to implement `Effect` with animation support.
///
/// This bridges the pure-data `Filter` trait from filtrate-core to the
/// GPU-aware `Effect` trait used by the rendering system.
///
/// When filter parameters change with animation metadata, this adapter
/// smoothly interpolates values and signals for continued rendering.
///
/// ## Pipeline Selection
///
/// - **Color-only filters** (`F::COLOR_ONLY = true`): Use fragment shaders for native HDR support.
/// - **Spatial filters** (`F::COLOR_ONLY = false`): Use compute shaders with intermediate texture for HDR.
pub struct FilterAdapter<F: Filter> {
    filter: F,
    /// Current parameter targets delivered by reactive watcher events.
    target_params: Vec<f32>,
    /// True when target parameters changed since the last successful render.
    target_params_dirty: bool,
    passes: Vec<CompiledPass>,
    /// Whether render should use scratch ping-pong textures.
    requires_scratch: bool,
    /// HDR/LDR behavior policy for intermediate passes.
    hdr_policy: HdrPolicy,
    /// Scratch texture format for intermediate passes (SDR/HDR).
    scratch_format: wgpu::TextureFormat,
    /// Sticky setup error: once set, render fails fast.
    setup_error: Option<&'static str>,
    // Shared resources
    sampler: Option<wgpu::Sampler>,
    /// Animation state owned by the render thread.
    animation_state: SharedAnimationState,
    /// Parameter watcher guards dropped before their event channel and callback.
    _watcher_guards: Vec<WatchGuard>,
    /// Parameter-change events, each carrying optional animation metadata.
    animation_events: Receiver<ParamAnimationEvent>,
    /// Host wake callback shared with every parameter watcher.
    redraw_callback: Arc<OnceLock<EffectRedrawCallback>>,
    // Scratch ping-pong textures for multi-pass.
    scratch_textures: [Option<wgpu::Texture>; 2],
    scratch_views: [Option<wgpu::TextureView>; 2],
    scratch_size: (u32, u32),
    // Final blit when last stage is spatial.
    blit_pipeline: Option<wgpu::RenderPipeline>,
    blit_bind_group_layout: Option<wgpu::BindGroupLayout>,
    blit_bind_group: Option<wgpu::BindGroup>,
    blit_source_scratch_slot: Option<usize>,
    final_spatial_output: Option<FinalSpatialOutputPipeline>,
    #[cfg(test)]
    last_render_used_direct_output: bool,
}

impl<F: Filter> fmt::Debug for FilterAdapter<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterAdapter").finish_non_exhaustive()
    }
}

#[allow(private_bounds)]
impl<F: Filter> FilterAdapter<F> {
    /// Create a new filter adapter.
    #[must_use]
    pub fn new(filter: F) -> Self {
        let param_count = <F::Params as ParamArray>::LEN;
        let (animation_events_tx, animation_events) = mpsc::channel();
        let redraw_callback = Arc::new(OnceLock::new());
        let mut watcher_guards = Vec::with_capacity(param_count);
        filter.visit_signals(&mut WatcherInstaller {
            sender: animation_events_tx,
            redraw_callback: redraw_callback.clone(),
            guards: &mut watcher_guards,
        });

        let mut target_params = alloc::vec![0.0; param_count];
        filter.params().write_to(&mut target_params);
        let animation_state = SharedAnimationState {
            tracks: target_params
                .iter()
                .copied()
                .map(|value| ParamTrackState {
                    track: AnimationTrack::new(value),
                    animated_target: None,
                })
                .collect(),
            current_values: target_params.clone(),
            has_active_animation: false,
        };
        Self {
            filter,
            target_params,
            target_params_dirty: true,
            passes: Vec::new(),
            requires_scratch: false,
            hdr_policy: HdrPolicy::default(),
            scratch_format: wgpu::TextureFormat::Rgba8Unorm,
            setup_error: None,
            sampler: None,
            animation_state,
            _watcher_guards: watcher_guards,
            animation_events,
            redraw_callback,
            scratch_textures: [None, None],
            scratch_views: [None, None],
            scratch_size: (0, 0),
            blit_pipeline: None,
            blit_bind_group_layout: None,
            blit_bind_group: None,
            blit_source_scratch_slot: None,
            final_spatial_output: None,
            #[cfg(test)]
            last_render_used_direct_output: false,
        }
    }

    /// Borrows the typed filter graph wrapped by this runtime adapter.
    #[must_use]
    pub const fn filter(&self) -> &F {
        &self.filter
    }

    /// Chain another filter onto this adapter.
    ///
    /// Returns a new `FilterAdapter` wrapping a `Chain` of both filters.
    /// Consecutive color-only filters will be fused into a single GPU pass.
    #[must_use]
    pub fn then<F2: Filter>(self, filter: F2) -> FilterAdapter<Chain<F, F2>> {
        let redraw_callback = self.redraw_callback.get().cloned();
        let mut next = FilterAdapter::new(Chain {
            first: self.filter,
            second: filter,
        });
        next.hdr_policy = self.hdr_policy;
        if let Some(redraw_callback) = redraw_callback {
            next.install_redraw_callback(redraw_callback);
        }
        next
    }

    /// Set HDR behavior policy for this filter chain.
    #[must_use]
    pub const fn hdr_policy(mut self, policy: HdrPolicy) -> Self {
        self.hdr_policy = policy;
        self
    }

    /// Require HDR intermediates; setup fails if unsupported.
    #[must_use]
    pub const fn require_hdr(self) -> Self {
        self.hdr_policy(HdrPolicy::RequireHdr)
    }

    /// Prefer HDR intermediates with automatic LDR fallback.
    #[must_use]
    pub const fn prefer_hdr(self) -> Self {
        self.hdr_policy(HdrPolicy::PreferHdr)
    }

    /// Force LDR intermediates for maximum compatibility.
    #[must_use]
    pub const fn force_ldr(self) -> Self {
        self.hdr_policy(HdrPolicy::ForceLdr)
    }

    fn install_redraw_callback(&self, callback: EffectRedrawCallback) {
        assert!(
            self.redraw_callback.set(callback).is_ok(),
            "FilterAdapter redraw callback must be installed exactly once before setup"
        );
    }

    fn apply_target_params_to_current_values(&mut self) {
        let param_count = self.target_params.len();
        for i in 0..param_count {
            let target = self.target_params[i];
            self.animation_state.current_values[i] = target;
            self.animation_state.tracks[i]
                .track
                .set_target(target, None);
            self.animation_state.tracks[i].animated_target = None;
        }
        self.target_params_dirty = false;
    }

    fn consume_animation_events(&mut self) {
        while let Ok(event) = self.animation_events.try_recv() {
            assert!(
                event.param_index < self.animation_state.current_values.len(),
                "FilterAdapter watcher produced out-of-range parameter index {}",
                event.param_index
            );
            self.target_params[event.param_index] = event.target_value;
            self.target_params_dirty = true;
            let entry = &mut self.animation_state.tracks[event.param_index];
            entry
                .track
                .set_target(event.target_value, event.interpolator);
            entry.animated_target = entry.track.is_active().then_some(event.target_value);
        }
        self.animation_state.has_active_animation = self
            .animation_state
            .tracks
            .iter()
            .any(|entry| entry.track.is_active());
    }

    /// Update interpolated parameters in-place; returns whether another frame is needed.
    fn update_interpolated_params(&mut self, delta: Duration) -> bool {
        let param_count = self.target_params.len();
        if param_count > MAX_FILTER_PARAMS {
            return false;
        }
        self.consume_animation_events();
        let mut needs_redraw = false;

        for i in 0..param_count {
            let target = self.target_params[i];
            let entry = &mut self.animation_state.tracks[i];

            if let Some(animated_target) = entry.animated_target {
                // Underlying target changed without a new animation event:
                // fail fast to direct target sync so state stays coherent.
                if !approx_param_eq(animated_target, target) {
                    entry.track.set_target(target, None);
                    entry.animated_target = None;
                }
            }

            if entry.animated_target.is_none()
                && !approx_param_eq(self.animation_state.current_values[i], target)
            {
                entry.track.set_target(target, None);
            }

            let active = entry.track.advance(delta);
            self.animation_state.current_values[i] = entry.track.value();

            if active {
                needs_redraw = true;
            } else {
                entry.animated_target = None;
            }
        }

        self.animation_state.has_active_animation = needs_redraw;
        needs_redraw
    }

    fn set_setup_error(&mut self, err: &'static str) {
        if self.setup_error.is_none() {
            self.setup_error = Some(err);
            tracing::error!("[Filter] setup failed fast: {err}");
        }
    }

    #[cfg(test)]
    const fn has_setup_error(&self) -> bool {
        self.setup_error.is_some()
    }

    #[cfg(test)]
    const fn last_render_used_direct_output(&self) -> bool {
        self.last_render_used_direct_output
    }

    #[cfg(test)]
    const fn allocated_scratch_slots(&self) -> [bool; 2] {
        [
            self.scratch_views[0].is_some(),
            self.scratch_views[1].is_some(),
        ]
    }

    fn required_scratch_slots_for_frame(
        &self,
        direct_output_pass_index: Option<usize>,
    ) -> [bool; 2] {
        let mut required = [false; 2];

        for (pass_index, pass) in self.passes.iter().enumerate() {
            match pass.binding_plan {
                PassBindingPlan::Color { source, target } => {
                    if let PassTextureSource::Scratch(slot) = source {
                        required[slot] = true;
                    }
                    if let ColorTarget::Scratch(slot) = target {
                        required[slot] = true;
                    }
                }
                PassBindingPlan::Spatial {
                    source,
                    target_scratch,
                } => {
                    if let PassTextureSource::Scratch(slot) = source {
                        required[slot] = true;
                    }
                    if direct_output_pass_index != Some(pass_index) {
                        required[target_scratch] = true;
                    }
                }
            }
        }

        if direct_output_pass_index.is_none()
            && let Some(blit_slot) = self.blit_source_scratch_slot
        {
            required[blit_slot] = true;
        }

        required
    }

    fn ensure_scratch_textures(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        required_slots: [bool; 2],
    ) {
        if !self.requires_scratch {
            return;
        }

        let size_changed = self.scratch_size != (width, height);
        let mut bindings_invalidated = false;

        for (slot, required) in required_slots.iter().copied().enumerate() {
            if !required {
                if self.scratch_textures[slot].is_some() || self.scratch_views[slot].is_some() {
                    self.scratch_textures[slot] = None;
                    self.scratch_views[slot] = None;
                    bindings_invalidated = true;
                }
                continue;
            }

            let missing =
                self.scratch_textures[slot].is_none() || self.scratch_views[slot].is_none();
            if size_changed || missing {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("filter scratch texture"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.scratch_format,
                    usage: scratch_texture_usage(),
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.scratch_textures[slot] = Some(texture);
                self.scratch_views[slot] = Some(view);
                bindings_invalidated = true;
            }
        }

        if bindings_invalidated {
            for pass in &mut self.passes {
                pass.cached_bind_group = None;
                pass.cached_dynamic_bind_group = None;
            }
            self.blit_bind_group = None;
        }

        self.scratch_size = if required_slots.iter().any(|required| *required) {
            (width, height)
        } else {
            (0, 0)
        };
    }

    #[expect(
        clippy::too_many_lines,
        clippy::future_not_send,
        reason = "filter pass compilation keeps one ordered device-bound resource graph on the GPU host thread"
    )]
    async fn build_compiled_passes(
        &mut self,
        ctx: &EffectContext<'_>,
        planned: &[PlannedPass],
        scratch_format: wgpu::TextureFormat,
    ) -> Result<(), &'static str> {
        self.passes.clear();
        self.blit_bind_group = None;
        self.final_spatial_output = None;
        let (binding_plans, blit_source_scratch_slot) = plan_runtime_bindings(planned)?;
        self.blit_source_scratch_slot = blit_source_scratch_slot;

        if self.requires_scratch {
            ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let probe = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("filter scratch format probe"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: scratch_format,
                usage: scratch_texture_usage(),
                view_formats: &[],
            });
            let _ = probe.create_view(&wgpu::TextureViewDescriptor::default());
            if ctx.device.pop_error_scope().await.is_some() {
                return Err("selected scratch texture format is unsupported on this device");
            }
        }

        for (pass, binding_plan) in planned.iter().zip(binding_plans) {
            match &pass.kind {
                PlannedPassKind::Color { fragments } => {
                    let target_format = match binding_plan {
                        PassBindingPlan::Color {
                            target: ColorTarget::Output,
                            ..
                        } => ctx.output_format,
                        PassBindingPlan::Color {
                            target: ColorTarget::Scratch(_),
                            ..
                        } => scratch_format,
                        PassBindingPlan::Spatial { .. } => {
                            return Err("runtime planner produced invalid color binding plan");
                        }
                    };
                    ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
                    let (pipeline, bind_group_layout) =
                        Self::create_color_pipeline(ctx, fragments, target_format);
                    if ctx.device.pop_error_scope().await.is_some() {
                        return Err("failed to create color pipeline for selected target format");
                    }
                    self.passes.push(CompiledPass {
                        kind: CompiledPassKind::Color {
                            pipeline,
                            bind_group_layout,
                        },
                        param_offset: pass.param_offset,
                        param_count: pass.param_count,
                        binding_plan,
                        uniform_buffer: create_pass_uniform_buffer(
                            ctx.device,
                            "filter color uniform buffer",
                        ),
                        last_uniform_data: None,
                        cached_bind_group: None,
                        cached_dynamic_bind_group: None,
                    });
                }
                PlannedPassKind::Spatial {
                    shader,
                    original_input,
                } => {
                    if !matches!(binding_plan, PassBindingPlan::Spatial { .. }) {
                        return Err("runtime planner produced invalid spatial binding plan");
                    }
                    ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
                    let (pipeline, bind_group_layout) = Self::create_spatial_pipeline(
                        ctx,
                        shader,
                        scratch_format,
                        *original_input,
                    )?;
                    if let Some(err) = ctx.device.pop_error_scope().await {
                        tracing::error!("[Filter] spatial pipeline validation error: {err:?}");
                        return Err(
                            "failed to create spatial pipeline for selected storage format",
                        );
                    }
                    self.passes.push(CompiledPass {
                        kind: CompiledPassKind::Spatial {
                            pipeline,
                            bind_group_layout,
                            original_input: *original_input,
                        },
                        param_offset: pass.param_offset,
                        param_count: pass.param_count,
                        binding_plan,
                        uniform_buffer: create_pass_uniform_buffer(
                            ctx.device,
                            "filter spatial uniform buffer",
                        ),
                        last_uniform_data: None,
                        cached_bind_group: None,
                        cached_dynamic_bind_group: None,
                    });
                }
            }
        }

        if self.passes.is_empty() {
            return Err("filter setup produced no executable passes");
        }

        if let Some((pass_index, shader)) =
            planned
                .iter()
                .enumerate()
                .find_map(|(idx, pass)| match pass.kind {
                    PlannedPassKind::Spatial {
                        shader,
                        original_input,
                    } if idx + 1 == planned.len() => {
                        if original_input {
                            return None;
                        }
                        Some((idx, shader))
                    }
                    _ => None,
                })
            && storage_format_to_wgsl(ctx.output_format).is_ok()
        {
            ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
            match Self::create_spatial_pipeline(ctx, shader, ctx.output_format, false) {
                Ok((pipeline, bind_group_layout)) => {
                    if ctx.device.pop_error_scope().await.is_none() {
                        self.final_spatial_output = Some(FinalSpatialOutputPipeline {
                            pass_index,
                            pipeline,
                            bind_group_layout,
                        });
                    } else {
                        tracing::debug!(
                            "[Filter] final spatial direct-output path unavailable for output format {:?}",
                            ctx.output_format
                        );
                    }
                }
                Err(_) => {
                    tracing::debug!(
                        "[Filter] final spatial direct-output shader specialization unsupported for {:?}",
                        ctx.output_format
                    );
                }
            }
        }

        if self.blit_source_scratch_slot.is_some() {
            ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let (blit_pipeline, blit_bind_group_layout) = Self::create_blit_pipeline(ctx);
            if ctx.device.pop_error_scope().await.is_some() {
                return Err("failed to create final blit pipeline");
            }
            self.blit_pipeline = Some(blit_pipeline);
            self.blit_bind_group_layout = Some(blit_bind_group_layout);
        } else {
            self.blit_pipeline = None;
            self.blit_bind_group_layout = None;
            self.blit_bind_group = None;
        }

        Ok(())
    }

    fn plan_setup(&mut self, ctx: &EffectContext<'_>) -> Result<FilterSetupPlan, &'static str> {
        let _ = self
            .redraw_callback
            .get_or_init(|| Arc::new(|| {}) as EffectRedrawCallback);
        if <F::Params as ParamArray>::LEN > MAX_FILTER_PARAMS {
            return Err("filter chain exceeds 64 params (uniform limit)");
        }

        self.sampler = Some(ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("filter sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        }));

        let planned = fuse_stages(&collect_filter_stages(&self.filter))?;
        self.requires_scratch = planned.len() > 1
            || matches!(
                planned.last().map(|pass| &pass.kind),
                Some(PlannedPassKind::Spatial { .. })
            );
        let needs_original_input_scratch = planned.iter().any(|pass| {
            matches!(
                pass.kind,
                PlannedPassKind::Spatial {
                    original_input: true,
                    ..
                }
            )
        });
        let scratch_candidates = if self.requires_scratch {
            match self.hdr_policy {
                HdrPolicy::ForceLdr => alloc::vec![wgpu::TextureFormat::Rgba8Unorm],
                HdrPolicy::RequireHdr => alloc::vec![wgpu::TextureFormat::Rgba16Float],
                HdrPolicy::PreferHdr if needs_original_input_scratch => {
                    alloc::vec![wgpu::TextureFormat::Rgba16Float]
                }
                HdrPolicy::PreferHdr => {
                    let preferred = preferred_scratch_format(ctx.input_format, ctx.output_format);
                    if is_hdr_texture_format(preferred) {
                        alloc::vec![preferred, wgpu::TextureFormat::Rgba8Unorm]
                    } else {
                        alloc::vec![preferred, wgpu::TextureFormat::Rgba16Float]
                    }
                }
            }
        } else {
            alloc::vec![wgpu::TextureFormat::Rgba8Unorm]
        };
        Ok(FilterSetupPlan {
            passes: planned,
            scratch_formats: scratch_candidates,
        })
    }

    fn clear_compiled_passes(&mut self) {
        self.passes.clear();
        self.blit_pipeline = None;
        self.blit_bind_group_layout = None;
        self.blit_bind_group = None;
        self.blit_source_scratch_slot = None;
        self.final_spatial_output = None;
    }

    #[expect(
        clippy::future_not_send,
        reason = "filter pipeline compilation awaits device validation on the GPU host thread"
    )]
    async fn compile_setup_plan(
        &mut self,
        ctx: &EffectContext<'_>,
        planned: &[PlannedPass],
        scratch_candidates: &[wgpu::TextureFormat],
    ) -> Result<(), &'static str> {
        let mut last_error = None;
        for (index, candidate) in scratch_candidates.iter().copied().enumerate() {
            match self.build_compiled_passes(ctx, planned, candidate).await {
                Ok(()) => {
                    self.scratch_format = candidate;
                    if index > 0 && self.hdr_policy == HdrPolicy::PreferHdr {
                        tracing::warn!(
                            "[Filter] preferred scratch format unavailable, falling back to {candidate:?}"
                        );
                    }
                    return Ok(());
                }
                Err(error) => {
                    last_error = Some(error);
                    self.clear_compiled_passes();
                }
            }
        }

        Err(match (self.hdr_policy, last_error) {
            (HdrPolicy::RequireHdr, Some(_)) => {
                "HDR is required by policy but unavailable for this filter pipeline"
            }
            (_, Some(error)) => error,
            (_, None) => "filter setup produced no executable passes",
        })
    }
}

impl<F: Filter> Effect for FilterAdapter<F> {
    fn set_redraw_callback(&mut self, callback: EffectRedrawCallback) {
        self.install_redraw_callback(callback);
    }

    #[expect(
        clippy::future_not_send,
        reason = "filter setup owns device-bound pipelines and runs on the GPU host thread"
    )]
    async fn setup(&mut self, ctx: &EffectContext<'_>) -> EffectSetupResult {
        let plan = match self.plan_setup(ctx) {
            Ok(plan) => plan,
            Err(error) => {
                self.set_setup_error(error);
                return Err(error);
            }
        };
        if let Err(error) = self
            .compile_setup_plan(ctx, &plan.passes, &plan.scratch_formats)
            .await
        {
            self.set_setup_error(error);
            return Err(error);
        }
        self.apply_target_params_to_current_values();
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn encode_render(
        &mut self,
        input: &EffectInput,
        output: &EffectOutput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        #[cfg(test)]
        {
            self.last_render_used_direct_output = false;
        }
        if let Some(err) = self.setup_error {
            return Err(err);
        }
        if self.passes.is_empty() {
            return Err("filter render called before a compiled pass graph exists");
        }

        let direct_output_runtime = self
            .final_spatial_output
            .as_ref()
            .and_then(|direct_output| {
                if output
                    .texture
                    .usage()
                    .contains(wgpu::TextureUsages::STORAGE_BINDING)
                {
                    Some((
                        direct_output.pass_index,
                        direct_output.pipeline.clone(),
                        direct_output.bind_group_layout.clone(),
                    ))
                } else {
                    None
                }
            });

        if self.requires_scratch {
            let direct_output_pass_index = direct_output_runtime
                .as_ref()
                .map(|(pass_index, _, _)| *pass_index);
            let required_scratch_slots =
                self.required_scratch_slots_for_frame(direct_output_pass_index);
            self.ensure_scratch_textures(
                input.device,
                output.width,
                output.height,
                required_scratch_slots,
            );
            for (slot, required) in required_scratch_slots.into_iter().enumerate() {
                if required && self.scratch_views[slot].is_none() {
                    return Err("required scratch texture view was not allocated");
                }
            }
        }

        let needs_redraw = self.update_interpolated_params(input.timing.delta());
        let current_values = &self.animation_state.current_values;
        if current_values.is_empty() && <F::Params as ParamArray>::LEN > 0 {
            return Err("filter render missing current parameter values");
        }

        let Some(sampler) = &self.sampler else {
            return Err("filter sampler missing after setup");
        };

        let mut used_direct_spatial_output = false;
        let mut source_width = input.width;
        let mut source_height = input.height;
        for (pass_index, pass) in self.passes.iter_mut().enumerate() {
            let param_start = pass.param_offset;
            let param_end = param_start + pass.param_count;
            let params = &current_values[param_start..param_end];

            match (&pass.kind, pass.binding_plan) {
                (
                    CompiledPassKind::Color {
                        pipeline,
                        bind_group_layout,
                    },
                    PassBindingPlan::Color { source, target },
                ) => {
                    let source_view: &wgpu::TextureView = match source {
                        PassTextureSource::Input => &input.view,
                        PassTextureSource::Scratch(slot) => {
                            let Some(view) = self.scratch_views[slot].as_ref() else {
                                return Err("color pass source scratch view missing");
                            };
                            view
                        }
                    };
                    let target_view: &wgpu::TextureView = match target {
                        ColorTarget::Output => &output.view,
                        ColorTarget::Scratch(slot) => {
                            let Some(view) = self.scratch_views[slot].as_ref() else {
                                return Err("color pass target scratch view missing");
                            };
                            view
                        }
                    };

                    let uniform_data =
                        build_color_uniform_data(source_width, source_height, params);
                    upload_uniform_if_changed(
                        input.queue,
                        &pass.uniform_buffer,
                        &mut pass.last_uniform_data,
                        &uniform_data,
                    );

                    let bind_group = if matches!(source, PassTextureSource::Input) {
                        if pass
                            .cached_dynamic_bind_group
                            .as_ref()
                            .is_none_or(|cached| !cached.matches(source_view, None, None))
                        {
                            let bind_group =
                                input.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                    label: Some("filter color dynamic bind group"),
                                    layout: bind_group_layout,
                                    entries: &[
                                        wgpu::BindGroupEntry {
                                            binding: 0,
                                            resource: wgpu::BindingResource::TextureView(
                                                source_view,
                                            ),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 1,
                                            resource: wgpu::BindingResource::Sampler(sampler),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 2,
                                            resource: pass.uniform_buffer.as_entire_binding(),
                                        },
                                    ],
                                });
                            pass.cached_dynamic_bind_group = Some(CachedDynamicBindGroup {
                                source_view: source_view.clone(),
                                target_view: None,
                                original_view: None,
                                bind_group,
                            });
                        }
                        let Some(cached) = pass.cached_dynamic_bind_group.as_ref() else {
                            return Err("color dynamic bind group cache missing after creation");
                        };
                        &cached.bind_group
                    } else {
                        if pass.cached_bind_group.is_none() {
                            pass.cached_bind_group =
                                Some(input.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                    label: Some("filter color static bind group"),
                                    layout: bind_group_layout,
                                    entries: &[
                                        wgpu::BindGroupEntry {
                                            binding: 0,
                                            resource: wgpu::BindingResource::TextureView(
                                                source_view,
                                            ),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 1,
                                            resource: wgpu::BindingResource::Sampler(sampler),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 2,
                                            resource: pass.uniform_buffer.as_entire_binding(),
                                        },
                                    ],
                                }));
                        }
                        let Some(bind_group) = pass.cached_bind_group.as_ref() else {
                            return Err("color pass bind group cache missing after creation");
                        };
                        bind_group
                    };

                    {
                        let mut render_pass =
                            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("filter color pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: target_view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                            });
                        render_pass.set_pipeline(pipeline);
                        render_pass.set_bind_group(0, bind_group, &[]);
                        render_pass.draw(0..6, 0..1);
                    }

                    if matches!(target, ColorTarget::Scratch(_)) {
                        source_width = output.width;
                        source_height = output.height;
                    }
                }
                (
                    CompiledPassKind::Spatial {
                        pipeline,
                        bind_group_layout,
                        original_input,
                    },
                    PassBindingPlan::Spatial {
                        source,
                        target_scratch,
                    },
                ) => {
                    let source_view: &wgpu::TextureView = match source {
                        PassTextureSource::Input => &input.view,
                        PassTextureSource::Scratch(slot) => {
                            let Some(view) = self.scratch_views[slot].as_ref() else {
                                return Err("spatial pass source scratch view missing");
                            };
                            view
                        }
                    };
                    let mut writes_output_directly = false;
                    let (target_view, dispatch_pipeline, dispatch_bind_group_layout): (
                        &wgpu::TextureView,
                        &wgpu::ComputePipeline,
                        &wgpu::BindGroupLayout,
                    ) = if let Some((
                        direct_pass_index,
                        direct_pipeline,
                        direct_bind_group_layout,
                    )) = &direct_output_runtime
                    {
                        if *direct_pass_index == pass_index {
                            writes_output_directly = true;
                            (&output.view, direct_pipeline, direct_bind_group_layout)
                        } else {
                            let Some(target_view) = self.scratch_views[target_scratch].as_ref()
                            else {
                                return Err("spatial pass target scratch view missing");
                            };
                            (target_view, pipeline, bind_group_layout)
                        }
                    } else {
                        let Some(target_view) = self.scratch_views[target_scratch].as_ref() else {
                            return Err("spatial pass target scratch view missing");
                        };
                        (target_view, pipeline, bind_group_layout)
                    };
                    let target_width = output.width;
                    let target_height = output.height;

                    let uniform_data = build_spatial_uniform_data(
                        target_width,
                        target_height,
                        source_width,
                        source_height,
                        params,
                    );
                    upload_uniform_if_changed(
                        input.queue,
                        &pass.uniform_buffer,
                        &mut pass.last_uniform_data,
                        &uniform_data,
                    );

                    let original_view = if *original_input {
                        Some(&input.view)
                    } else {
                        None
                    };
                    let bind_group = if writes_output_directly
                        || matches!(source, PassTextureSource::Input)
                        || *original_input
                    {
                        if pass
                            .cached_dynamic_bind_group
                            .as_ref()
                            .is_none_or(|cached| {
                                !cached.matches(source_view, Some(target_view), original_view)
                            })
                        {
                            let mut entries = alloc::vec![
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(source_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(target_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: pass.uniform_buffer.as_entire_binding(),
                                },
                            ];
                            if let Some(original_view) = original_view {
                                entries.push(wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(original_view),
                                });
                            }
                            let bind_group =
                                input.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                    label: Some("filter spatial dynamic bind group"),
                                    layout: dispatch_bind_group_layout,
                                    entries: &entries,
                                });
                            pass.cached_dynamic_bind_group = Some(CachedDynamicBindGroup {
                                source_view: source_view.clone(),
                                target_view: Some(target_view.clone()),
                                original_view: original_view.cloned(),
                                bind_group,
                            });
                        }
                        let Some(cached) = pass.cached_dynamic_bind_group.as_ref() else {
                            return Err("spatial dynamic bind group cache missing after creation");
                        };
                        &cached.bind_group
                    } else {
                        if pass.cached_bind_group.is_none() {
                            pass.cached_bind_group =
                                Some(input.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                    label: Some("filter spatial static bind group"),
                                    layout: dispatch_bind_group_layout,
                                    entries: &[
                                        wgpu::BindGroupEntry {
                                            binding: 0,
                                            resource: wgpu::BindingResource::TextureView(
                                                source_view,
                                            ),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 1,
                                            resource: wgpu::BindingResource::TextureView(
                                                target_view,
                                            ),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 2,
                                            resource: pass.uniform_buffer.as_entire_binding(),
                                        },
                                    ],
                                }));
                        }
                        let Some(bind_group) = pass.cached_bind_group.as_ref() else {
                            return Err("spatial pass bind group cache missing after creation");
                        };
                        bind_group
                    };

                    {
                        let mut compute_pass =
                            encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                label: Some("filter spatial pass"),
                                timestamp_writes: None,
                            });
                        compute_pass.set_pipeline(dispatch_pipeline);
                        compute_pass.set_bind_group(0, bind_group, &[]);
                        let workgroups_x = target_width.div_ceil(SPATIAL_WORKGROUP_X);
                        let workgroups_y = target_height.div_ceil(SPATIAL_WORKGROUP_Y);
                        compute_pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
                    }

                    if writes_output_directly {
                        used_direct_spatial_output = true;
                    }

                    source_width = target_width;
                    source_height = target_height;
                }
                _ => {
                    return Err(
                        "compiled filter pass kind does not match its runtime binding plan",
                    );
                }
            }
        }

        if !used_direct_spatial_output && let Some(blit_source_slot) = self.blit_source_scratch_slot
        {
            let Some(blit_pipeline) = &self.blit_pipeline else {
                return Err("final blit pipeline missing after setup");
            };
            let Some(blit_bind_group_layout) = &self.blit_bind_group_layout else {
                return Err("final blit bind group layout missing after setup");
            };
            let Some(blit_source_view) = self.scratch_views[blit_source_slot].as_ref() else {
                return Err("final blit source scratch view missing");
            };

            if self.blit_bind_group.is_none() {
                self.blit_bind_group =
                    Some(input.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("filter final blit bind group"),
                        layout: blit_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(blit_source_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            },
                        ],
                    }));
            }
            let Some(blit_bind_group) = self.blit_bind_group.as_ref() else {
                return Err("final blit bind group missing after creation");
            };

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("filter final blit pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &output.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                render_pass.set_pipeline(blit_pipeline);
                render_pass.set_bind_group(0, blit_bind_group, &[]);
                render_pass.draw(0..6, 0..1);
            }
        }

        self.target_params_dirty = false;
        #[cfg(test)]
        {
            self.last_render_used_direct_output = used_direct_spatial_output;
        }
        Ok(needs_redraw)
    }

    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        self.filter.output_size(input_width, input_height)
    }

    fn redraw_hint(&self) -> bool {
        self.target_params_dirty || self.animation_state.has_active_animation
    }
}
#[allow(private_bounds)]
impl<F: Filter> FilterAdapter<F> {
    fn create_color_pipeline(
        ctx: &EffectContext,
        fragments: &str,
        target_format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
        let preamble = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/shared/fragment_preamble.wgsl"
        ));
        let postamble = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/shared/fragment_postamble.wgsl"
        ));

        let mut shader_source = alloc::string::String::from(preamble);
        shader_source.push_str(fragments);
        shader_source.push_str(postamble);

        let shader =
            ctx.shader_cache
                .get_or_create(ctx.device, "filter color shader", &shader_source);

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("filter color bind group layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("filter color pipeline layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("filter color pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader.as_ref(),
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader.as_ref(),
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: ctx.pipeline_cache,
            });

        (pipeline, bind_group_layout)
    }

    fn create_spatial_pipeline(
        ctx: &EffectContext,
        shader_source: &str,
        storage_format: wgpu::TextureFormat,
        original_input: bool,
    ) -> Result<(wgpu::ComputePipeline, wgpu::BindGroupLayout), &'static str> {
        let shader_source = specialize_spatial_shader(shader_source, storage_format)?;
        let shader =
            ctx.shader_cache
                .get_or_create(ctx.device, "filter spatial shader", &shader_source);

        let original_entry = wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let entries = if original_input {
            alloc::vec![
                spatial_source_layout_entry(),
                spatial_target_layout_entry(storage_format),
                spatial_uniform_layout_entry(),
                original_entry,
            ]
        } else {
            alloc::vec![
                spatial_source_layout_entry(),
                spatial_target_layout_entry(storage_format),
                spatial_uniform_layout_entry(),
            ]
        };

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("filter spatial bind group layout"),
                    entries: &entries,
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("filter spatial pipeline layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("filter spatial pipeline"),
                layout: Some(&pipeline_layout),
                module: shader.as_ref(),
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: ctx.pipeline_cache,
            });

        Ok((pipeline, bind_group_layout))
    }

    fn create_blit_pipeline(ctx: &EffectContext) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
        let shader = ctx.shader_cache.get_or_create(
            ctx.device,
            "filter blit shader",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/shared/blit.wgsl"
            )),
        );

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("filter blit bind group layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                            count: None,
                        },
                    ],
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("filter blit pipeline layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("filter blit pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader.as_ref(),
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader.as_ref(),
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.output_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: ctx.pipeline_cache,
            });

        (pipeline, bind_group_layout)
    }
}

fn build_color_uniform_data(
    width: u32,
    height: u32,
    params: &[f32],
) -> [f32; FILTER_UNIFORM_WORDS] {
    let mut data = [0.0f32; FILTER_UNIFORM_WORDS];
    data[0] = u32_to_f32(width);
    data[1] = u32_to_f32(height);
    for (idx, value) in params.iter().enumerate().take(MAX_FILTER_PARAMS) {
        data[4 + idx] = *value;
    }
    data
}

fn build_spatial_uniform_data(
    output_width: u32,
    output_height: u32,
    input_width: u32,
    input_height: u32,
    params: &[f32],
) -> [f32; FILTER_UNIFORM_WORDS] {
    let mut data = [0.0f32; FILTER_UNIFORM_WORDS];
    data[0] = u32_to_f32(output_width);
    data[1] = u32_to_f32(output_height);
    data[2] = u32_to_f32(input_width);
    data[3] = u32_to_f32(input_height);
    for (idx, value) in params.iter().enumerate().take(MAX_FILTER_PARAMS) {
        data[4 + idx] = *value;
    }
    data
}

const fn spatial_source_layout_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

const fn spatial_target_layout_entry(
    storage_format: wgpu::TextureFormat,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: storage_format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

const fn spatial_uniform_layout_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn u32_to_f32(value: u32) -> f32 {
    value
        .to_f32()
        .unwrap_or_else(|| panic!("value {value} must fit into f32"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;

    struct TestGpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter_info: wgpu::AdapterInfo,
        rgba8_storage: bool,
        rgba16_storage: bool,
        shader_cache: crate::ShaderCache,
    }

    fn create_test_device() -> TestGpu {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("filter GPU tests require a high-performance adapter");
        let adapter_info = adapter.get_info();
        let rgba8_storage = adapter
            .get_texture_format_features(wgpu::TextureFormat::Rgba8Unorm)
            .allowed_usages
            .contains(wgpu::TextureUsages::STORAGE_BINDING);
        let rgba16_storage = adapter
            .get_texture_format_features(wgpu::TextureFormat::Rgba16Float)
            .allowed_usages
            .contains(wgpu::TextureUsages::STORAGE_BINDING);
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("filter GPU tests require a working device");
        TestGpu {
            device,
            queue,
            adapter_info,
            rgba8_storage,
            rgba16_storage,
            shader_cache: crate::ShaderCache::new(),
        }
    }

    fn assert_f32_eq(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= f32::EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    fn readback_rgba8_pixel(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> [u8; 4] {
        const BYTES_PER_PIXEL: u32 = 4;
        const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let unpadded_bpr = width * BYTES_PER_PIXEL;
        let padded_bpr = unpadded_bpr.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
        let copy_size = u64::from(padded_bpr) * u64::from(height);

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("filter gpu test readback buffer"),
            size: copy_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("filter gpu test readback encoder"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bpr),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let map_result = rx
            .recv()
            .expect("map callback should return a completion result");
        map_result.expect("buffer mapping should succeed");

        let mapped = slice.get_mapped_range();
        let pixel = [mapped[0], mapped[1], mapped[2], mapped[3]];
        drop(mapped);
        buffer.unmap();
        pixel
    }

    fn readback_rgba8_image(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        const BYTES_PER_PIXEL: u32 = 4;
        const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let unpadded_bpr = width * BYTES_PER_PIXEL;
        let padded_bpr = unpadded_bpr.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
        let copy_size = u64::from(padded_bpr) * u64::from(height);

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("filter gpu test full readback buffer"),
            size: copy_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("filter gpu test full readback encoder"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bpr),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let map_result = rx
            .recv()
            .expect("map callback should return a completion result");
        map_result.expect("buffer mapping should succeed");

        let mapped = slice.get_mapped_range();
        let mut out = vec![0u8; (width * height * BYTES_PER_PIXEL) as usize];
        for row in 0..height as usize {
            let src_start = row * padded_bpr as usize;
            let src_end = src_start + unpadded_bpr as usize;
            let dst_start = row * unpadded_bpr as usize;
            let dst_end = dst_start + unpadded_bpr as usize;
            out[dst_start..dst_end].copy_from_slice(&mapped[src_start..src_end]);
        }
        drop(mapped);
        buffer.unmap();
        out
    }

    fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
        let img = RgbaImage::from_raw(width, height, rgba.to_vec())
            .expect("rgba buffer length should match dimensions");
        img.save(path).expect("failed to save png");
    }

    #[derive(Clone, Copy)]
    struct FilterReadbackSize {
        input: (u32, u32),
        output: (u32, u32),
    }

    fn rgba_len(width: u32, height: u32) -> usize {
        usize::try_from(u64::from(width) * u64::from(height) * 4)
            .expect("rgba dimensions fit usize")
    }

    fn rgba_index(width: u32, x: u32, y: u32) -> usize {
        usize::try_from((u64::from(y) * u64::from(width) + u64::from(x)) * 4)
            .expect("rgba index fits usize")
    }

    fn scale_to_u8(numerator: u32, denominator: u32) -> u8 {
        let value = u64::from(numerator) * 255 / u64::from(denominator.max(1));
        u8::try_from(value).expect("scaled channel fits u8")
    }

    fn clamp_i32_to_u8(value: i32) -> u8 {
        u8::try_from(value.clamp(0, i32::from(u8::MAX))).expect("clamped channel fits u8")
    }

    fn create_test_input_rgba(width: u32, height: u32) -> Vec<u8> {
        let mut data = vec![0u8; rgba_len(width, height)];
        let max_x = width.saturating_sub(1).max(1);
        let max_y = height.saturating_sub(1).max(1);
        let min_dimension = i64::from(width.min(height));
        let inner_edge_radius = min_dimension * 28 / 100;
        let outer_edge_radius = min_dimension * 32 / 100;
        let inner_edge_radius_sq = inner_edge_radius * inner_edge_radius;
        let outer_edge_radius_sq = outer_edge_radius * outer_edge_radius;
        let center_x = i64::from(width) / 2;
        let center_y = i64::from(height) / 2;

        for y in 0..height {
            for x in 0..width {
                let idx = rgba_index(width, x, y);
                let x_gradient = scale_to_u8(x, max_x);
                let y_gradient = scale_to_u8(y, max_y);
                let checker = if ((x / 16) + (y / 16)) % 2 == 0 {
                    32
                } else {
                    -32
                };
                let dx = i64::from(x) - center_x;
                let dy = i64::from(y) - center_y;
                let ring_sq = dx * dx + dy * dy;
                let edge = if ring_sq > inner_edge_radius_sq && ring_sq < outer_edge_radius_sq {
                    80
                } else {
                    0
                };
                let inverse_gradient = u8::try_from(
                    u64::from(max_x - x) * u64::from(max_y - y) * 255
                        / (u64::from(max_x) * u64::from(max_y)),
                )
                .expect("inverse channel fits u8");

                let r = clamp_i32_to_u8(i32::from(x_gradient) + checker + edge);
                let g = clamp_i32_to_u8(i32::from(y_gradient) - checker + edge);
                let b = clamp_i32_to_u8(i32::from(inverse_gradient) + edge);

                data[idx] = r;
                data[idx + 1] = g;
                data[idx + 2] = b;
                data[idx + 3] = 255;
            }
        }
        data
    }

    fn run_filter_and_readback<G: Effect>(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shader_cache: &crate::ShaderCache,
        input_texture: &wgpu::Texture,
        size: FilterReadbackSize,
        mut filter: G,
    ) -> Vec<u8> {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let (input_width, input_height) = size.input;
        let (output_width, output_height) = size.output;
        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter gallery output"),
            size: wgpu::Extent3d {
                width: output_width,
                height: output_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let ctx = EffectContext {
            device,
            queue,
            input_format: format,
            output_format: format,
            pipeline_cache: None,
            shader_cache,
        };
        pollster::block_on(filter.setup(&ctx)).expect("test filter setup should succeed");

        let input = EffectInput {
            device,
            queue,
            texture: input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width: input_width,
            height: input_height,
            timing: EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, 0),
        };
        let output = EffectOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width: output_width,
            height: output_height,
        };

        let _ = filter.render(&input, &output);
        readback_rgba8_image(device, queue, &output_texture, output_width, output_height)
    }

    #[test]
    fn reject_empty_stage_graph() {
        let err = fuse_stages(&[]).expect_err("empty stage graph should fail");
        assert_eq!(err, "filter graph produced no stages");
    }

    #[test]
    fn fuse_adjacent_color_stages() {
        let filter = Chain {
            first: crate::filters::Brightness(0.2f32),
            second: Chain {
                first: crate::filters::Contrast(1.1f32),
                second: crate::filters::Invert,
            },
        };

        let stages = collect_filter_stages(&filter);
        let passes = fuse_stages(&stages).expect("fuse should succeed");

        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].param_offset, 0);
        assert_eq!(passes[0].param_count, 2);
        assert!(matches!(passes[0].kind, PlannedPassKind::Color { .. }));
    }

    #[test]
    fn keep_spatial_boundaries() {
        let filter = Chain {
            first: crate::filters::Blur(2.0f32),
            second: Chain {
                first: crate::filters::Brightness(0.1f32),
                second: crate::filters::Sharpen(0.8f32),
            },
        };

        let stages = collect_filter_stages(&filter);
        let passes = fuse_stages(&stages).expect("fuse should succeed");

        assert_eq!(passes.len(), 4);
        assert!(matches!(passes[0].kind, PlannedPassKind::Spatial { .. }));
        assert!(matches!(passes[1].kind, PlannedPassKind::Spatial { .. }));
        assert!(matches!(passes[2].kind, PlannedPassKind::Color { .. }));
        assert!(matches!(passes[3].kind, PlannedPassKind::Spatial { .. }));
    }

    #[test]
    fn preserve_param_offsets_across_fused_and_spatial_passes() {
        let filter = Chain {
            first: crate::filters::Brightness(0.2f32),
            second: Chain {
                first: crate::filters::Contrast(1.1f32),
                second: Chain {
                    first: crate::filters::Blur(2.0f32),
                    second: crate::filters::Sepia(0.7f32),
                },
            },
        };

        let stages = collect_filter_stages(&filter);
        let passes = fuse_stages(&stages).expect("fuse should succeed");

        assert_eq!(passes.len(), 4);
        assert_eq!(passes[0].param_offset, 0);
        assert_eq!(passes[0].param_count, 2);
        assert_eq!(passes[1].param_offset, 2);
        assert_eq!(passes[1].param_count, 1);
        assert_eq!(passes[2].param_offset, 3);
        assert_eq!(passes[2].param_count, 1);
        assert_eq!(passes[3].param_offset, 4);
        assert_eq!(passes[3].param_count, 1);
    }

    #[test]
    fn runtime_binding_plan_tracks_scratch_ping_pong_and_blit_source() {
        let filter = Chain {
            first: crate::filters::Blur(2.0f32),
            second: Chain {
                first: crate::filters::Brightness(0.2f32),
                second: crate::filters::Sharpen(0.8f32),
            },
        };

        let stages = collect_filter_stages(&filter);
        let passes = fuse_stages(&stages).expect("fuse should succeed");

        let (plans, blit_source) =
            plan_runtime_bindings(&passes).expect("runtime binding planning should succeed");

        assert_eq!(plans.len(), 4);
        assert_eq!(
            plans[0],
            PassBindingPlan::Spatial {
                source: PassTextureSource::Input,
                target_scratch: 0
            }
        );
        assert_eq!(
            plans[1],
            PassBindingPlan::Spatial {
                source: PassTextureSource::Scratch(0),
                target_scratch: 1
            }
        );
        assert_eq!(
            plans[2],
            PassBindingPlan::Color {
                source: PassTextureSource::Scratch(1),
                target: ColorTarget::Scratch(0)
            }
        );
        assert_eq!(
            plans[3],
            PassBindingPlan::Spatial {
                source: PassTextureSource::Scratch(0),
                target_scratch: 1
            }
        );
        assert_eq!(blit_source, Some(1));
    }

    #[test]
    fn runtime_binding_plan_for_fused_color_chain_uses_direct_output() {
        let filter = Chain {
            first: crate::filters::Brightness(0.1f32),
            second: Chain {
                first: crate::filters::Contrast(1.2f32),
                second: crate::filters::Invert,
            },
        };

        let stages = collect_filter_stages(&filter);
        let passes = fuse_stages(&stages).expect("fuse should succeed");
        let (plans, blit_source) =
            plan_runtime_bindings(&passes).expect("runtime binding planning should succeed");

        assert_eq!(
            plans,
            vec![PassBindingPlan::Color {
                source: PassTextureSource::Input,
                target: ColorTarget::Output
            }]
        );
        assert_eq!(blit_source, None);
    }

    type HugeParams = (
        (
            (
                (((([f32; 8], [f32; 8]), [f32; 8]), [f32; 8]), [f32; 8]),
                [f32; 8],
            ),
            [f32; 8],
        ),
        ([f32; 8], [f32; 8]),
    );

    #[derive(Debug, Clone, Copy)]
    struct HugeFilter;

    impl Filter for HugeFilter {
        const COLOR_ONLY: bool = true;
        type Params = HugeParams;
        type Fragments = &'static str;

        fn params(&self) -> Self::Params {
            (
                (
                    (
                        (((([0.0; 8], [0.0; 8]), [0.0; 8]), [0.0; 8]), [0.0; 8]),
                        [0.0; 8],
                    ),
                    [0.0; 8],
                ),
                ([0.0; 8], [0.0; 8]),
            )
        }

        fn fragments(&self) -> Self::Fragments {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/color/adjustment/brightness.wgsl"
            ))
        }

        fn collect_stages<C: StageCollector>(&self, c: &mut C) {
            c.color_fragment(
                include_str!("shaders/color/adjustment/brightness.wgsl"),
                <Self::Params as ParamArray>::LEN,
            );
        }
    }

    #[test]
    fn fast_fail_when_param_count_exceeds_uniform_limit() {
        const {
            assert!(<HugeParams as ParamArray>::LEN > MAX_FILTER_PARAMS);
        }

        let mut adapter = FilterAdapter::new(HugeFilter);
        let needs_redraw = adapter.update_interpolated_params(Duration::ZERO);
        assert!(!needs_redraw);
    }

    #[test]
    fn prefer_hdr_scratch_format_for_hdr_input_or_output() {
        assert_eq!(
            preferred_scratch_format(
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureFormat::Bgra8Unorm
            ),
            wgpu::TextureFormat::Rgba16Float
        );
        assert_eq!(
            preferred_scratch_format(
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::TextureFormat::Rgba16Float
            ),
            wgpu::TextureFormat::Rgba16Float
        );
        assert_eq!(
            preferred_scratch_format(
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::TextureFormat::Bgra8Unorm
            ),
            wgpu::TextureFormat::Rgba8Unorm
        );
    }

    #[test]
    fn specialize_spatial_shader_rewrites_storage_format_token() {
        let src =
            "@group(0) @binding(2) var out_tex: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;";
        let shader = specialize_spatial_shader(src, wgpu::TextureFormat::Rgba16Float)
            .expect("specialization should succeed");
        let shader_text = shader.as_str();
        assert!(shader_text.contains("texture_storage_2d<rgba16float, write>"));
        assert!(!shader_text.contains(SPATIAL_OUTPUT_FORMAT_TOKEN));
    }

    #[test]
    fn specialize_spatial_shader_rewrites_workgroup_tokens() {
        let src = "@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)";
        let shader = specialize_spatial_shader(src, wgpu::TextureFormat::Rgba8Unorm)
            .expect("specialization should succeed");
        let expected = alloc::format!(
            "@compute @workgroup_size({SPATIAL_WORKGROUP_X}, {SPATIAL_WORKGROUP_Y})"
        );
        assert_eq!(shader, expected);
    }

    #[test]
    fn spatial_uniform_data_uses_vec4_packed_layout() {
        let data = build_spatial_uniform_data(320, 240, 640, 480, &[2.0, 3.0]);
        assert_eq!(data.len(), 4 + MAX_FILTER_PARAMS);
        assert_f32_eq(data[0], 320.0);
        assert_f32_eq(data[1], 240.0);
        assert_f32_eq(data[2], 640.0);
        assert_f32_eq(data[3], 480.0);
        assert_f32_eq(data[4], 2.0);
        assert_f32_eq(data[5], 3.0);
    }

    #[test]
    fn color_uniform_data_uses_fixed_array_layout() {
        let data = build_color_uniform_data(800, 600, &[1.0, 2.0]);
        assert_eq!(data.len(), 4 + MAX_FILTER_PARAMS);
        assert_f32_eq(data[0], 800.0);
        assert_f32_eq(data[1], 600.0);
        assert_f32_eq(data[4], 1.0);
        assert_f32_eq(data[5], 2.0);
    }

    #[test]
    fn hdr_color_fragments_do_not_clamp_to_unit_range() {
        let brightness = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/color/adjustment/brightness.wgsl"
        ));
        let contrast = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/color/adjustment/contrast.wgsl"
        ));
        let sharpen = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/image/convolution/sharpen.wgsl"
        ));

        assert!(!brightness.contains("clamp("));
        assert!(!contrast.contains("clamp("));
        assert!(!sharpen.contains("clamp(result.rgb"));
    }

    #[test]
    fn spatial_shaders_use_dynamic_storage_format_and_texture_load() {
        let blur_horizontal = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/image/blur/blur_horizontal.wgsl"
        ));
        let blur_vertical = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/image/blur/blur_vertical.wgsl"
        ));
        let sharpen = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/image/convolution/sharpen.wgsl"
        ));

        for shader in [blur_horizontal, blur_vertical, sharpen] {
            assert!(shader.contains(SPATIAL_OUTPUT_FORMAT_TOKEN));
            assert!(shader.contains("textureLoad("));
            assert!(!shader.contains("input_sampler"));
        }
    }

    #[test]
    fn hdr_policy_builders_update_adapter_policy() {
        let adapter = FilterAdapter::new(crate::filters::Blur(2.0f32));
        assert_eq!(adapter.hdr_policy, HdrPolicy::PreferHdr);

        let adapter = adapter.require_hdr();
        assert_eq!(adapter.hdr_policy, HdrPolicy::RequireHdr);

        let adapter = adapter.force_ldr();
        assert_eq!(adapter.hdr_policy, HdrPolicy::ForceLdr);
    }

    #[test]
    fn then_preserves_hdr_policy() {
        let adapter = FilterAdapter::new(crate::filters::Blur(2.0f32)).require_hdr();
        let chained = adapter.then(crate::filters::Sharpen(1.0f32));
        assert_eq!(chained.hdr_policy, HdrPolicy::RequireHdr);
    }

    #[test]
    fn gpu_color_filter_executes_and_writes_output() {
        let gpu = create_test_device();
        let device = &gpu.device;
        let queue = &gpu.queue;

        let width = 8;
        let height = 8;
        let format = wgpu::TextureFormat::Rgba8Unorm;

        let input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter gpu color input"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let input_data = vec![0u8; (width * height * 4) as usize];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &input_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter gpu color output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let mut adapter = FilterAdapter::new(crate::filters::Brightness(0.25f32));
        let ctx = EffectContext {
            device,
            queue,
            input_format: format,
            output_format: format,
            pipeline_cache: None,
            shader_cache: &gpu.shader_cache,
        };
        pollster::block_on(Effect::setup(&mut adapter, &ctx))
            .expect("test filter setup should succeed");

        let input = EffectInput {
            device,
            queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
            timing: EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, 0),
        };
        let output = EffectOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
        };

        let needs_redraw = Effect::render(&mut adapter, &input, &output);
        assert_eq!(needs_redraw, Ok(false));

        let pixel = readback_rgba8_pixel(device, queue, &output_texture, width, height);
        assert!(pixel[0] > 0 || pixel[1] > 0 || pixel[2] > 0);
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "GPU integration test keeps setup, render, and readback assertions in one scenario"
    )]
    fn gpu_spatial_filter_supports_mismatched_input_output_sizes() {
        let gpu = create_test_device();
        let device = &gpu.device;
        let queue = &gpu.queue;

        let in_width = 6;
        let in_height = 4;
        let out_width = 11;
        let out_height = 7;
        let format = wgpu::TextureFormat::Rgba8Unorm;

        let input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter gpu spatial input"),
            size: wgpu::Extent3d {
                width: in_width,
                height: in_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let input_data = vec![255u8; (in_width * in_height * 4) as usize];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &input_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(in_width * 4),
                rows_per_image: Some(in_height),
            },
            wgpu::Extent3d {
                width: in_width,
                height: in_height,
                depth_or_array_layers: 1,
            },
        );

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter gpu spatial output"),
            size: wgpu::Extent3d {
                width: out_width,
                height: out_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let mut adapter = FilterAdapter::new(crate::filters::Blur(1.0f32));
        let ctx = EffectContext {
            device,
            queue,
            input_format: format,
            output_format: format,
            pipeline_cache: None,
            shader_cache: &gpu.shader_cache,
        };
        pollster::block_on(Effect::setup(&mut adapter, &ctx))
            .expect("test filter setup should succeed");
        assert!(
            !adapter.has_setup_error(),
            "spatial filter setup failed on adapter {} ({:?}); rgba8_storage={}, rgba16f_storage={}, error={:?}",
            gpu.adapter_info.name,
            gpu.adapter_info.backend,
            gpu.rgba8_storage,
            gpu.rgba16_storage,
            adapter.setup_error,
        );
        assert!(
            !adapter.passes.is_empty(),
            "spatial passes should be compiled"
        );

        let input = EffectInput {
            device,
            queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width: in_width,
            height: in_height,
            timing: EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, 0),
        };
        let output = EffectOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width: out_width,
            height: out_height,
        };

        let needs_redraw = Effect::render(&mut adapter, &input, &output);
        assert_eq!(needs_redraw, Ok(false));

        let pixel = readback_rgba8_pixel(device, queue, &output_texture, out_width, out_height);
        assert!(
            pixel.iter().any(|&c| c > 0),
            "spatial output should not be all zeros, got {pixel:?}"
        );
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "GPU integration test keeps setup, render, and fallback assertions in one scenario"
    )]
    fn gpu_spatial_filter_uses_direct_output_when_storage_binding_is_available() {
        let gpu = create_test_device();
        let device = &gpu.device;
        let queue = &gpu.queue;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let width = 8;
        let height = 8;

        let input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter direct output input"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let input_data = vec![255u8; (width * height * 4) as usize];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &input_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter direct output texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let mut adapter = FilterAdapter::new(crate::filters::Blur(1.0f32));
        let ctx = EffectContext {
            device,
            queue,
            input_format: format,
            output_format: format,
            pipeline_cache: None,
            shader_cache: &gpu.shader_cache,
        };
        pollster::block_on(Effect::setup(&mut adapter, &ctx))
            .expect("test filter setup should succeed");
        assert!(
            !adapter.has_setup_error(),
            "spatial filter setup failed: {:?}",
            adapter.setup_error
        );
        let expected_direct_output = adapter.final_spatial_output.is_some();

        let input = EffectInput {
            device,
            queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
            timing: EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, 0),
        };
        let output = EffectOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
        };

        let _ = Effect::render(&mut adapter, &input, &output);
        assert_eq!(
            adapter.last_render_used_direct_output(),
            expected_direct_output
        );
        if expected_direct_output {
            assert_eq!(
                adapter.allocated_scratch_slots(),
                [true, false],
                "direct output path should avoid allocating the final scratch slot"
            );
        } else {
            assert_eq!(
                adapter.allocated_scratch_slots(),
                [true, true],
                "fallback path should preserve both blur scratch slots"
            );
        }
    }

    #[test]
    fn gpu_spatial_filter_falls_back_when_output_lacks_storage_binding_usage() {
        let gpu = create_test_device();
        let device = &gpu.device;
        let queue = &gpu.queue;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let width = 8;
        let height = 8;

        let input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter fallback output input"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let input_data = vec![255u8; (width * height * 4) as usize];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &input_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter fallback output texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let mut adapter = FilterAdapter::new(crate::filters::Blur(1.0f32));
        let ctx = EffectContext {
            device,
            queue,
            input_format: format,
            output_format: format,
            pipeline_cache: None,
            shader_cache: &gpu.shader_cache,
        };
        pollster::block_on(Effect::setup(&mut adapter, &ctx))
            .expect("test filter setup should succeed");
        assert!(
            !adapter.has_setup_error(),
            "spatial filter setup failed: {:?}",
            adapter.setup_error
        );

        let input = EffectInput {
            device,
            queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
            timing: EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, 0),
        };
        let output = EffectOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
        };

        let _ = Effect::render(&mut adapter, &input, &output);
        assert!(!adapter.last_render_used_direct_output());
        assert_eq!(
            adapter.allocated_scratch_slots(),
            [true, true],
            "non-storage output must keep both blur scratch slots for fallback blit"
        );
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "gallery export intentionally enumerates every filter case in one visual artifact generator"
    )]
    fn gpu_export_filter_gallery_images() {
        let gpu = create_test_device();
        let device = &gpu.device;
        let queue = &gpu.queue;

        let input_width = 256;
        let input_height = 256;
        let output_dir = PathBuf::from("/tmp/waterui_filter_gallery");
        fs::create_dir_all(&output_dir).expect("failed to create output directory");

        let input_rgba = create_test_input_rgba(input_width, input_height);
        write_png(
            &output_dir.join("input.png"),
            input_width,
            input_height,
            &input_rgba,
        );

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter gallery input"),
            size: wgpu::Extent3d {
                width: input_width,
                height: input_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &input_rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(input_width * 4),
                rows_per_image: Some(input_height),
            },
            wgpu::Extent3d {
                width: input_width,
                height: input_height,
                depth_or_array_layers: 1,
            },
        );

        macro_rules! export_filter {
            ($name:literal, $ow:expr, $oh:expr, $filter:expr) => {{
                let result = run_filter_and_readback(
                    device,
                    queue,
                    &gpu.shader_cache,
                    &input_texture,
                    FilterReadbackSize {
                        input: (input_width, input_height),
                        output: ($ow, $oh),
                    },
                    $filter,
                );
                write_png(&output_dir.join($name), $ow, $oh, &result);
            }};
        }

        export_filter!(
            "brightness.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Brightness(0.2f32))
        );
        export_filter!(
            "contrast.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Contrast(1.4f32))
        );
        export_filter!(
            "saturation.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Saturation(1.8f32))
        );
        export_filter!(
            "grayscale.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Grayscale(1.0f32))
        );
        export_filter!(
            "hue_rotation.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::HueRotation(120.0f32))
        );
        export_filter!(
            "sepia.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Sepia(1.0f32))
        );
        export_filter!(
            "invert.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Invert)
        );
        export_filter!(
            "blur.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Blur(3.0f32))
        );
        export_filter!(
            "sharpen.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Sharpen(1.5f32))
        );
        export_filter!(
            "chain_blur_brightness.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Blur(2.0f32))
                .then(crate::filters::Brightness(0.15f32))
                .then(crate::filters::Contrast(1.2f32))
        );
        export_filter!(
            "blur_resized_384x216.png",
            384,
            216,
            FilterAdapter::new(crate::filters::Blur(2.0f32))
        );

        // P9 additions — verify each new spatial / preset filter actually
        // round-trips through the wgpu pipeline end-to-end.
        export_filter!(
            "sobel.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Sobel)
        );
        export_filter!(
            "prewitt.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Prewitt)
        );
        export_filter!(
            "median3x3.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Median3x3)
        );
        export_filter!(
            "morphology_min.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::MorphologyMin)
        );
        export_filter!(
            "morphology_max.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::MorphologyMax)
        );
        export_filter!(
            "morphology_gradient.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::MorphologyGradient)
        );
        // 3x3 sharpen kernel: identity * 5 minus the four neighbours.
        export_filter!(
            "convolution3x3_sharpen.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Convolution3x3([
                0.0f32, -1.0, 0.0, -1.0, 5.0, -1.0, 0.0, -1.0, 0.0,
            ]))
        );
        // 5x5 identity (centre = 1, rest = 0). Output should match input.
        export_filter!("convolution5x5_identity.png", input_width, input_height, {
            let mut kernel = [0.0f32; 25];
            kernel[12] = 1.0;
            FilterAdapter::new(crate::filters::Convolution5x5(kernel))
        });
        export_filter!(
            "photo_effect_mono.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::PhotoEffectMono)
        );
        export_filter!(
            "photo_effect_noir.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::PhotoEffectNoir)
        );
        export_filter!(
            "photo_effect_chrome.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::PhotoEffectChrome)
        );
        export_filter!(
            "photo_effect_instant.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::PhotoEffectInstant)
        );
        export_filter!(
            "photo_effect_fade.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::PhotoEffectFade)
        );
        export_filter!(
            "photo_effect_process.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::PhotoEffectProcess)
        );
        export_filter!(
            "photo_effect_tonal.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::PhotoEffectTonal)
        );
        export_filter!(
            "photo_effect_transfer.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::PhotoEffectTransfer)
        );
        // Mixed chain: photo preset chained with tunable color filters.
        export_filter!(
            "chain_chrome_brightness_contrast.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::PhotoEffectChrome)
                .then(crate::filters::Brightness(0.05f32))
                .then(crate::filters::Contrast(1.1f32))
        );
        // Mixed chain: edge detection feeding a tonal preset.
        export_filter!(
            "chain_sobel_then_tonal.png",
            input_width,
            input_height,
            FilterAdapter::new(crate::filters::Sobel).then(crate::filters::PhotoEffectTonal)
        );
    }
}
