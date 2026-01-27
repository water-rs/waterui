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
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::future::Future;
use core::time::Duration;

use filtrate_core::{Chain, Filter, FragmentList, ParamArray};
use nami::Signal;
use nami::signal::IntoSignal;
use waterui_core::animation::Animation;
use waterui_core::easing::EasingCurve;
use waterui_core::metadata::MetadataKey;
use waterui_core::{Environment, IntoSignalF32, Metadata, View};

use crate::gpu_surface::SetupFuture;

/// Convert a wgpu texture format to its WGSL storage texture format string.
///
/// This is used to dynamically generate shaders that support different
/// texture formats, including HDR formats like Rgba16Float.
fn texture_format_to_wgsl(format: wgpu::TextureFormat) -> &'static str {
    match format {
        // 8-bit formats
        wgpu::TextureFormat::R8Unorm => "r8unorm",
        wgpu::TextureFormat::R8Snorm => "r8snorm",
        wgpu::TextureFormat::R8Uint => "r8uint",
        wgpu::TextureFormat::R8Sint => "r8sint",
        wgpu::TextureFormat::Rg8Unorm => "rg8unorm",
        wgpu::TextureFormat::Rg8Snorm => "rg8snorm",
        wgpu::TextureFormat::Rg8Uint => "rg8uint",
        wgpu::TextureFormat::Rg8Sint => "rg8sint",
        wgpu::TextureFormat::Rgba8Unorm => "rgba8unorm",
        wgpu::TextureFormat::Rgba8UnormSrgb => "rgba8unorm", // WGSL doesn't have srgb storage
        wgpu::TextureFormat::Rgba8Snorm => "rgba8snorm",
        wgpu::TextureFormat::Rgba8Uint => "rgba8uint",
        wgpu::TextureFormat::Rgba8Sint => "rgba8sint",
        wgpu::TextureFormat::Bgra8Unorm => "bgra8unorm",
        wgpu::TextureFormat::Bgra8UnormSrgb => "bgra8unorm", // WGSL doesn't have srgb storage
        // 16-bit formats
        wgpu::TextureFormat::R16Uint => "r16uint",
        wgpu::TextureFormat::R16Sint => "r16sint",
        wgpu::TextureFormat::R16Float => "r16float",
        wgpu::TextureFormat::Rg16Uint => "rg16uint",
        wgpu::TextureFormat::Rg16Sint => "rg16sint",
        wgpu::TextureFormat::Rg16Float => "rg16float",
        wgpu::TextureFormat::Rgba16Uint => "rgba16uint",
        wgpu::TextureFormat::Rgba16Sint => "rgba16sint",
        wgpu::TextureFormat::Rgba16Float => "rgba16float",
        // 32-bit formats
        wgpu::TextureFormat::R32Uint => "r32uint",
        wgpu::TextureFormat::R32Sint => "r32sint",
        wgpu::TextureFormat::R32Float => "r32float",
        wgpu::TextureFormat::Rg32Uint => "rg32uint",
        wgpu::TextureFormat::Rg32Sint => "rg32sint",
        wgpu::TextureFormat::Rg32Float => "rg32float",
        wgpu::TextureFormat::Rgba32Uint => "rgba32uint",
        wgpu::TextureFormat::Rgba32Sint => "rgba32sint",
        wgpu::TextureFormat::Rgba32Float => "rgba32float",
        // Default fallback for unsupported formats
        _ => "rgba8unorm",
    }
}

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
}

/// Object-safe trait for type-erased GPU filters.
pub(crate) trait GpuFilterImpl: 'static {
    fn setup<'a>(&'a mut self, ctx: &'a FilterContext<'a>) -> SetupFuture<'a>;
    fn render(&mut self, input: &FilterInput, output: &FilterOutput) -> bool;
}

impl<T: GpuFilter> GpuFilterImpl for T {
    fn setup<'a>(&'a mut self, ctx: &'a FilterContext<'a>) -> SetupFuture<'a> {
        Box::pin(GpuFilter::setup(self, ctx))
    }

    fn render(&mut self, input: &FilterInput, output: &FilterOutput) -> bool {
        GpuFilter::render(self, input, output)
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
}

