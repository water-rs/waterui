//! GPU filter processing for captured view content.
//!
//! This module provides the `GpuFilter` trait for implementing GPU-based filters
//! that process captured view textures. Native backends capture child views to
//! textures, pass them to Rust for GPU processing via wgpu.
//!
//! # Architecture
//!
//! 1. Native backend captures child view to a texture
//! 2. Native calls into Rust with input texture handle
//! 3. Rust applies filter pipeline via wgpu
//! 4. Result written to output texture for native to display
//!
//! # Animation Support
//!
//! Filters support Rust-side animation interpolation. When a reactive value
//! changes with animation metadata, the filter smoothly interpolates between
//! values and signals `needs_redraw = true` until the animation completes.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::future::Future;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Instant;

use filtrate_core::{Chain, Filter, ParamArray};
use nami::{Computed, Signal, SignalExt as _};
use waterui_core::animation::{Animation, AnimationTrack};
use waterui_core::layout::StretchAxis;
use waterui_core::metadata::MetadataKey;
use waterui_core::{AnyView, Environment, IntoSignalF32, Metadata, View};

use crate::gpu_surface::SetupFuture;

/// GPU resources provided to the filter during setup.
///
/// Contains references to the wgpu device, queue, and texture formats.
pub struct FilterContext<'a> {
    /// The wgpu device for creating GPU resources.
    pub device: &'a wgpu::Device,
    /// The wgpu queue for submitting commands.
    pub queue: &'a wgpu::Queue,
    /// The texture format of the input (captured view).
    pub input_format: wgpu::TextureFormat,
    /// The texture format of the output.
    pub output_format: wgpu::TextureFormat,
    /// Optional pipeline cache for faster pipeline creation.
    pub pipeline_cache: Option<&'a wgpu::PipelineCache>,
}

impl core::fmt::Debug for FilterContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilterContext")
            .field("input_format", &self.input_format)
            .field("output_format", &self.output_format)
            .finish_non_exhaustive()
    }
}

/// Input texture provided during filter rendering.
pub struct FilterInput<'a> {
    /// The wgpu device.
    pub device: &'a wgpu::Device,
    /// The wgpu queue.
    pub queue: &'a wgpu::Queue,
    /// The captured view's texture.
    pub texture: &'a wgpu::Texture,
    /// A view into the input texture.
    pub view: wgpu::TextureView,
    /// The texture format.
    pub format: wgpu::TextureFormat,
    /// Width of the input texture in pixels.
    pub width: u32,
    /// Height of the input texture in pixels.
    pub height: u32,
}

impl core::fmt::Debug for FilterInput<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilterInput")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Output texture provided during filter rendering.
pub struct FilterOutput<'a> {
    /// The wgpu device.
    pub device: &'a wgpu::Device,
    /// The wgpu queue.
    pub queue: &'a wgpu::Queue,
    /// The output texture to write to.
    pub texture: &'a wgpu::Texture,
    /// A view into the output texture.
    pub view: wgpu::TextureView,
    /// The texture format.
    pub format: wgpu::TextureFormat,
    /// Width of the output texture in pixels.
    pub width: u32,
    /// Height of the output texture in pixels.
    pub height: u32,
}

impl core::fmt::Debug for FilterOutput<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilterOutput")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Trait for GPU filter processors.
///
/// Implement this trait to create custom GPU filters that process captured
/// view textures. The filter receives input and output textures with their
/// dimensions, allowing for effects that change output size.
///
/// # Async Setup
///
/// The `setup` method returns a future, allowing async initialization.
/// For sync filters, return `async {}` after doing sync work.
///
/// # Animation Support
///
/// The `render` method returns a boolean indicating whether another frame
/// is needed (for animations). Return `true` if an animation is in progress.
pub trait GpuFilter: 'static {
    /// Called once when GPU resources are ready.
    ///
    /// Use this to create pipelines, bind groups, samplers, and other
    /// GPU resources that persist across frames.
    fn setup(&mut self, ctx: &FilterContext) -> impl Future<Output = ()>;

    /// Called each frame to apply the filter.
    ///
    /// Read from `input.texture`/`input.view` and write to `output.texture`/`output.view`.
    /// Input and output may have different dimensions.
    ///
    /// Returns `true` if another frame is needed (animation in progress).
    fn render(&mut self, input: &FilterInput, output: &FilterOutput) -> bool;

    /// Snapshot reactive target values before render dispatch.
    ///
    /// Native backends call this on the UI thread before scheduling render on a
    /// background queue. Filters without reactive sources can keep the default.
    fn sync_targets(&mut self) {}

    /// Whether the filter has pending state that requires another render pass.
    ///
    /// This is used by native backends to keep on-demand rendering responsive
    /// when reactive parameters change without layout updates.
    fn redraw_hint(&self) -> bool {
        false
    }
}

/// Object-safe trait for type-erased GPU filters.
pub(crate) trait GpuFilterImpl: 'static {
    fn setup<'a>(&'a mut self, ctx: &'a FilterContext<'a>) -> SetupFuture<'a>;
    fn render(&mut self, input: &FilterInput, output: &FilterOutput) -> bool;
    fn sync_targets(&mut self);
    fn redraw_hint(&self) -> bool;
}

impl<T: GpuFilter> GpuFilterImpl for T {
    fn setup<'a>(&'a mut self, ctx: &'a FilterContext<'a>) -> SetupFuture<'a> {
        Box::pin(GpuFilter::setup(self, ctx))
    }

    fn render(&mut self, input: &FilterInput, output: &FilterOutput) -> bool {
        GpuFilter::render(self, input, output)
    }

    fn sync_targets(&mut self) {
        GpuFilter::sync_targets(self);
    }

    fn redraw_hint(&self) -> bool {
        GpuFilter::redraw_hint(self)
    }
}

/// Type-erased filter for FFI boundary.
///
/// This wraps a `Box<dyn GpuFilterImpl>` and implements `MetadataKey`, allowing
/// it to be used with the `Metadata<T>` pattern.
pub struct AppliedFilter {
    filter: Box<dyn GpuFilterImpl>,
}

impl core::fmt::Debug for AppliedFilter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AppliedFilter").finish_non_exhaustive()
    }
}

impl MetadataKey for AppliedFilter {}

impl AppliedFilter {
    /// Create a new `AppliedFilter` from a GPU filter.
    pub fn new<F: GpuFilter>(filter: F) -> Self {
        Self {
            filter: Box::new(filter),
        }
    }

    /// Calls `setup` on the filter, returning a future that completes when ready.
    pub fn setup<'a>(&'a mut self, ctx: &'a FilterContext<'a>) -> SetupFuture<'a> {
        self.filter.setup(ctx)
    }

    /// Calls `render` on the filter.
    ///
    /// Returns `true` if another frame is needed (animation in progress).
    pub fn render(&mut self, input: &FilterInput, output: &FilterOutput) -> bool {
        self.filter.render(input, output)
    }

    /// Snapshot reactive target values before render dispatch.
    pub fn sync_targets(&mut self) {
        self.filter.sync_targets();
    }

    /// Query whether this filter needs a redraw even without layout changes.
    #[must_use]
    pub fn redraw_hint(&self) -> bool {
        self.filter.redraw_hint()
    }
}