/// A view with an applied GPU filter.
///
/// `FilteredView` wraps a view and a `GpuFilter`, converting to `Metadata<AppliedFilter>`
/// at the View boundary. For the simpler `Filter` trait, use `FilteredViewWithFilter`.
///
/// # Layout
///
/// `FilteredView` is transparent to layout - the child view's size determines
/// the overall size. The filter does not affect layout calculations.
pub struct FilteredView<V: View, F: GpuFilter> {
    content: V,
    filter: F,
}

impl<V: View, F: GpuFilter> core::fmt::Debug for FilteredView<V, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilteredView").finish_non_exhaustive()
    }
}

impl<V: View, F: GpuFilter> FilteredView<V, F> {
    /// Create a new filtered view with a `GpuFilter`.
    #[must_use]
    pub fn new(content: V, filter: F) -> Self {
        Self { content, filter }
    }
}

impl<V: View, F: Filter> FilteredView<V, FilterAdapter<F>> {
    /// Chain another filter onto this view.
    ///
    /// Returns a new `FilteredView` with the filters chained together.
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
    pub fn then<F2: Filter>(self, filter: F2) -> FilteredView<V, FilterAdapter<Chain<F, F2>>> {
        FilteredView::new(self.content, self.filter.then(filter))
    }
}

impl<V: View, F: GpuFilter> View for FilteredView<V, F> {
    fn body(self, _env: &Environment) -> impl View {
        Metadata::new(self.content, AppliedFilter::new(self.filter))
    }
}

// ============================================================================
// Animation State Tracking
// ============================================================================

/// Tracks the state of an in-progress animation for a single parameter.
#[derive(Debug, Clone)]
struct ParamAnimation {
    /// The starting value when animation began.
    start_value: f32,
    /// The target value to animate towards.
    target_value: f32,
    /// When the animation started (as duration since program start).
    start_time: Duration,
    /// The animation configuration.
    animation: Animation,
    /// Current interpolated value.
    current_value: f32,
    /// For spring animations: current velocity.
    velocity: f32,
}

impl ParamAnimation {
    /// Create a new animation state.
    fn new(start_value: f32, target_value: f32, animation: Animation) -> Self {
        Self {
            start_value,
            target_value,
            start_time: current_time(),
            animation,
            current_value: start_value,
            velocity: 0.0,
        }
    }

    /// Update the animation and return (current_value, is_complete).
    fn update(&mut self) -> (f32, bool) {
        let elapsed = current_time().saturating_sub(self.start_time);

        match &self.animation {
            Animation::Default => {
                // Default uses ease-in-out with 250ms
                let duration = Duration::from_millis(250);
                let (value, complete) = self.interpolate_easing(elapsed, duration, ease_in_out);
                self.current_value = value;
                (value, complete)
            }
            Animation::Linear(duration) => {
                let (value, complete) = self.interpolate_easing(elapsed, *duration, linear);
                self.current_value = value;
                (value, complete)
            }
            Animation::EaseIn(duration) => {
                let (value, complete) = self.interpolate_easing(elapsed, *duration, ease_in);
                self.current_value = value;
                (value, complete)
            }
            Animation::EaseOut(duration) => {
                let (value, complete) = self.interpolate_easing(elapsed, *duration, ease_out);
                self.current_value = value;
                (value, complete)
            }
            Animation::EaseInOut(duration) => {
                let (value, complete) = self.interpolate_easing(elapsed, *duration, ease_in_out);
                self.current_value = value;
                (value, complete)
            }
            Animation::CubicBezier { duration, .. } => {
                // Use the unified easing system for custom bezier curves
                let (value, complete) =
                    self.interpolate_with_curve(elapsed, *duration, self.animation.curve());
                self.current_value = value;
                (value, complete)
            }
            Animation::Spring { stiffness, damping } => {
                let (value, complete) = self.update_spring(*stiffness, *damping);
                self.current_value = value;
                (value, complete)
            }
        }
    }

    /// Interpolate using an easing function.
    fn interpolate_easing(
        &self,
        elapsed: Duration,
        duration: Duration,
        easing: fn(f32) -> f32,
    ) -> (f32, bool) {
        if duration.is_zero() {
            return (self.target_value, true);
        }

        let t = elapsed.as_secs_f32() / duration.as_secs_f32();
        if t >= 1.0 {
            (self.target_value, true)
        } else {
            let eased_t = easing(t);
            let value = self.start_value + (self.target_value - self.start_value) * eased_t;
            (value, false)
        }
    }