/// Public filter wrapper returned by `ViewExt` APIs.
///
/// `Filtered` preserves the concrete content type for fluent chaining. Its `body()`
/// erases content to [`AnyView`] and yields [`FilteredView<F>`], which is the stable
/// backend hook node.
pub struct Filtered<V: View, F: GpuFilter> {
    content: V,
    filter: F,
}

impl<V: View, F: GpuFilter> core::fmt::Debug for Filtered<V, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Filtered").finish_non_exhaustive()
    }
}

impl<V: View, F: GpuFilter> Filtered<V, F> {
    /// Create a new filtered view with a `GpuFilter`.
    #[must_use]
    pub fn new(content: V, filter: F) -> Self {
        Self { content, filter }
    }
}

#[allow(private_bounds)]
impl<V: View, F: Filter + FilterGraph> Filtered<V, FilterAdapter<F>> {
    /// Chain another filter onto this view.
    ///
    /// Returns a new `Filtered` with the filters chained together.
    /// Consecutive color-only filters will be fused into a single GPU pass.
    ///
    /// # Example
    ///
    /// ```ignore
    /// my_view
    ///     .blur(10.0)
    ///     .then(Brightness(0.2))
    ///     .then(Contrast(1.5))
    /// ```
    #[must_use]
    pub fn then<F2: Filter + FilterGraph>(
        self,
        filter: F2,
    ) -> Filtered<V, FilterAdapter<Chain<F, F2>>> {
        Filtered::new(self.content, self.filter.then(filter))
    }

    /// Set HDR behavior policy for this filtered view.
    #[must_use]
    pub fn hdr_policy(mut self, policy: HdrPolicy) -> Self {
        self.filter = self.filter.hdr_policy(policy);
        self
    }

    /// Require HDR intermediates; setup fails if unsupported.
    #[must_use]
    pub fn require_hdr(self) -> Self {
        self.hdr_policy(HdrPolicy::RequireHdr)
    }

    /// Prefer HDR intermediates with automatic LDR fallback.
    #[must_use]
    pub fn prefer_hdr(self) -> Self {
        self.hdr_policy(HdrPolicy::PreferHdr)
    }

    /// Force LDR intermediates for compatibility/performance.
    #[must_use]
    pub fn force_ldr(self) -> Self {
        self.hdr_policy(HdrPolicy::ForceLdr)
    }
}

impl<V: View, F: GpuFilter> View for Filtered<V, F> {
    fn body(self, _env: &Environment) -> impl View {
        FilteredView::new(AnyView::new(self.content), self.filter)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.content.stretch_axis()
    }
}

/// Stable backend hook node for filter dispatch.
///
/// Backends can register concrete handlers such as `FilteredView<Blur>`. If a
/// backend does not hook this node, normal view expansion continues and falls back
/// to `Metadata<AppliedFilter>`.
pub struct FilteredView<F: GpuFilter> {
    content: AnyView,
    filter: F,
}

impl<F: GpuFilter> core::fmt::Debug for FilteredView<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilteredView").finish_non_exhaustive()
    }
}

impl<F: GpuFilter> FilteredView<F> {
    /// Create a backend hook node with type-erased content.
    #[must_use]
    pub fn new(content: AnyView, filter: F) -> Self {
        Self { content, filter }
    }

    /// Returns a reference to the wrapped content.
    #[must_use]
    pub fn content(&self) -> &AnyView {
        &self.content
    }

    /// Returns a reference to the wrapped filter.
    #[must_use]
    pub fn filter(&self) -> &F {
        &self.filter
    }

    /// Takes ownership of the wrapped content.
    #[must_use]
    pub fn into_content(self) -> AnyView {
        self.content
    }

    /// Takes ownership of the wrapped filter.
    #[must_use]
    pub fn into_filter(self) -> F {
        self.filter
    }
}

impl<F: GpuFilter> View for FilteredView<F> {
    fn body(self, _env: &Environment) -> impl View {
        Metadata::new(self.content, AppliedFilter::new(self.filter))
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.content.stretch_axis()
    }
}

// ============================================================================
// Filter Graph + Animation Tracking
// ============================================================================

const MAX_FILTER_PARAMS: usize = 64;
const FILTER_UNIFORM_WORDS: usize = 4 + MAX_FILTER_PARAMS;
const SPATIAL_OUTPUT_FORMAT_TOKEN: &str = "OUTPUT_STORAGE_FORMAT";

/// Policy for HDR behavior in filter pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdrPolicy {
    /// Require HDR-capable intermediate pipeline; fail setup if unavailable.
    RequireHdr,
    /// Prefer HDR intermediates and automatically downgrade to LDR when unsupported.
    PreferHdr,
    /// Force LDR intermediates even on HDR-capable devices.
    ForceLdr,
}

impl Default for HdrPolicy {
    fn default() -> Self {
        Self::PreferHdr
    }
}

fn is_hdr_texture_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    )
}

fn preferred_scratch_format(
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

fn storage_format_to_wgsl(format: wgpu::TextureFormat) -> Result<&'static str, &'static str> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => Ok("rgba8unorm"),
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
    Ok(shader_source.replace(SPATIAL_OUTPUT_FORMAT_TOKEN, storage_ty))
}