    /// Interpolate using an EasingCurve (unified easing system).
    fn interpolate_with_curve(
        &self,
        elapsed: Duration,
        duration: Duration,
        curve: EasingCurve,
    ) -> (f32, bool) {
        if duration.is_zero() {
            return (self.target_value, true);
        }

        let t = elapsed.as_secs_f32() / duration.as_secs_f32();
        if t >= 1.0 {
            (self.target_value, true)
        } else {
            let eased_t = curve.ease(t);
            let value = self.start_value + (self.target_value - self.start_value) * eased_t;
            (value, false)
        }
    }

    /// Update spring physics simulation.
    fn update_spring(&mut self, stiffness: f32, damping: f32) -> (f32, bool) {
        // Spring physics: F = -kx - cv
        // where k = stiffness, c = damping, x = displacement, v = velocity
        const DT: f32 = 1.0 / 60.0; // Assume 60fps for physics step
        const VELOCITY_THRESHOLD: f32 = 0.001;
        const POSITION_THRESHOLD: f32 = 0.0001;

        let displacement = self.current_value - self.target_value;
        let spring_force = -stiffness * displacement;
        let damping_force = -damping * self.velocity;
        let acceleration = spring_force + damping_force;

        self.velocity += acceleration * DT;
        self.current_value += self.velocity * DT;

        // Check if animation is complete (settled)
        let is_settled = self.velocity.abs() < VELOCITY_THRESHOLD
            && (self.current_value - self.target_value).abs() < POSITION_THRESHOLD;

        if is_settled {
            self.current_value = self.target_value;
            self.velocity = 0.0;
            (self.target_value, true)
        } else {
            (self.current_value, false)
        }
    }
}

// Easing functions
fn linear(t: f32) -> f32 {
    t
}

fn ease_in(t: f32) -> f32 {
    t * t
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
    }
}

/// Get current time as Duration since program start.
fn current_time() -> Duration {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed()
}

/// Shared animation state that can be updated from watcher callbacks.
#[derive(Debug, Default)]
struct SharedAnimationState {
    /// Active animations for each parameter index.
    animations: Vec<Option<ParamAnimation>>,
    /// Current values for each parameter (either animated or direct).
    current_values: Vec<f32>,
    /// Whether any animation is active.
    has_active_animation: bool,
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
    // Fragment shader resources (for COLOR_ONLY = true)
    fragment_pipeline: Option<wgpu::RenderPipeline>,
    fragment_bind_group_layout: Option<wgpu::BindGroupLayout>,
    // Compute shader resources (for COLOR_ONLY = false)
    compute_pipeline: Option<wgpu::ComputePipeline>,
    compute_bind_group_layout: Option<wgpu::BindGroupLayout>,
    // Shared resources
    sampler: Option<wgpu::Sampler>,
    /// Shared animation state (updated by watchers, read during render).
    animation_state: Rc<RefCell<SharedAnimationState>>,
    /// Watcher guard to keep the watcher alive.
    _watcher_guard: Option<Box<dyn core::any::Any>>,
    // HDR fallback for spatial filters
    intermediate_texture: Option<wgpu::Texture>,
    intermediate_view: Option<wgpu::TextureView>,
    blit_pipeline: Option<wgpu::RenderPipeline>,
    blit_bind_group_layout: Option<wgpu::BindGroupLayout>,
    /// Cached output format from setup
    output_format: wgpu::TextureFormat,
}

impl<F: Filter> core::fmt::Debug for FilterAdapter<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilterAdapter").finish_non_exhaustive()
    }
}

impl<F: Filter> FilterAdapter<F> {
    /// Create a new filter adapter.
    #[must_use]
    pub fn new(filter: F) -> Self {
        let param_count = <F::Params as ParamArray>::LEN;
        let animation_state = Rc::new(RefCell::new(SharedAnimationState {
            animations: alloc::vec![None; param_count],
            current_values: alloc::vec![0.0; param_count],
            has_active_animation: false,
        }));

        Self {
            filter,
            fragment_pipeline: None,
            fragment_bind_group_layout: None,
            compute_pipeline: None,
            compute_bind_group_layout: None,
            sampler: None,
            animation_state,
            _watcher_guard: None,
            intermediate_texture: None,
            intermediate_view: None,
            blit_pipeline: None,
            blit_bind_group_layout: None,
            output_format: wgpu::TextureFormat::Rgba8Unorm, // Default, updated in setup
        }
    }

    /// Chain another filter onto this adapter.
    ///
    /// Returns a new `FilterAdapter` wrapping a `Chain` of both filters.
    /// Consecutive color-only filters will be fused into a single GPU pass.
    #[must_use]
    pub fn then<F2: Filter>(self, filter: F2) -> FilterAdapter<Chain<F, F2>> {
        FilterAdapter::new(Chain {
            first: self.filter,
            second: filter,
        })
    }

    /// Get the current interpolated parameters, updating any active animations.
    fn get_interpolated_params(&self) -> (Vec<f32>, bool) {
        let mut state = self.animation_state.borrow_mut();
        let param_count = <F::Params as ParamArray>::LEN;

        // Get target values from the filter
        let mut target_params = [0.0f32; 64];
        self.filter.params().write_to(&mut target_params);

        let mut needs_redraw = false;

        for i in 0..param_count {
            let target = target_params[i];

            if let Some(ref mut anim) = state.animations[i] {
                // Check if target changed during animation
                if (anim.target_value - target).abs() > f32::EPSILON {
                    // Retarget: start new animation from current position
                    anim.start_value = anim.current_value;
                    anim.target_value = target;
                    anim.start_time = current_time();
                    anim.velocity = 0.0; // Reset velocity for spring
                }

                let (value, complete) = anim.update();
                state.current_values[i] = value;

                if complete {
                    state.animations[i] = None;
                } else {
                    needs_redraw = true;
                }
            } else {
                // No animation, use target directly
                state.current_values[i] = target;
            }
        }

        state.has_active_animation = needs_redraw;
        (state.current_values.clone(), needs_redraw)
    }

    /// Start an animation for a parameter.
    ///
    /// This is called from watcher callbacks when animation metadata is detected.
    #[allow(dead_code)]
    fn start_animation(&self, param_index: usize, target: f32, animation: Animation) {
        let mut state = self.animation_state.borrow_mut();
        if param_index < state.current_values.len() {
            let start_value = state.current_values[param_index];
            state.animations[param_index] =
                Some(ParamAnimation::new(start_value, target, animation));
            state.has_active_animation = true;
        }
    }
}

impl<F: Filter> GpuFilter for FilterAdapter<F> {
    fn setup(&mut self, ctx: &FilterContext) -> impl Future<Output = ()> {
        self.output_format = ctx.output_format;

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

        // Choose pipeline based on filter type
        if F::COLOR_ONLY {
            self.setup_fragment_pipeline(ctx);
        } else {
            self.setup_compute_pipeline(ctx);
            // Spatial filters always blit from the intermediate texture.
            self.setup_blit_pipeline(ctx);
        }

        // Initialize current values from filter
        {
            let mut state = self.animation_state.borrow_mut();
            let mut target_params = [0.0f32; 64];
            self.filter.params().write_to(&mut target_params);
            let param_count = <F::Params as ParamArray>::LEN;
            for i in 0..param_count {
                state.current_values[i] = target_params[i];
            }
        }

        async {}
    }

    fn render(&mut self, input: &FilterInput, output: &FilterOutput) -> bool {
        if F::COLOR_ONLY {
            self.render_fragment(input, output)
        } else {
            self.render_compute_with_blit(input, output)
        }
    }
}

/// Check if a texture format is HDR (not supported as storage texture on Metal)
fn is_hdr_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba16Float
            | wgpu::TextureFormat::Rgba32Float
            | wgpu::TextureFormat::Rg16Float
            | wgpu::TextureFormat::Rg32Float
            | wgpu::TextureFormat::R16Float
            | wgpu::TextureFormat::R32Float
    )
}