#[derive(Debug, Clone, Copy)]
enum AtomicStageKind {
    ColorFragment(&'static str),
    SpatialShader(&'static str),
}

#[derive(Debug, Clone, Copy)]
struct AtomicStage {
    kind: AtomicStageKind,
    param_count: usize,
}

#[derive(Debug, Clone)]
enum PlannedPassKind {
    Color { fragments: alloc::string::String },
    Spatial { shader: &'static str },
}

#[derive(Debug, Clone)]
struct PlannedPass {
    kind: PlannedPassKind,
    param_offset: usize,
    param_count: usize,
}

enum CompiledPassKind {
    Color {
        pipeline: wgpu::RenderPipeline,
        bind_group_layout: wgpu::BindGroupLayout,
    },
    Spatial {
        pipeline: wgpu::ComputePipeline,
        bind_group_layout: wgpu::BindGroupLayout,
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
}

struct FinalSpatialOutputPipeline {
    pass_index: usize,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

/// Filter graph introspection for smart pass planning and animation hook installation.
trait FilterGraph: Filter {
    /// Collect atomic stages in source order.
    fn collect_stages(&self, out: &mut Vec<AtomicStage>);

    /// Install animation watchers for all reactive parameters.
    fn bind_animation_watchers(
        &self,
        _param_base: usize,
        _animation_events: Sender<ParamAnimationEvent>,
        _guards: &mut Vec<Box<dyn core::any::Any>>,
    ) {
    }
}

fn push_color_stage(stages: &mut Vec<AtomicStage>, fragment: &'static str, param_count: usize) {
    stages.push(AtomicStage {
        kind: AtomicStageKind::ColorFragment(fragment),
        param_count,
    });
}

fn push_spatial_stage(stages: &mut Vec<AtomicStage>, shader: &'static str, param_count: usize) {
    stages.push(AtomicStage {
        kind: AtomicStageKind::SpatialShader(shader),
        param_count,
    });
}

fn bind_param_watcher<S>(
    signal: &S,
    param_index: usize,
    animation_events: Sender<ParamAnimationEvent>,
    guards: &mut Vec<Box<dyn core::any::Any>>,
) where
    S: Signal<Output = f32> + 'static,
    S::Guard: 'static,
{
    let guard = signal.watch(move |context| {
        let animation = context.metadata().try_get::<Animation>();
        if let Some(animation) = animation {
            let target = context.into_value();
            let _ = animation_events.send(ParamAnimationEvent {
                param_index,
                target_value: target,
                animation,
            });
        }
    });
    guards.push(Box::new(guard));
}

macro_rules! impl_filter_graph_one_param {
    ($ty:ident, color) => {
        impl<S> FilterGraph for filtrate_core::filters::$ty<S>
        where
            S: Signal<Output = f32> + 'static,
            S::Guard: 'static,
        {
            fn collect_stages(&self, out: &mut Vec<AtomicStage>) {
                push_color_stage(out, self.fragments(), 1);
            }

            fn bind_animation_watchers(
                &self,
                param_base: usize,
                animation_events: Sender<ParamAnimationEvent>,
                guards: &mut Vec<Box<dyn core::any::Any>>,
            ) {
                bind_param_watcher(&self.0, param_base, animation_events, guards);
            }
        }
    };
    ($ty:ident, spatial) => {
        impl<S> FilterGraph for filtrate_core::filters::$ty<S>
        where
            S: Signal<Output = f32> + 'static,
            S::Guard: 'static,
        {
            fn collect_stages(&self, out: &mut Vec<AtomicStage>) {
                push_spatial_stage(out, self.fragments(), 1);
            }

            fn bind_animation_watchers(
                &self,
                param_base: usize,
                animation_events: Sender<ParamAnimationEvent>,
                guards: &mut Vec<Box<dyn core::any::Any>>,
            ) {
                bind_param_watcher(&self.0, param_base, animation_events, guards);
            }
        }
    };
}

impl_filter_graph_one_param!(Brightness, color);
impl_filter_graph_one_param!(Contrast, color);
impl_filter_graph_one_param!(Saturation, color);
impl_filter_graph_one_param!(Grayscale, color);
impl_filter_graph_one_param!(HueRotation, color);
impl_filter_graph_one_param!(Sepia, color);
impl_filter_graph_one_param!(Sharpen, spatial);

impl<S> FilterGraph for filtrate_core::filters::Blur<S>
where
    S: Signal<Output = f32> + 'static,
    S::Guard: 'static,
{
    fn collect_stages(&self, out: &mut Vec<AtomicStage>) {
        // Separable blur: horizontal then vertical, both driven by the same radius.
        push_spatial_stage(
            out,
            include_str!("../../../utils/filtrate-core/src/shaders/blur_horizontal.wgsl"),
            1,
        );
        push_spatial_stage(
            out,
            include_str!("../../../utils/filtrate-core/src/shaders/blur_vertical.wgsl"),
            1,
        );
    }

    fn bind_animation_watchers(
        &self,
        param_base: usize,
        animation_events: Sender<ParamAnimationEvent>,
        guards: &mut Vec<Box<dyn core::any::Any>>,
    ) {
        let guard = self.0.watch(move |context| {
            let Some(animation) = context.metadata().try_get::<Animation>() else {
                return;
            };
            let target = context.into_value();
            let _ = animation_events.send(ParamAnimationEvent {
                param_index: param_base,
                target_value: target,
                animation: animation.clone(),
            });
            let _ = animation_events.send(ParamAnimationEvent {
                param_index: param_base + 1,
                target_value: target,
                animation,
            });
        });
        guards.push(Box::new(guard));
    }
}

impl FilterGraph for filtrate_core::filters::Invert {
    fn collect_stages(&self, out: &mut Vec<AtomicStage>) {
        push_color_stage(out, self.fragments(), 0);
    }
}

impl<R, S> FilterGraph for filtrate_core::filters::Vignette<R, S>
where
    R: Signal<Output = f32> + 'static,
    S: Signal<Output = f32> + 'static,
    R::Guard: 'static,
    S::Guard: 'static,
{
    fn collect_stages(&self, out: &mut Vec<AtomicStage>) {
        push_color_stage(out, self.fragments(), 2);
    }

    fn bind_animation_watchers(
        &self,
        param_base: usize,
        animation_events: Sender<ParamAnimationEvent>,
        guards: &mut Vec<Box<dyn core::any::Any>>,
    ) {
        bind_param_watcher(&self.0, param_base, animation_events.clone(), guards);
        bind_param_watcher(&self.1, param_base + 1, animation_events, guards);
    }
}

impl<A, B> FilterGraph for Chain<A, B>
where
    A: Filter + FilterGraph,
    B: Filter + FilterGraph,
{
    fn collect_stages(&self, out: &mut Vec<AtomicStage>) {
        self.first.collect_stages(out);
        self.second.collect_stages(out);
    }

    fn bind_animation_watchers(
        &self,
        param_base: usize,
        animation_events: Sender<ParamAnimationEvent>,
        guards: &mut Vec<Box<dyn core::any::Any>>,
    ) {
        self.first
            .bind_animation_watchers(param_base, animation_events.clone(), guards);
        self.second.bind_animation_watchers(
            param_base + <A::Params as ParamArray>::LEN,
            animation_events,
            guards,
        );
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
                    kind: PlannedPassKind::Spatial { shader },
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
    uniform_data: [f32; FILTER_UNIFORM_WORDS],
) {
    let needs_upload = *last_uniform_data != Some(uniform_data);
    if needs_upload {
        queue.write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&uniform_data[..]));
        *last_uniform_data = Some(uniform_data);
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
    track: AnimationTrack<f32>,
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
    /// Last timestamp used for animation advancement.
    last_tick: Instant,
}

const fn approx_param_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= PARAM_EPSILON
}

#[derive(Debug, Clone)]
struct ParamAnimationEvent {
    param_index: usize,
    target_value: f32,
    animation: Animation,
}

// ============================================================================
// Filter trait adapter - converts Filter to GpuFilter with animation support
// ============================================================================

/// Adapter that wraps a `Filter` to implement `GpuFilter` with animation support.
///
/// This bridges the pure-data `Filter` trait from filtrate-core to the
/// GPU-aware `GpuFilter` trait used by the rendering system.
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
    /// Reused parameter buffer to avoid per-frame heap allocations.
    target_params: Vec<f32>,
    /// Scratch buffer used to snapshot signal values before diffing.
    staged_params: Vec<f32>,
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
    /// Animation events produced by signal watchers.
    animation_events: Receiver<ParamAnimationEvent>,
    /// Watcher guards to keep animation watchers alive.
    _watcher_guards: Vec<Box<dyn core::any::Any>>,
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

impl<F: Filter> core::fmt::Debug for FilterAdapter<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilterAdapter").finish_non_exhaustive()
    }
}

#[allow(private_bounds)]
impl<F: Filter + FilterGraph> FilterAdapter<F> {
    /// Create a new filter adapter.
    #[must_use]
    pub fn new(filter: F) -> Self {
        let param_count = <F::Params as ParamArray>::LEN;
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
            last_tick: Instant::now(),
        };
        let staged_params = target_params.clone();
        let (animation_events_tx, animation_events) = mpsc::channel();

        let mut watcher_guards: Vec<Box<dyn core::any::Any>> = Vec::new();
        filter.bind_animation_watchers(0, animation_events_tx, &mut watcher_guards);

        Self {
            filter,
            target_params,
            staged_params,
            target_params_dirty: true,
            passes: Vec::new(),
            requires_scratch: false,
            hdr_policy: HdrPolicy::default(),
            scratch_format: wgpu::TextureFormat::Rgba8Unorm,
            setup_error: None,
            sampler: None,
            animation_state,
            animation_events,
            _watcher_guards: watcher_guards,
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

    /// Chain another filter onto this adapter.
    ///
    /// Returns a new `FilterAdapter` wrapping a `Chain` of both filters.
    /// Consecutive color-only filters will be fused into a single GPU pass.
    #[must_use]
    pub fn then<F2: Filter + FilterGraph>(self, filter: F2) -> FilterAdapter<Chain<F, F2>> {
        let mut next = FilterAdapter::new(Chain {
            first: self.filter,
            second: filter,
        });
        next.hdr_policy = self.hdr_policy;
        next
    }

    /// Set HDR behavior policy for this filter chain.
    #[must_use]
    pub fn hdr_policy(mut self, policy: HdrPolicy) -> Self {
        self.hdr_policy = policy;
        self
    }

    /// Require HDR intermediates; setup fails if unsupported.
    #[must_use]
    pub fn require_hdr(self) -> Self {
        self.hdr_policy(HdrPolicy::RequireHdr)
    }

    /// Prefer HDR intermediates with automatic LDR fallback.
    #[must_use]
    pub fn prefer_hdr(self) -> Self {
        self.hdr_policy(HdrPolicy::PreferHdr)
    }

    /// Force LDR intermediates for maximum compatibility.
    #[must_use]
    pub fn force_ldr(self) -> Self {
        self.hdr_policy(HdrPolicy::ForceLdr)
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
        loop {
            match self.animation_events.try_recv() {
                Ok(event) => {
                    if event.param_index >= self.animation_state.current_values.len() {
                        continue;
                    }
                    let entry = &mut self.animation_state.tracks[event.param_index];
                    entry
                        .track
                        .set_target(event.target_value, Some(event.animation));
                    entry.animated_target = Some(event.target_value);
                    self.animation_state.has_active_animation = true;
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    /// Update interpolated parameters in-place; returns whether another frame is needed.
    fn update_interpolated_params(&mut self) -> bool {
        let param_count = self.target_params.len();
        if param_count > MAX_FILTER_PARAMS {
            return false;
        }
        self.consume_animation_events();
        let now = Instant::now();
        let delta = now.saturating_duration_since(self.animation_state.last_tick);
        self.animation_state.last_tick = now;

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

    fn has_setup_error(&self) -> bool {
        self.setup_error.is_some()
    }

    #[cfg(test)]
    fn last_render_used_direct_output(&self) -> bool {
        self.last_render_used_direct_output
    }

    #[cfg(test)]
    fn allocated_scratch_slots(&self) -> [bool; 2] {
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

        for slot in 0..2 {
            if !required_slots[slot] {
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
            }
            self.blit_bind_group = None;
        }

        self.scratch_size = if required_slots.iter().any(|required| *required) {
            (width, height)
        } else {
            (0, 0)
        };
    }

    fn take_validation_error(device: &wgpu::Device) -> Option<wgpu::Error> {
        crate::pop_error_scope_now(device, "filter_view::take_validation_error")
    }

    fn build_compiled_passes(
        &mut self,
        ctx: &FilterContext,
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
            if Self::take_validation_error(ctx.device).is_some() {
                return Err("selected scratch texture format is unsupported on this device");
            }
        }

        for (pass, binding_plan) in planned.iter().zip(binding_plans.into_iter()) {
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
                        self.create_color_pipeline(ctx, fragments, target_format);
                    if Self::take_validation_error(ctx.device).is_some() {
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
                    });
                }
                PlannedPassKind::Spatial { shader } => {
                    if !matches!(binding_plan, PassBindingPlan::Spatial { .. }) {
                        return Err("runtime planner produced invalid spatial binding plan");
                    }
                    ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
                    let (pipeline, bind_group_layout) =
                        self.create_spatial_pipeline(ctx, shader, scratch_format)?;
                    if let Some(err) = Self::take_validation_error(ctx.device) {
                        tracing::error!("[Filter] spatial pipeline validation error: {err:?}");
                        return Err(
                            "failed to create spatial pipeline for selected storage format",
                        );
                    }
                    self.passes.push(CompiledPass {
                        kind: CompiledPassKind::Spatial {
                            pipeline,
                            bind_group_layout,
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
                    PlannedPassKind::Spatial { shader } if idx + 1 == planned.len() => {
                        Some((idx, shader))
                    }
                    _ => None,
                })
        {
            if storage_format_to_wgsl(ctx.output_format).is_ok() {
                ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
                match self.create_spatial_pipeline(ctx, shader, ctx.output_format) {
                    Ok((pipeline, bind_group_layout)) => {
                        if Self::take_validation_error(ctx.device).is_none() {
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
        }

        if self.blit_source_scratch_slot.is_some() {
            ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let (blit_pipeline, blit_bind_group_layout) = self.create_blit_pipeline(ctx);
            if Self::take_validation_error(ctx.device).is_some() {
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
}

impl<F: Filter + FilterGraph> GpuFilter for FilterAdapter<F> {
    fn setup(&mut self, ctx: &FilterContext) -> impl Future<Output = ()> {
        let param_count = <F::Params as ParamArray>::LEN;
        if param_count > MAX_FILTER_PARAMS {
            self.set_setup_error("filter chain exceeds 64 params (uniform limit)");
            return core::future::ready(());
        }

        // Create shared sampler
        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("filter sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        self.sampler = Some(sampler);

        let mut stages = Vec::new();
        self.filter.collect_stages(&mut stages);
        let planned = match fuse_stages(&stages) {
            Ok(passes) => passes,
            Err(err) => {
                self.set_setup_error(err);
                return core::future::ready(());
            }
        };

        self.requires_scratch = planned.len() > 1
            || matches!(
                planned.last().map(|p| &p.kind),
                Some(PlannedPassKind::Spatial { .. })
            );

        let scratch_candidates = if self.requires_scratch {
            match self.hdr_policy {
                HdrPolicy::ForceLdr => alloc::vec![wgpu::TextureFormat::Rgba8Unorm],
                HdrPolicy::RequireHdr => alloc::vec![wgpu::TextureFormat::Rgba16Float],
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

        let mut build_ok = false;
        let mut last_err: Option<&'static str> = None;
        for (idx, candidate) in scratch_candidates.iter().enumerate() {
            match self.build_compiled_passes(ctx, &planned, *candidate) {
                Ok(()) => {
                    self.scratch_format = *candidate;
                    build_ok = true;
                    if idx > 0 && self.hdr_policy == HdrPolicy::PreferHdr {
                        tracing::warn!(
                            "[Filter] preferred scratch format unavailable, falling back to {:?}",
                            candidate
                        );
                    }
                    break;
                }
                Err(err) => {
                    last_err = Some(err);
                    self.passes.clear();
                    self.blit_pipeline = None;
                    self.blit_bind_group_layout = None;
                    self.blit_bind_group = None;
                    self.blit_source_scratch_slot = None;
                    self.final_spatial_output = None;
                }
            }
        }

        if !build_ok {
            self.set_setup_error(match (self.hdr_policy, last_err) {
                (HdrPolicy::RequireHdr, Some(_)) => {
                    "HDR is required by policy but unavailable for this filter pipeline"
                }
                (_, Some(err)) => err,
                (_, None) => "filter setup produced no executable passes",
            });
            return core::future::ready(());
        }

        // Initialize current values from the latest snapped targets.
        self.apply_target_params_to_current_values();

        core::future::ready(())
    }

    fn render(&mut self, input: &FilterInput, output: &FilterOutput) -> bool {
        #[cfg(test)]
        {
            self.last_render_used_direct_output = false;
        }
        if self.has_setup_error() || self.passes.is_empty() {
            return false;
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
                    return false;
                }
            }
        }

        let needs_redraw = self.update_interpolated_params();
        let current_values = &self.animation_state.current_values;
        if current_values.is_empty() && <F::Params as ParamArray>::LEN > 0 {
            return false;
        }

        let Some(sampler) = &self.sampler else {
            return false;
        };

        let mut encoder = input
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("filter pass encoder"),
            });
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
                                return false;
                            };
                            view
                        }
                    };
                    let target_view: &wgpu::TextureView = match target {
                        ColorTarget::Output => &output.view,
                        ColorTarget::Scratch(slot) => {
                            let Some(view) = self.scratch_views[slot].as_ref() else {
                                return false;
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
                        uniform_data,
                    );

                    let transient_bind_group;
                    let bind_group = if matches!(source, PassTextureSource::Input) {
                        transient_bind_group =
                            input.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("filter color dynamic bind group"),
                                layout: bind_group_layout,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::TextureView(source_view),
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
                        &transient_bind_group
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
                            return false;
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
                                return false;
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
                                return false;
                            };
                            (target_view, pipeline, bind_group_layout)
                        }
                    } else {
                        let Some(target_view) = self.scratch_views[target_scratch].as_ref() else {
                            return false;
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
                        uniform_data,
                    );

                    let transient_bind_group;
                    let bind_group = if writes_output_directly
                        || matches!(source, PassTextureSource::Input)
                    {
                        transient_bind_group =
                            input.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("filter spatial dynamic bind group"),
                                layout: dispatch_bind_group_layout,
                                entries: &[
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
                                ],
                            });
                        &transient_bind_group
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
                            return false;
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
                        let workgroups_x = target_width.div_ceil(8);
                        let workgroups_y = target_height.div_ceil(8);
                        compute_pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
                    }

                    if writes_output_directly {
                        used_direct_spatial_output = true;
                    }

                    source_width = target_width;
                    source_height = target_height;
                }
                _ => return false,
            }
        }

        if !used_direct_spatial_output {
            if let Some(blit_source_slot) = self.blit_source_scratch_slot {
                let Some(blit_pipeline) = &self.blit_pipeline else {
                    return false;
                };
                let Some(blit_bind_group_layout) = &self.blit_bind_group_layout else {
                    return false;
                };
                let Some(blit_source_view) = self.scratch_views[blit_source_slot].as_ref() else {
                    return false;
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
                    return false;
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
        }

        input.queue.submit([encoder.finish()]);
        self.target_params_dirty = false;
        #[cfg(test)]
        {
            self.last_render_used_direct_output = used_direct_spatial_output;
        }
        needs_redraw
    }

    fn sync_targets(&mut self) {
        self.filter.params().write_to(&mut self.staged_params);
        if self.staged_params != self.target_params {
            self.target_params.copy_from_slice(&self.staged_params);
            self.target_params_dirty = true;
        }
    }

    fn redraw_hint(&self) -> bool {
        self.target_params_dirty || self.animation_state.has_active_animation
    }
}
#[allow(private_bounds)]
impl<F: Filter + FilterGraph> FilterAdapter<F> {
    fn create_color_pipeline(
        &self,
        ctx: &FilterContext,
        fragments: &str,
        target_format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
        let preamble =
            include_str!("../../../utils/filtrate-core/src/shaders/fragment_preamble.wgsl");
        let postamble =
            include_str!("../../../utils/filtrate-core/src/shaders/fragment_postamble.wgsl");

        let mut shader_source = alloc::string::String::from(preamble);
        shader_source.push_str(fragments);
        shader_source.push_str(postamble);

        let shader = crate::shared_context::create_cached_shader_module(
            ctx.device,
            "filter color shader",
            &shader_source,
        );

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
        &self,
        ctx: &FilterContext,
        shader_source: &str,
        storage_format: wgpu::TextureFormat,
    ) -> Result<(wgpu::ComputePipeline, wgpu::BindGroupLayout), &'static str> {
        let shader_source = specialize_spatial_shader(shader_source, storage_format)?;
        let shader = crate::shared_context::create_cached_shader_module(
            ctx.device,
            "filter spatial shader",
            &shader_source,
        );

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("filter spatial bind group layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::StorageTexture {
                                access: wgpu::StorageTextureAccess::WriteOnly,
                                format: storage_format,
                                view_dimension: wgpu::TextureViewDimension::D2,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::COMPUTE,
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

    fn create_blit_pipeline(
        &self,
        ctx: &FilterContext,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
        let shader = crate::shared_context::create_cached_shader_module(
            ctx.device,
            "filter blit shader",
            include_str!("shaders/blit.wgsl"),
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
    data[0] = width as f32;
    data[1] = height as f32;
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
    data[0] = output_width as f32;
    data[1] = output_height as f32;
    data[2] = input_width as f32;
    data[3] = input_height as f32;
    for (idx, value) in params.iter().enumerate().take(MAX_FILTER_PARAMS) {
        data[4 + idx] = *value;
    }
    data
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
    }

    fn create_test_device() -> Option<TestGpu> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..Default::default()
        });
        let adapter =
            crate::pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            }))
            .ok()?;
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
            crate::pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .ok()?;
        Some(TestGpu {
            device,
            queue,
            adapter_info,
            rgba8_storage,
            rgba16_storage,
        })
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
        let copy_size = (padded_bpr * height) as u64;

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
        let copy_size = (padded_bpr * height) as u64;

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

    fn create_test_input_rgba(width: u32, height: u32) -> Vec<u8> {
        let mut data = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width + x) * 4) as usize;
                let xf = x as f32 / (width.saturating_sub(1)).max(1) as f32;
                let yf = y as f32 / (height.saturating_sub(1)).max(1) as f32;
                let checker = if ((x / 16) + (y / 16)) % 2 == 0 {
                    32.0
                } else {
                    -32.0
                };
                let ring = (((x as i32 - width as i32 / 2).pow(2)
                    + (y as i32 - height as i32 / 2).pow(2)) as f32)
                    .sqrt();
                let edge = if ring > (width.min(height) as f32 * 0.28)
                    && ring < (width.min(height) as f32 * 0.32)
                {
                    80.0
                } else {
                    0.0
                };

                let r = (xf * 255.0 + checker + edge).clamp(0.0, 255.0) as u8;
                let g = (yf * 255.0 - checker + edge).clamp(0.0, 255.0) as u8;
                let b = (((1.0 - xf) * (1.0 - yf) * 255.0) + edge).clamp(0.0, 255.0) as u8;

                data[idx] = r;
                data[idx + 1] = g;
                data[idx + 2] = b;
                data[idx + 3] = 255;
            }
        }
        data
    }

    fn run_filter_and_readback<G: GpuFilter>(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        input_texture: &wgpu::Texture,
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        mut filter: G,
    ) -> Option<Vec<u8>> {
        let format = wgpu::TextureFormat::Rgba8Unorm;
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

        let ctx = FilterContext {
            device,
            queue,
            input_format: format,
            output_format: format,
            pipeline_cache: None,
        };
        crate::pollster::block_on(filter.setup(&ctx));

        let input = FilterInput {
            device,
            queue,
            texture: input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width: input_width,
            height: input_height,
        };
        let output = FilterOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width: output_width,
            height: output_height,
        };

        let _ = filter.render(&input, &output);
        Some(readback_rgba8_image(
            device,
            queue,
            &output_texture,
            output_width,
            output_height,
        ))
    }

    #[test]
    fn reject_empty_stage_graph() {
        let err = fuse_stages(&[]).expect_err("empty stage graph should fail");
        assert_eq!(err, "filter graph produced no stages");
    }

    #[test]
    fn fuse_adjacent_color_stages() {
        let filter = Chain {
            first: filtrate_core::filters::Brightness(0.2f32),
            second: Chain {
                first: filtrate_core::filters::Contrast(1.1f32),
                second: filtrate_core::filters::Invert,
            },
        };

        let mut stages = Vec::new();
        filter.collect_stages(&mut stages);
        let passes = fuse_stages(&stages).expect("fuse should succeed");

        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].param_offset, 0);
        assert_eq!(passes[0].param_count, 2);
        assert!(matches!(passes[0].kind, PlannedPassKind::Color { .. }));
    }

    #[test]
    fn keep_spatial_boundaries() {
        let filter = Chain {
            first: filtrate_core::filters::Blur(2.0f32),
            second: Chain {
                first: filtrate_core::filters::Brightness(0.1f32),
                second: filtrate_core::filters::Sharpen(0.8f32),
            },
        };

        let mut stages = Vec::new();
        filter.collect_stages(&mut stages);
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
            first: filtrate_core::filters::Brightness(0.2f32),
            second: Chain {
                first: filtrate_core::filters::Contrast(1.1f32),
                second: Chain {
                    first: filtrate_core::filters::Blur(2.0f32),
                    second: filtrate_core::filters::Sepia(0.7f32),
                },
            },
        };

        let mut stages = Vec::new();
        filter.collect_stages(&mut stages);
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
            first: filtrate_core::filters::Blur(2.0f32),
            second: Chain {
                first: filtrate_core::filters::Brightness(0.2f32),
                second: filtrate_core::filters::Sharpen(0.8f32),
            },
        };

        let mut stages = Vec::new();
        filter.collect_stages(&mut stages);
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
            first: filtrate_core::filters::Brightness(0.1f32),
            second: Chain {
                first: filtrate_core::filters::Contrast(1.2f32),
                second: filtrate_core::filters::Invert,
            },
        };

        let mut stages = Vec::new();
        filter.collect_stages(&mut stages);
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
            include_str!("../../../utils/filtrate-core/src/shaders/fragments/brightness.wgsl")
        }
    }

    impl FilterGraph for HugeFilter {
        fn collect_stages(&self, out: &mut Vec<AtomicStage>) {
            push_color_stage(
                out,
                include_str!("../../../utils/filtrate-core/src/shaders/fragments/brightness.wgsl"),
                <Self::Params as ParamArray>::LEN,
            );
        }
    }

    #[test]
    fn fast_fail_when_param_count_exceeds_uniform_limit() {
        assert!(<HugeParams as ParamArray>::LEN > MAX_FILTER_PARAMS);

        let mut adapter = FilterAdapter::new(HugeFilter);
        let needs_redraw = adapter.update_interpolated_params();
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
    fn spatial_uniform_data_uses_vec4_packed_layout() {
        let data = build_spatial_uniform_data(320, 240, 640, 480, &[2.0, 3.0]);
        assert_eq!(data.len(), 4 + MAX_FILTER_PARAMS);
        assert_eq!(data[0], 320.0);
        assert_eq!(data[1], 240.0);
        assert_eq!(data[2], 640.0);
        assert_eq!(data[3], 480.0);
        assert_eq!(data[4], 2.0);
        assert_eq!(data[5], 3.0);
    }

    #[test]
    fn color_uniform_data_uses_fixed_array_layout() {
        let data = build_color_uniform_data(800, 600, &[1.0, 2.0]);
        assert_eq!(data.len(), 4 + MAX_FILTER_PARAMS);
        assert_eq!(data[0], 800.0);
        assert_eq!(data[1], 600.0);
        assert_eq!(data[4], 1.0);
        assert_eq!(data[5], 2.0);
    }

    #[test]
    fn hdr_color_fragments_do_not_clamp_to_unit_range() {
        let brightness =
            include_str!("../../../utils/filtrate-core/src/shaders/fragments/brightness.wgsl");
        let contrast =
            include_str!("../../../utils/filtrate-core/src/shaders/fragments/contrast.wgsl");
        let sharpen = include_str!("../../../utils/filtrate-core/src/shaders/sharpen.wgsl");

        assert!(!brightness.contains("clamp("));
        assert!(!contrast.contains("clamp("));
        assert!(!sharpen.contains("clamp(result.rgb"));
    }

    #[test]
    fn spatial_shaders_use_dynamic_storage_format_and_texture_load() {
        let blur = include_str!("../../../utils/filtrate-core/src/shaders/blur.wgsl");
        let sharpen = include_str!("../../../utils/filtrate-core/src/shaders/sharpen.wgsl");

        assert!(blur.contains(SPATIAL_OUTPUT_FORMAT_TOKEN));
        assert!(sharpen.contains(SPATIAL_OUTPUT_FORMAT_TOKEN));
        assert!(blur.contains("textureLoad("));
        assert!(sharpen.contains("textureLoad("));
        assert!(!blur.contains("input_sampler"));
        assert!(!sharpen.contains("input_sampler"));
    }

    #[test]
    fn hdr_policy_builders_update_adapter_policy() {
        let adapter = FilterAdapter::new(filtrate_core::filters::Blur(2.0f32));
        assert_eq!(adapter.hdr_policy, HdrPolicy::PreferHdr);

        let adapter = adapter.require_hdr();
        assert_eq!(adapter.hdr_policy, HdrPolicy::RequireHdr);

        let adapter = adapter.force_ldr();
        assert_eq!(adapter.hdr_policy, HdrPolicy::ForceLdr);
    }

    #[test]
    fn then_preserves_hdr_policy() {
        let adapter = FilterAdapter::new(filtrate_core::filters::Blur(2.0f32)).require_hdr();
        let chained = adapter.then(filtrate_core::filters::Sharpen(1.0f32));
        assert_eq!(chained.hdr_policy, HdrPolicy::RequireHdr);
    }

    #[test]
    fn gpu_color_filter_executes_and_writes_output() {
        let Some(gpu) = create_test_device() else {
            eprintln!("Skipping GPU test: no compatible adapter/device");
            return;
        };
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

        let mut adapter = FilterAdapter::new(filtrate_core::filters::Brightness(0.25f32));
        let ctx = FilterContext {
            device,
            queue,
            input_format: format,
            output_format: format,
            pipeline_cache: None,
        };
        crate::pollster::block_on(GpuFilter::setup(&mut adapter, &ctx));

        let input = FilterInput {
            device: &device,
            queue: &queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
        };
        let output = FilterOutput {
            device: &device,
            queue: &queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
        };

        let needs_redraw = GpuFilter::render(&mut adapter, &input, &output);
        assert!(!needs_redraw);

        let pixel = readback_rgba8_pixel(device, queue, &output_texture, width, height);
        assert!(pixel[0] > 0 || pixel[1] > 0 || pixel[2] > 0);
    }

    #[test]
    fn gpu_spatial_filter_supports_mismatched_input_output_sizes() {
        let Some(gpu) = create_test_device() else {
            eprintln!("Skipping GPU test: no compatible adapter/device");
            return;
        };
        let device = &gpu.device;
        let queue = &gpu.queue;
        let adapter_info = &gpu.adapter_info;

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

        let mut adapter = FilterAdapter::new(filtrate_core::filters::Blur(1.0f32));
        let ctx = FilterContext {
            device,
            queue,
            input_format: format,
            output_format: format,
            pipeline_cache: None,
        };
        crate::pollster::block_on(GpuFilter::setup(&mut adapter, &ctx));
        if adapter.has_setup_error() {
            eprintln!(
                "Skipping spatial GPU test on adapter {} ({:?}): unsupported capability ({:?})",
                adapter_info.name, adapter_info.backend, adapter.setup_error,
            );
            eprintln!(
                "Adapter storage support: rgba8={}, rgba16f={}",
                gpu.rgba8_storage, gpu.rgba16_storage
            );
            return;
        }
        assert!(
            !adapter.passes.is_empty(),
            "spatial passes should be compiled"
        );

        let input = FilterInput {
            device: &device,
            queue: &queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width: in_width,
            height: in_height,
        };
        let output = FilterOutput {
            device: &device,
            queue: &queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width: out_width,
            height: out_height,
        };

        let needs_redraw = GpuFilter::render(&mut adapter, &input, &output);
        assert!(!needs_redraw);

        let pixel = readback_rgba8_pixel(device, queue, &output_texture, out_width, out_height);
        assert!(
            pixel.iter().any(|&c| c > 0),
            "spatial output should not be all zeros, got {pixel:?}"
        );
    }

    #[test]
    fn gpu_spatial_filter_uses_direct_output_when_storage_binding_is_available() {
        let Some(gpu) = create_test_device() else {
            eprintln!("Skipping GPU test: no compatible adapter/device");
            return;
        };
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

        let mut adapter = FilterAdapter::new(filtrate_core::filters::Blur(1.0f32));
        let ctx = FilterContext {
            device,
            queue,
            input_format: format,
            output_format: format,
            pipeline_cache: None,
        };
        crate::pollster::block_on(GpuFilter::setup(&mut adapter, &ctx));
        if adapter.has_setup_error() {
            eprintln!(
                "Skipping GPU test: setup failed ({:?})",
                adapter.setup_error
            );
            return;
        }
        let expected_direct_output = adapter.final_spatial_output.is_some();

        let input = FilterInput {
            device: &device,
            queue: &queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
        };
        let output = FilterOutput {
            device: &device,
            queue: &queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
        };

        let _ = GpuFilter::render(&mut adapter, &input, &output);
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
        let Some(gpu) = create_test_device() else {
            eprintln!("Skipping GPU test: no compatible adapter/device");
            return;
        };
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

        let mut adapter = FilterAdapter::new(filtrate_core::filters::Blur(1.0f32));
        let ctx = FilterContext {
            device,
            queue,
            input_format: format,
            output_format: format,
            pipeline_cache: None,
        };
        crate::pollster::block_on(GpuFilter::setup(&mut adapter, &ctx));
        if adapter.has_setup_error() {
            eprintln!(
                "Skipping GPU test: setup failed ({:?})",
                adapter.setup_error
            );
            return;
        }

        let input = FilterInput {
            device: &device,
            queue: &queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
        };
        let output = FilterOutput {
            device: &device,
            queue: &queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format,
            width,
            height,
        };

        let _ = GpuFilter::render(&mut adapter, &input, &output);
        assert!(!adapter.last_render_used_direct_output());
        assert_eq!(
            adapter.allocated_scratch_slots(),
            [true, true],
            "non-storage output must keep both blur scratch slots for fallback blit"
        );
    }

    #[test]
    fn gpu_export_filter_gallery_images() {
        let Some(gpu) = create_test_device() else {
            eprintln!("Skipping GPU gallery test: no compatible adapter/device");
            return;
        };
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
                    &input_texture,
                    input_width,
                    input_height,
                    $ow,
                    $oh,
                    $filter,
                )
                .expect("filter execution should succeed");
                write_png(&output_dir.join($name), $ow, $oh, &result);
            }};
        }

        export_filter!(
            "brightness.png",
            input_width,
            input_height,
            FilterAdapter::new(filtrate_core::filters::Brightness(0.2f32))
        );
        export_filter!(
            "contrast.png",
            input_width,
            input_height,
            FilterAdapter::new(filtrate_core::filters::Contrast(1.4f32))
        );
        export_filter!(
            "saturation.png",
            input_width,
            input_height,
            FilterAdapter::new(filtrate_core::filters::Saturation(1.8f32))
        );
        export_filter!(
            "grayscale.png",
            input_width,
            input_height,
            FilterAdapter::new(filtrate_core::filters::Grayscale(1.0f32))
        );
        export_filter!(
            "hue_rotation.png",
            input_width,
            input_height,
            FilterAdapter::new(filtrate_core::filters::HueRotation(120.0f32))
        );
        export_filter!(
            "sepia.png",
            input_width,
            input_height,
            FilterAdapter::new(filtrate_core::filters::Sepia(1.0f32))
        );
        export_filter!(
            "invert.png",
            input_width,
            input_height,
            FilterAdapter::new(filtrate_core::filters::Invert)
        );
        export_filter!(
            "blur.png",
            input_width,
            input_height,
            FilterAdapter::new(filtrate_core::filters::Blur(3.0f32))
        );
        export_filter!(
            "sharpen.png",
            input_width,
            input_height,
            FilterAdapter::new(filtrate_core::filters::Sharpen(1.5f32))
        );
        export_filter!(
            "chain_blur_brightness.png",
            input_width,
            input_height,
            FilterAdapter::new(filtrate_core::filters::Blur(2.0f32))
                .then(filtrate_core::filters::Brightness(0.15f32))
                .then(filtrate_core::filters::Contrast(1.2f32))
        );
        export_filter!(
            "blur_resized_384x216.png",
            384,
            216,
            FilterAdapter::new(filtrate_core::filters::Blur(2.0f32))
        );

        eprintln!("Filter gallery exported to {}", output_dir.display());
    }
}