impl<F: Filter> FilterAdapter<F> {
    /// Setup fragment shader pipeline for color-only filters.
    /// Fragment shaders can write to any render attachment format including HDR.
    fn setup_fragment_pipeline(&mut self, ctx: &FilterContext) {
        tracing::debug!(
            "[Filter] setup_fragment_pipeline: creating pipeline for {:?}",
            ctx.output_format
        );
        let preamble =
            include_str!("../../../utils/filtrate-core/src/shaders/fragment_preamble.wgsl");
        let postamble =
            include_str!("../../../utils/filtrate-core/src/shaders/fragment_postamble.wgsl");

        let mut shader_source = alloc::string::String::from(preamble);
        self.filter.fragments().write_to(&mut shader_source);
        shader_source.push_str(postamble);

        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("filter fragment shader"),
                source: wgpu::ShaderSource::Wgsl(shader_source.into()),
            });

        // Fragment shader bind group: input texture + sampler + uniforms
        // Use VERTEX_FRAGMENT visibility because WGSL module-level bindings are checked for all stages
        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("filter fragment bind group layout"),
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
                label: Some("filter fragment pipeline layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("filter fragment pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.output_format, // Native HDR support!
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

        self.fragment_pipeline = Some(pipeline);
        self.fragment_bind_group_layout = Some(bind_group_layout);
    }

    /// Setup compute shader pipeline for spatial filters.
    /// Uses standalone shader directly (not fused with preamble/postamble).
    /// Uses Rgba8Unorm for storage texture (universally supported).
    fn setup_compute_pipeline(&mut self, ctx: &FilterContext) {
        tracing::debug!(
            "[Filter] setup_compute_pipeline: creating pipeline for {:?}",
            ctx.output_format
        );
        // Spatial filters provide a complete standalone shader via fragments()
        // Don't wrap with preamble/postamble - use directly
        let mut shader_source = alloc::string::String::new();
        self.filter.fragments().write_to(&mut shader_source);
        tracing::debug!(
            "[Filter] setup_compute_pipeline: shader source length = {}",
            shader_source.len()
        );

        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("filter compute shader"),
                source: wgpu::ShaderSource::Wgsl(shader_source.into()),
            });

        // Spatial filter shaders use 3 bindings: input texture, output storage, uniforms
        // No sampler needed (they use textureLoad for sampling)
        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("filter compute bind group layout"),
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
                                format: wgpu::TextureFormat::Rgba8Unorm, // Always use Rgba8Unorm
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
                label: Some("filter compute pipeline layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("filter compute pipeline"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: ctx.pipeline_cache,
            });

        self.compute_pipeline = Some(pipeline);
        self.compute_bind_group_layout = Some(bind_group_layout);
    }

    /// Setup blit pipeline for copying intermediate Rgba8Unorm to HDR output.
    fn setup_blit_pipeline(&mut self, ctx: &FilterContext) {
        let blit_shader_source = include_str!("shaders/blit.wgsl");
        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("filter blit shader"),
                source: wgpu::ShaderSource::Wgsl(blit_shader_source.into()),
            });

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
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.output_format, // HDR format
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

        self.blit_pipeline = Some(pipeline);
        self.blit_bind_group_layout = Some(bind_group_layout);
    }

    /// Render using fragment shader (for color-only filters).
    fn render_fragment(&mut self, input: &FilterInput, output: &FilterOutput) -> bool {
        let Some(pipeline) = &self.fragment_pipeline else {
            tracing::warn!(
                "[Filter] render_fragment: fragment_pipeline is None, was setup called?"
            );
            return false;
        };
        let Some(bind_group_layout) = &self.fragment_bind_group_layout else {
            tracing::warn!("[Filter] render_fragment: bind_group_layout is None");
            return false;
        };
        let Some(sampler) = &self.sampler else {
            tracing::warn!("[Filter] render_fragment: sampler is None");
            return false;
        };
        tracing::debug!(
            "[Filter] render_fragment: rendering {}x{}",
            input.width,
            input.height
        );

        let (current_values, needs_redraw) = self.get_interpolated_params();

        // Build uniform buffer with proper alignment for WGSL:
        // - dimensions: vec2<f32> (2 floats)
        // - _padding: vec2<f32> (2 floats for 16-byte alignment)
        // - params: array<vec4<f32>, 16> (64 floats)
        let param_count = <F::Params as ParamArray>::LEN;
        let mut uniform_data = alloc::vec![0.0f32; 4 + 64]; // 4 for header + 64 for params
        uniform_data[0] = input.width as f32;
        uniform_data[1] = input.height as f32;
        // uniform_data[2] and [3] are padding (already 0)
        for (i, &value) in current_values.iter().enumerate().take(param_count) {
            uniform_data[4 + i] = value;
        }

        let uniform_buffer = input
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("filter uniform buffer"),
                contents: bytemuck::cast_slice(&uniform_data),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let bind_group = input.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filter fragment bind group"),
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = input
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("filter fragment encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("filter fragment render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..6, 0..1); // Full-screen quad
        }

        input.queue.submit([encoder.finish()]);
        needs_redraw
    }

    /// Render using compute shader (for spatial filters, SDR output).
    fn render_compute(&mut self, input: &FilterInput, output: &FilterOutput) -> bool {
        let Some(pipeline) = &self.compute_pipeline else {
            return false;
        };
        let Some(bind_group_layout) = &self.compute_bind_group_layout else {
            return false;
        };

        let (current_values, needs_redraw) = self.get_interpolated_params();

        // Spatial filter uniform layout: dimensions first, then params
        // Matches blur.wgsl: struct Uniforms { dimensions: vec2<f32>, radius: f32, _padding: f32 }
        let param_count = <F::Params as ParamArray>::LEN;
        let mut uniform_data = alloc::vec![0.0f32; 4 + param_count]; // dimensions (2) + padding (2) + params
        uniform_data[0] = input.width as f32;
        uniform_data[1] = input.height as f32;
        // uniform_data[2] is first param, [3] is second param, etc.
        for (i, &value) in current_values.iter().enumerate().take(param_count) {
            uniform_data[2 + i] = value;
        }

        let uniform_buffer = input
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("filter uniform buffer"),
                contents: bytemuck::cast_slice(&uniform_data),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        // Spatial filters use 3 bindings: input texture, output storage, uniforms
        let bind_group = input.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filter compute bind group"),
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&output.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = input
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("filter compute encoder"),
            });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("filter compute pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);
            let workgroups_x = (output.width + 7) / 8;
            let workgroups_y = (output.height + 7) / 8;
            compute_pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
        }

        input.queue.submit([encoder.finish()]);
        needs_redraw
    }

    /// Render using compute shader with blit to HDR output (for spatial filters).
    fn render_compute_with_blit(&mut self, input: &FilterInput, output: &FilterOutput) -> bool {
        let Some(compute_pipeline) = &self.compute_pipeline else {
            tracing::warn!("[Filter] render_compute_with_blit: compute_pipeline is None");
            return false;
        };
        let Some(compute_bind_group_layout) = &self.compute_bind_group_layout else {
            tracing::warn!("[Filter] render_compute_with_blit: compute_bind_group_layout is None");
            return false;
        };
        let Some(blit_pipeline) = &self.blit_pipeline else {
            tracing::warn!("[Filter] render_compute_with_blit: blit_pipeline is None");
            return false;
        };
        let Some(blit_bind_group_layout) = &self.blit_bind_group_layout else {
            tracing::warn!("[Filter] render_compute_with_blit: blit_bind_group_layout is None");
            return false;
        };
        let Some(sampler) = &self.sampler else {
            tracing::warn!("[Filter] render_compute_with_blit: sampler is None");
            return false;
        };
        tracing::debug!(
            "[Filter] render_compute_with_blit: rendering {}x{}",
            input.width,
            input.height
        );

        // Ensure intermediate texture exists and is correct size
        let needs_new_intermediate = self.intermediate_texture.as_ref().map_or(true, |tex| {
            tex.width() != output.width || tex.height() != output.height
        });

        if needs_new_intermediate {
            let texture = input.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("filter intermediate texture"),
                size: wgpu::Extent3d {
                    width: output.width,
                    height: output.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.intermediate_texture = Some(texture);
            self.intermediate_view = Some(view);
        }

        let intermediate_view = self.intermediate_view.as_ref().unwrap();

        let (current_values, needs_redraw) = self.get_interpolated_params();

        // Step 1: Compute to intermediate
        // Spatial filter uniform layout: dimensions first, then params
        let param_count = <F::Params as ParamArray>::LEN;
        let mut uniform_data = alloc::vec![0.0f32; 4 + param_count];
        uniform_data[0] = input.width as f32;
        uniform_data[1] = input.height as f32;
        for (i, &value) in current_values.iter().enumerate().take(param_count) {
            uniform_data[2 + i] = value;
        }

        let uniform_buffer = input
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("filter uniform buffer"),
                contents: bytemuck::cast_slice(&uniform_data),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        // Compute bind group: 3 bindings (input texture, output storage, uniforms)
        let compute_bind_group = input.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filter compute bind group"),
            layout: compute_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(intermediate_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = input
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("filter compute+blit encoder"),
            });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("filter compute pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(compute_pipeline);
            compute_pass.set_bind_group(0, &compute_bind_group, &[]);
            let workgroups_x = (output.width + 7) / 8;
            let workgroups_y = (output.height + 7) / 8;
            compute_pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
        }

        // Step 2: Blit intermediate to HDR output
        let blit_bind_group = input.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filter blit bind group"),
            layout: blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(intermediate_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("filter blit render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            render_pass.set_pipeline(blit_pipeline);
            render_pass.set_bind_group(0, &blit_bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }

        input.queue.submit([encoder.finish()]);
        needs_redraw
    }
}

// Need wgpu::util for BufferInitDescriptor
use wgpu::util::DeviceExt;

// ============================================================================
// Animated Filter Adapter - wraps FilterAdapter with watcher for animation
// ============================================================================

/// A filter adapter that watches for value changes and handles animation metadata.
///
/// This wraps a `FilterAdapter` and sets up a watcher on the underlying signal
/// to detect animation metadata when values change.
pub struct AnimatedFilterAdapter<F: Filter, S: Signal<Output = f32>> {
    adapter: FilterAdapter<F>,
    /// Keep signal alive for the watcher to work.
    #[allow(dead_code)]
    signal: S,
    /// Watcher guard to keep the watcher alive.
    _guard: Rc<dyn core::any::Any>,
}

impl<F: Filter, S: Signal<Output = f32>> core::fmt::Debug for AnimatedFilterAdapter<F, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnimatedFilterAdapter")
            .finish_non_exhaustive()
    }
}

impl<F: Filter, S: Signal<Output = f32> + 'static> AnimatedFilterAdapter<F, S> {
    /// Create a new animated filter adapter.
    pub fn new<C>(filter_fn: C, signal: S) -> Self
    where
        C: FnOnce(S) -> F,
        S: Clone,
    {
        let filter = filter_fn(signal.clone());
        let adapter = FilterAdapter::new(filter);
        let animation_state = adapter.animation_state.clone();

        // Set up watcher to detect animation metadata
        let guard = signal.watch(move |context| {
            let animation = context.metadata().try_get::<Animation>();
            let value = context.into_value();
            if let Some(animation) = animation {
                // Animation metadata present - start animation
                let mut state = animation_state.borrow_mut();
                if !state.current_values.is_empty() {
                    let start_value = state.current_values[0];
                    state.animations[0] = Some(ParamAnimation::new(start_value, value, animation));
                    state.has_active_animation = true;
                }
            }
            // If no animation metadata, the filter will use the value directly
        });

        Self {
            adapter,
            signal,
            _guard: Rc::new(guard),
        }
    }
}

impl<F: Filter, S: Signal<Output = f32> + 'static> GpuFilter for AnimatedFilterAdapter<F, S> {
    fn setup(&mut self, ctx: &FilterContext) -> impl Future<Output = ()> {
        GpuFilter::setup(&mut self.adapter, ctx)
    }

    fn render(&mut self, input: &FilterInput, output: &FilterOutput) -> bool {
        GpuFilter::render(&mut self.adapter, input, output)
    }
}