/// Concrete filter aliases with stable type identities.
///
/// These aliases intentionally normalize reactive parameters to `Computed<f32>`
/// so backend hook nodes remain concrete (`FilteredView<Blur>`, etc.).
pub type Blur = FilterAdapter<filtrate_core::filters::Blur<Computed<f32>>>;
pub type Brightness = FilterAdapter<filtrate_core::filters::Brightness<Computed<f32>>>;
pub type Contrast = FilterAdapter<filtrate_core::filters::Contrast<Computed<f32>>>;
pub type Saturation = FilterAdapter<filtrate_core::filters::Saturation<Computed<f32>>>;
pub type Grayscale = FilterAdapter<filtrate_core::filters::Grayscale<Computed<f32>>>;
pub type HueRotation = FilterAdapter<filtrate_core::filters::HueRotation<Computed<f32>>>;
pub type Invert = FilterAdapter<filtrate_core::filters::Invert>;
pub type Sepia = FilterAdapter<filtrate_core::filters::Sepia<Computed<f32>>>;
pub type Sharpen = FilterAdapter<filtrate_core::filters::Sharpen<Computed<f32>>>;
pub type Vignette = FilterAdapter<filtrate_core::filters::Vignette<Computed<f32>, Computed<f32>>>;

impl Blur {
    /// Returns the reactive blur radius signal driving this filter.
    #[must_use]
    pub fn radius_signal(&self) -> &Computed<f32> {
        &self.filter.0
    }
}

/// Rebuilds the canonical blur filter adapter from a reactive radius signal.
#[must_use]
pub fn blur_from_radius_signal(radius: Computed<f32>) -> Blur {
    FilterAdapter::new(filtrate_core::filters::Blur(radius))
}

/// Extension methods for applying filters to views.
pub trait FilterViewExt: View + Sized {
    /// Apply a `GpuFilter` to this view.
    ///
    /// For the high-level `Filter` API with automatic optimization,
    /// use convenience methods like `.blur()`, `.brightness()`, etc.
    fn filter<F: GpuFilter>(self, filter: F) -> Filtered<Self, F> {
        Filtered::new(self, filter)
    }

    // ========================================================================
    // Convenience methods - return FilterAdapter which supports .then()
    // ========================================================================

    /// Apply a blur filter.
    ///
    /// Accepts reactive values that will be automatically animated.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Static value
    /// my_view.blur(10.0)
    ///
    /// // Reactive value with animation
    /// let radius = binding(10.0);
    /// my_view.blur(radius.animated())
    /// ```
    fn blur<T: IntoSignalF32>(self, radius: T) -> Filtered<Self, Blur> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Blur(
                radius.into_signal_f32().computed(),
            )),
        )
    }

    /// Apply a brightness filter.
    fn brightness<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Brightness> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Brightness(
                amount.into_signal_f32().computed(),
            )),
        )
    }

    /// Apply a contrast filter.
    fn contrast<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Contrast> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Contrast(
                amount.into_signal_f32().computed(),
            )),
        )
    }

    /// Apply a saturation filter.
    fn saturation<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Saturation> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Saturation(
                amount.into_signal_f32().computed(),
            )),
        )
    }

    /// Apply a grayscale filter.
    fn grayscale<T: IntoSignalF32>(self, intensity: T) -> Filtered<Self, Grayscale> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Grayscale(
                intensity.into_signal_f32().computed(),
            )),
        )
    }

    /// Apply a hue rotation filter.
    fn hue_rotation<T: IntoSignalF32>(self, angle: T) -> Filtered<Self, HueRotation> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate_core::filters::HueRotation(
                angle.into_signal_f32().computed(),
            )),
        )
    }

    /// Apply an invert filter.
    fn invert(self) -> Filtered<Self, Invert> {
        Filtered::new(self, FilterAdapter::new(filtrate_core::filters::Invert))
    }

    /// Apply a sepia filter.
    fn sepia<T: IntoSignalF32>(self, intensity: T) -> Filtered<Self, Sepia> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Sepia(
                intensity.into_signal_f32().computed(),
            )),
        )
    }

    /// Apply a sharpen filter.
    fn sharpen<T: IntoSignalF32>(self, amount: T) -> Filtered<Self, Sharpen> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Sharpen(
                amount.into_signal_f32().computed(),
            )),
        )
    }

    /// Apply a vignette filter.
    fn vignette<R: IntoSignalF32, S: IntoSignalF32>(
        self,
        radius: R,
        softness: S,
    ) -> Filtered<Self, Vignette> {
        Filtered::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Vignette(
                radius.into_signal_f32().computed(),
                softness.into_signal_f32().computed(),
            )),
        )
    }
}

impl<V: View> FilterViewExt for V {}