/// Extension methods for applying filters to views.
pub trait FilterViewExt: View + Sized {
    /// Apply a `GpuFilter` to this view.
    ///
    /// For the high-level `Filter` API with automatic optimization,
    /// use convenience methods like `.blur()`, `.brightness()`, etc.
    fn filter<F: GpuFilter>(self, filter: F) -> FilteredView<Self, F> {
        FilteredView::new(self, filter)
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
    fn blur<T: IntoSignalF32>(
        self,
        radius: T,
    ) -> FilteredView<Self, FilterAdapter<filtrate_core::filters::Blur<T::Signal>>>
    where
        T::Signal: 'static,
    {
        FilteredView::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Blur(radius.into_signal_f32())),
        )
    }

    /// Apply a brightness filter.
    fn brightness<T: IntoSignalF32>(
        self,
        amount: T,
    ) -> FilteredView<Self, FilterAdapter<filtrate_core::filters::Brightness<T::Signal>>>
    where
        T::Signal: 'static,
    {
        FilteredView::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Brightness(amount.into_signal_f32())),
        )
    }

    /// Apply a contrast filter.
    fn contrast<T: IntoSignalF32>(
        self,
        amount: T,
    ) -> FilteredView<Self, FilterAdapter<filtrate_core::filters::Contrast<T::Signal>>>
    where
        T::Signal: 'static,
    {
        FilteredView::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Contrast(amount.into_signal_f32())),
        )
    }

    /// Apply a saturation filter.
    fn saturation<T: IntoSignalF32>(
        self,
        amount: T,
    ) -> FilteredView<Self, FilterAdapter<filtrate_core::filters::Saturation<T::Signal>>>
    where
        T::Signal: 'static,
    {
        FilteredView::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Saturation(amount.into_signal_f32())),
        )
    }

    /// Apply a grayscale filter.
    fn grayscale<T: IntoSignalF32>(
        self,
        intensity: T,
    ) -> FilteredView<Self, FilterAdapter<filtrate_core::filters::Grayscale<T::Signal>>>
    where
        T::Signal: 'static,
    {
        FilteredView::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Grayscale(
                intensity.into_signal_f32(),
            )),
        )
    }

    /// Apply a hue rotation filter.
    fn hue_rotation<T: IntoSignalF32>(
        self,
        angle: T,
    ) -> FilteredView<Self, FilterAdapter<filtrate_core::filters::HueRotation<T::Signal>>>
    where
        T::Signal: 'static,
    {
        FilteredView::new(
            self,
            FilterAdapter::new(filtrate_core::filters::HueRotation(angle.into_signal_f32())),
        )
    }

    /// Apply an invert filter.
    fn invert(self) -> FilteredView<Self, FilterAdapter<filtrate_core::filters::Invert>> {
        FilteredView::new(self, FilterAdapter::new(filtrate_core::filters::Invert))
    }

    /// Apply an opacity filter.
    fn opacity<T: IntoSignalF32>(
        self,
        amount: T,
    ) -> FilteredView<Self, FilterAdapter<filtrate_core::filters::Opacity<T::Signal>>>
    where
        T::Signal: 'static,
    {
        FilteredView::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Opacity(amount.into_signal_f32())),
        )
    }

    /// Apply a sepia filter.
    fn sepia<T: IntoSignalF32>(
        self,
        intensity: T,
    ) -> FilteredView<Self, FilterAdapter<filtrate_core::filters::Sepia<T::Signal>>>
    where
        T::Signal: 'static,
    {
        FilteredView::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Sepia(intensity.into_signal_f32())),
        )
    }

    /// Apply a sharpen filter.
    fn sharpen<T: IntoSignalF32>(
        self,
        amount: T,
    ) -> FilteredView<Self, FilterAdapter<filtrate_core::filters::Sharpen<T::Signal>>>
    where
        T::Signal: 'static,
    {
        FilteredView::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Sharpen(amount.into_signal_f32())),
        )
    }

    /// Apply a vignette filter.
    fn vignette<R: IntoSignalF32, S: IntoSignalF32>(
        self,
        radius: R,
        softness: S,
    ) -> FilteredView<Self, FilterAdapter<filtrate_core::filters::Vignette<R::Signal, S::Signal>>>
    {
        FilteredView::new(
            self,
            FilterAdapter::new(filtrate_core::filters::Vignette(
                radius.into_signal_f32(),
                softness.into_signal_f32(),
            )),
        )
    }
}

impl<V: View> FilterViewExt for V {}
