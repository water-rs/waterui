//! A user shader as a view: a WGSL fragment shader filling the view's frame.
//!
//! The fragment is WGSL against a prelude — `uniforms.time`,
//! `uniforms.resolution`, `shader_param(index)` / `params` (`array<vec4<f32>, 16>`),
//! a `uv` in `[0, 1]` (top-left origin) and `@fragment fn main` — with user
//! parameters bound as reactive `f32` signals. It writes premultiplied linear
//! Display P3; the engine's colour helpers (`cherenkov_linear_srgb`,
//! `cherenkov_srgb`, ...) convert from sRGB.
//!
//! WGSL syntax, core WebGPU validation, the `@fragment fn main` entry point,
//! and the 64-parameter limit are checked at construction (or at compile time
//! via [`shader!`](crate::shader)).
//!
//! Whether the shader animates follows a signal too: while it is `true` the
//! content asks for a frame after every frame so `uniforms.time` advances,
//! and while it is `false` the shader renders once per change of its
//! parameters or size and the host stays idle.

extern crate alloc;

use alloc::borrow::Cow;
use alloc::string::{String, ToString as _};
#[cfg(feature = "gpu")]
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
#[cfg(feature = "gpu")]
use core::mem::size_of_val;

#[cfg(feature = "gpu")]
use arc_swap::{ArcSwap, ArcSwapOption};
use cherenkov_record::wgsl::validate_paint_wgsl;
#[cfg(feature = "gpu")]
use cherenkov_record::wgsl::{PAINT_ENTRY, paint_module};
pub use cherenkov_record::wgsl::{PAINT_MAX_PARAMS as SHADER_MAX_PARAMS, ShaderError};
use nami::watcher::BoxWatcherGuard;
use nami::{Computed, Signal, SignalExt};
use waterui_core::layout::StretchAxis;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, IntoSignalF32, Native, NativeView, Str, View, contract_violation};

use crate::draw::{Draw as _, Paint, Recorder, ShaderId, ShaderPaint, kurbo::Rect};
#[cfg(feature = "gpu")]
use crate::gpu::{Context, Frame, GpuContent, GpuView, RedrawHandle};
use crate::scene::resources::{RecordingResources, Registered, SceneResources};
use crate::scene_view::{
    NextFrame, SceneContent, SceneFrame, SceneInvalidator, SceneUpdate, invalidate_on_change,
};
use crate::source::ShaderSource;

/// A view painted by a WGSL fragment shader.
///
/// # Layout Behavior
///
/// Stretches on both axes; constrain it with `.frame()`.
pub struct ShaderPaintView {
    fragment: Cow<'static, str>,
    continuous: Computed<bool>,
    params: Vec<Computed<f32>>,
    label: Computed<Option<Str>>,
    value: Computed<Option<Str>>,
}

impl fmt::Debug for ShaderPaintView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShaderPaintView")
            .field("param_count", &self.params.len())
            .finish_non_exhaustive()
    }
}

impl ShaderPaintView {
    /// Builds and validates a shader paint from its WGSL fragment body.
    ///
    /// Line and column numbers in returned [`ShaderError`] diagnostics count
    /// in `fragment` itself.
    ///
    /// # Errors
    ///
    /// Returns [`ShaderError`] when `fragment` fails WGSL parsing, fails `naga`
    /// validation, or does not define `@fragment fn main`.
    pub fn new(fragment: impl Into<Cow<'static, str>>) -> Result<Self, ShaderError> {
        let fragment = fragment.into();
        validate_paint_wgsl(&fragment)?;
        Ok(Self::from_validated(fragment))
    }

    /// Internal constructor used by [`shader!`](crate::shader) after
    /// compile-time WGSL validation succeeds.
    #[doc(hidden)]
    #[must_use]
    pub fn __from_validated_wgsl(fragment: &'static str) -> Self {
        Self::from_validated(Cow::Borrowed(fragment))
    }

    fn from_validated(fragment: Cow<'static, str>) -> Self {
        Self {
            fragment,
            continuous: nami::constant(false).computed(),
            params: Vec::new(),
            label: Computed::constant(None),
            value: Computed::constant(None),
        }
    }

    /// Whether the shader re-renders every frame so `uniforms.time` advances,
    /// following a signal.
    #[must_use]
    pub fn continuous(mut self, continuous: impl IntoComputed<bool>) -> Self {
        self.continuous = continuous.into_computed();
        self
    }

    /// Adds a scalar parameter, readable in WGSL via `shader_param(index)` or
    /// `params[index / 4u][index % 4u]`.
    ///
    /// # Errors
    ///
    /// Returns [`ShaderError::TooManyParams`] if the shader already carries
    /// [`SHADER_MAX_PARAMS`] parameters.
    pub fn param(mut self, value: impl IntoSignalF32) -> Result<Self, ShaderError> {
        if self.params.len() >= SHADER_MAX_PARAMS {
            return Err(ShaderError::TooManyParams {
                count: self.params.len() + 1,
                max: SHADER_MAX_PARAMS,
            });
        }
        self.params.push(value.into_signal_f32().computed());
        Ok(self)
    }

    /// Appends a sequence of scalar parameters in order.
    ///
    /// # Errors
    ///
    /// Returns [`ShaderError::TooManyParams`] without modifying the shader if
    /// appending `values` would exceed [`SHADER_MAX_PARAMS`].
    pub fn params<P: IntoSignalF32>(
        mut self,
        values: impl IntoIterator<Item = P>,
    ) -> Result<Self, ShaderError> {
        let iter = values.into_iter();
        let remaining = SHADER_MAX_PARAMS - self.params.len();
        let (lower, _) = iter.size_hint();
        if lower > remaining {
            return Err(ShaderError::TooManyParams {
                count: self.params.len() + lower,
                max: SHADER_MAX_PARAMS,
            });
        }
        let mut collected = Vec::with_capacity(lower.min(remaining));
        for value in iter {
            if collected.len() >= remaining {
                return Err(ShaderError::TooManyParams {
                    count: self.params.len() + collected.len() + 1,
                    max: SHADER_MAX_PARAMS,
                });
            }
            collected.push(value.into_signal_f32().computed());
        }
        self.params.extend(collected);
        Ok(self)
    }

    /// The number of parameters currently bound to this shader.
    #[must_use]
    pub const fn param_count(&self) -> usize {
        self.params.len()
    }

    /// The accessible name the shader offers. An app's own `.a11y_label(…)`
    /// overrides it.
    #[must_use]
    pub fn offered_label(mut self, label: impl IntoComputed<Option<Str>>) -> Self {
        self.label = label.into_computed();
        self
    }

    /// The accessible value the shader offers. An app's own `.a11y_value(…)`
    /// overrides it.
    #[must_use]
    pub fn offered_value(mut self, value: impl IntoComputed<Option<Str>>) -> Self {
        self.value = value.into_computed();
        self
    }

    /// Lowers this view into [`SceneContent`] that fills its box with
    /// [`Paint::Shader`] — the realization self-drawn backends run on both the
    /// GPU and CPU painters.
    #[must_use]
    pub fn into_scene_content(self) -> impl SceneContent {
        ShaderPaintScene {
            fragment: self.fragment,
            continuous: self.continuous,
            params: self.params,
            label: self.label,
            value: self.value,
            watchers: None,
            registered: None,
            recorded: None,
        }
    }

    /// Lowers this view into a [`GpuView`] running its fragment on the
    /// engine's `wgpu` device — the realization native backends without
    /// scene-level `Paint::Shader` run.
    #[cfg(feature = "gpu")]
    #[must_use]
    pub fn into_gpu_view(self) -> GpuView<impl GpuContent> {
        let shared = Arc::new(Shared::default());
        let posting = Posting::new(self.continuous, self.params, Arc::clone(&shared));
        GpuView::new(ShaderContent {
            fragment: self.fragment,
            shared,
            gpu: None,
        })
        // The hook also keeps the posting's watchers alive with the view.
        .on_frame(move || posting.post())
        .offered_label(self.label)
        .offered_value(self.value)
    }
}

impl NativeView for ShaderPaintView {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

impl View for ShaderPaintView {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }
}

struct RecordedState {
    continuous: bool,
    params_bits: Vec<u32>,
    width_bits: u32,
    height_bits: u32,
}

impl RecordedState {
    fn matches(&self, continuous: bool, params: &[f32], width: f32, height: f32) -> bool {
        self.continuous == continuous
            && self.width_bits == width.to_bits()
            && self.height_bits == height.to_bits()
            && self.params_bits.len() == params.len()
            && self
                .params_bits
                .iter()
                .zip(params)
                .all(|(&recorded, current)| recorded == current.to_bits())
    }
}

struct ShaderPaintScene {
    fragment: Cow<'static, str>,
    continuous: Computed<bool>,
    params: Vec<Computed<f32>>,
    label: Computed<Option<Str>>,
    value: Computed<Option<Str>>,
    watchers: Option<Vec<BoxWatcherGuard>>,
    registered: Option<(bool, Registered<ShaderId>)>,
    recorded: Option<RecordedState>,
}

impl ShaderPaintScene {
    fn snapshot_params(&self) -> Vec<f32> {
        self.params.iter().map(Signal::snapshot).collect()
    }
}

impl SceneContent for ShaderPaintScene {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> NextFrame {
        let continuous = self.continuous.snapshot();
        let params = self.snapshot_params();
        let params_bits: Vec<u32> = params.iter().map(|v| v.to_bits()).collect();
        let next = if continuous {
            NextFrame::Perpetual
        } else {
            NextFrame::Idle
        };

        if self
            .registered
            .as_ref()
            .is_none_or(|(was_continuous, _)| *was_continuous != continuous)
        {
            let source = ShaderSource::wgsl(self.fragment.clone());
            let source = if continuous {
                source.time_dependent()
            } else {
                source
            };
            match resources.shader(source) {
                Ok(handle) => self.registered = Some((continuous, handle)),
                Err(error) => {
                    self.registered = None;
                    contract_violation!("shader paint failed to register: {error}");
                }
            }
        }

        if width > 0.0
            && height > 0.0
            && let Some((_, handle)) = &self.registered
            && let Some(shader) = resources.name(handle)
        {
            recorder.fill(
                Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
                Paint::Shader(ShaderPaint {
                    shader,
                    uniforms: params,
                }),
            );
        }

        self.recorded = Some(RecordedState {
            continuous,
            params_bits,
            width_bits: width.to_bits(),
            height_bits: height.to_bits(),
        });
        next
    }

    fn rebuild_for_engine(&mut self) {
        self.registered = None;
        self.recorded = None;
    }

    fn update_scene(&mut self, _resources: &SceneResources, frame: SceneFrame) -> SceneUpdate {
        let Some(recorded) = &self.recorded else {
            return SceneUpdate::Record;
        };
        let continuous = self.continuous.snapshot();
        let params = self.snapshot_params();
        if recorded.matches(continuous, &params, frame.width, frame.height) {
            SceneUpdate::Retained {
                next: if continuous {
                    NextFrame::Perpetual
                } else {
                    NextFrame::Idle
                },
            }
        } else {
            SceneUpdate::Record
        }
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.watchers = invalidator.map(|inv| {
            let mut watchers = Vec::with_capacity(3 + self.params.len());
            watchers.push(invalidate_on_change(&inv, &self.continuous));
            watchers.push(invalidate_on_change(&inv, &self.label));
            watchers.push(invalidate_on_change(&inv, &self.value));
            for param in &self.params {
                watchers.push(invalidate_on_change(&inv, param));
            }
            watchers
        });
    }

    fn accessibility_label(&self) -> Option<String> {
        self.label.snapshot().map(|label| label.to_string())
    }

    fn accessibility_value(&self) -> Option<String> {
        self.value.snapshot().map(|value| value.to_string())
    }
}

/// What the UI side last read from the signals.
#[cfg(feature = "gpu")]
struct Posted {
    continuous: bool,
    params: [f32; SHADER_MAX_PARAMS],
}

/// The UI side's mailbox to the render thread.
#[cfg(feature = "gpu")]
struct Shared {
    posted: ArcSwap<Posted>,
    /// The host's wake, known once the content is set up.
    redraw: ArcSwapOption<RedrawHandle>,
}

#[cfg(feature = "gpu")]
impl Default for Shared {
    fn default() -> Self {
        Self {
            posted: ArcSwap::from_pointee(Posted {
                continuous: false,
                params: [0.0; SHADER_MAX_PARAMS],
            }),
            redraw: ArcSwapOption::empty(),
        }
    }
}

#[cfg(feature = "gpu")]
impl Shared {
    fn wake(&self) {
        if let Some(redraw) = &*self.redraw.load() {
            redraw.request_redraw();
        }
    }
}

/// The UI-thread half: reads the signals into the mailbox before each frame
/// and wakes an idle host when one changes.
#[cfg(feature = "gpu")]
struct Posting {
    continuous: Computed<bool>,
    params: Vec<Computed<f32>>,
    shared: Arc<Shared>,
    _watchers: Vec<BoxWatcherGuard>,
}

#[cfg(feature = "gpu")]
impl Posting {
    fn new(continuous: Computed<bool>, params: Vec<Computed<f32>>, shared: Arc<Shared>) -> Self {
        let mut watchers = Vec::with_capacity(1 + params.len());
        let continuous_shared = Arc::clone(&shared);
        watchers
            .push(Box::new(continuous.watch(move |_| continuous_shared.wake())) as BoxWatcherGuard);
        for param in &params {
            let param_shared = Arc::clone(&shared);
            watchers.push(Box::new(param.watch(move |_| param_shared.wake())) as BoxWatcherGuard);
        }
        let posting = Self {
            continuous,
            params,
            shared,
            _watchers: watchers,
        };
        posting.post();
        posting
    }

    fn post(&self) {
        let mut packed = [0.0; SHADER_MAX_PARAMS];
        for (dst, signal) in packed.iter_mut().zip(&self.params) {
            *dst = signal.snapshot();
        }
        self.shared.posted.store(Arc::new(Posted {
            continuous: self.continuous.snapshot(),
            params: packed,
        }));
    }
}

/// The render-thread half: the fragment's pipeline on the engine's device.
#[cfg(feature = "gpu")]
struct ShaderContent {
    fragment: Cow<'static, str>,
    shared: Arc<Shared>,
    gpu: Option<Program>,
}

#[cfg(feature = "gpu")]
struct Program {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    params: wgpu::Buffer,
}

#[cfg(feature = "gpu")]
const fn uniform_entry(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    }
}

#[cfg(feature = "gpu")]
fn uniform_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(feature = "gpu")]
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::unnecessary_wraps,
        reason = "wasm32 does not synchronously poll wgpu validation error scopes"
    )
)]
fn compile_program(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    fragment: &str,
) -> Result<Program, String> {
    let globals_size = size_of_val(&[0.0_f32; 4]) as u64;
    let params_size = size_of_val(&[0.0_f32; SHADER_MAX_PARAMS]) as u64;
    let module_source = paint_module(fragment);

    // Defence-in-depth for shaders that passed CPU-side naga validation
    // yet fail the backend driver's own compiler or pipeline limits.
    #[cfg(not(target_arch = "wasm32"))]
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shader paint"),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(module_source)),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shader paint"),
        entries: &[
            uniform_entry(0, globals_size),
            uniform_entry(1, params_size),
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("shader paint"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shader paint"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(PAINT_ENTRY),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(error) = pollster::block_on(scope.pop()) {
        return Err(error.to_string());
    }
    let globals = uniform_buffer(device, "shader paint globals", globals_size);
    let params = uniform_buffer(device, "shader paint params", params_size);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shader paint"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params.as_entire_binding(),
            },
        ],
    });
    Ok(Program {
        pipeline,
        bind_group,
        globals,
        params,
    })
}

#[cfg(feature = "gpu")]
impl GpuContent for ShaderContent {
    fn setup(&mut self, gpu: &Context<'_>) {
        self.shared
            .redraw
            .store(Some(Arc::new(gpu.redraw().clone())));
        match compile_program(gpu.device(), gpu.format(), &self.fragment) {
            Ok(program) => self.gpu = Some(program),
            Err(error) => {
                tracing::error!("shader paint device compilation failed: {error}");
                self.gpu = None;
            }
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let posted = self.shared.posted.load();
        #[expect(clippy::cast_precision_loss, reason = "texture sides fit f32 exactly")]
        let globals = [
            frame.elapsed().as_secs_f32(),
            0.0,
            frame.width() as f32,
            frame.height() as f32,
        ];
        frame
            .queue()
            .write_buffer(&gpu.globals, 0, bytemuck::cast_slice(&globals));
        frame
            .queue()
            .write_buffer(&gpu.params, 0, bytemuck::cast_slice(&posted.params));
        let mut encoder = frame
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shader paint"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shader paint"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&gpu.pipeline);
            pass.set_bind_group(0, &gpu.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        frame.queue().submit([encoder.finish()]);
        if posted.continuous {
            frame.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::cell::Cell;

    use nami::Binding;
    use waterui_core::{AnyView, Environment, Native, View};

    use crate::draw::{
        FontId, ImageData, ImageId, ImageLimits, ImageStorage, Instant, LayoutSize, ResourceError,
        Rgba8, Rgba16F,
    };
    use crate::scene::resources::{
        Handle, ResourceHandle, SceneBackend, SceneResources, ShaderBackend,
    };
    use crate::scene_view::{NextFrame, SceneContent, SceneFrame, SceneUpdate};
    use crate::source::FontSource;

    use super::*;

    const FRAGMENT: &str = "
@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return cherenkov_linear_srgb(vec3<f32>(uv.x, shader_param(0u), shader_param(1u))).xyzz;
}
";

    struct DummyShaderHandle(ShaderId);

    impl ResourceHandle for DummyShaderHandle {
        type Id = ShaderId;

        fn id(&self) -> ShaderId {
            self.0
        }
    }

    struct DummyShaders {
        next: Cell<u64>,
    }

    impl SceneBackend for DummyShaders {
        fn register_font(&self, _source: FontSource) -> Result<Handle<FontId>, ResourceError> {
            Err(ResourceError::Unsupported("font"))
        }

        fn register_rgba8(
            &self,
            _data: ImageData<Rgba8>,
        ) -> Result<Handle<ImageId>, ResourceError> {
            Err(ResourceError::Unsupported("rgba8"))
        }

        fn register_rgba16f(
            &self,
            _data: ImageData<Rgba16F>,
        ) -> Result<Handle<ImageId>, ResourceError> {
            Err(ResourceError::Unsupported("rgba16f"))
        }

        fn image_limits(&self) -> ImageLimits {
            ImageLimits {
                max_dimension: 1024,
                max_bytes: 1024 * 1024,
                storage: ImageStorage::CPU,
            }
        }
    }

    impl ShaderBackend for DummyShaders {
        fn register_shader(
            &self,
            _source: ShaderSource,
        ) -> Result<Handle<ShaderId>, ResourceError> {
            let id = self.next.get() + 1;
            self.next.set(id);
            Ok(Handle::new(DummyShaderHandle(ShaderId::new(id))))
        }
    }

    #[test]
    fn a_shader_view_is_a_stretching_native_view() {
        let view = ShaderPaintView::new(FRAGMENT).expect("valid fragment shader");
        assert_eq!(
            waterui_core::NativeView::stretch_axis(&view),
            StretchAxis::Both
        );
        let body = AnyView::new(view.body(&Environment::new()))
            .downcast::<Native<ShaderPaintView>>()
            .expect("a shader view's body is Native<ShaderPaintView>");
        assert_eq!(
            waterui_core::NativeView::stretch_axis(body.as_inner()),
            StretchAxis::Both
        );
    }

    #[test]
    fn new_rejects_invalid_wgsl_with_source_line() {
        let err = ShaderPaintView::new(
            "\n@fragment\nfn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {\n    return vec4<f32>(uv, 0.0;\n}\n",
        )
        .expect_err("syntax error must fail construction");
        match err {
            ShaderError::Parse(diagnostic) => {
                assert!(
                    diagnostic.contains(":4:"),
                    "parse diagnostic should name user source line 4, got:\n{diagnostic}"
                );
            }
            other => panic!("expected ShaderError::Parse, got {other:?}"),
        }
    }

    #[test]
    fn new_rejects_type_error_with_validation_error() {
        let err = ShaderPaintView::new(
            "@fragment\nfn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {\n    return uv;\n}\n",
        )
        .expect_err("mismatched return type must fail validation");
        assert!(
            matches!(err, ShaderError::Validation(_)),
            "expected ShaderError::Validation, got {err:?}"
        );
    }

    #[test]
    fn new_rejects_missing_fragment_entry_point() {
        let err = ShaderPaintView::new(
            "fn helper(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(uv, 0.0, 1.0); }",
        )
        .expect_err("missing @fragment fn main must be rejected");
        assert_eq!(err, ShaderError::MissingEntryPoint);
    }

    #[test]
    fn param_and_params_reject_overflow_above_capacity() {
        let mut view = ShaderPaintView::new(FRAGMENT).expect("valid shader");
        view = view
            .params([0.0_f32; SHADER_MAX_PARAMS])
            .expect("exact capacity is accepted");
        assert_eq!(view.param_count(), SHADER_MAX_PARAMS);
        assert_eq!(
            view.param(1.0_f32).unwrap_err(),
            ShaderError::TooManyParams {
                count: SHADER_MAX_PARAMS + 1,
                max: SHADER_MAX_PARAMS,
            }
        );

        let view = ShaderPaintView::new(FRAGMENT).expect("valid shader");
        assert_eq!(
            view.params([0.0_f32; SHADER_MAX_PARAMS + 1]).unwrap_err(),
            ShaderError::TooManyParams {
                count: SHADER_MAX_PARAMS + 1,
                max: SHADER_MAX_PARAMS,
            }
        );
    }

    #[test]
    fn scene_content_retains_static_recordings_and_invalidates_on_signal_changes() {
        let continuous = Binding::container(false);
        let param0 = Binding::container(1.0_f32);
        let mut scene = ShaderPaintView::new(FRAGMENT)
            .expect("valid fragment shader")
            .continuous(continuous.clone())
            .params([param0.computed(), nami::constant(2.0_f32).computed()])
            .expect("within param limit")
            .into_scene_content();

        let wakes = Rc::new(Cell::new(0_usize));
        let wakes_ref = Rc::clone(&wakes);
        scene.set_invalidator(Some(Rc::new(move || wakes_ref.set(wakes_ref.get() + 1))));

        let size = LayoutSize::new();
        let backend = Rc::new(DummyShaders { next: Cell::new(0) });
        let table = SceneResources::with_shaders(Rc::clone(&backend), backend);
        let mut recording = table.recording();
        let content = crate::draw::Content::record(&size, |recorder| {
            let next = scene.build_scene(recorder, &mut recording, 40.0, 20.0);
            assert_eq!(next, NextFrame::Idle);
        });
        let _held = recording.finish();
        assert!(!content.is_empty());

        let retained = scene.update_scene(
            &table,
            SceneFrame {
                width: 40.0,
                height: 20.0,
                time: Instant::now(),
            },
        );
        assert_eq!(
            retained,
            SceneUpdate::Retained {
                next: NextFrame::Idle
            }
        );

        param0.set(3.0);
        assert_eq!(wakes.get(), 1);
        assert_eq!(
            scene.update_scene(
                &table,
                SceneFrame {
                    width: 40.0,
                    height: 20.0,
                    time: Instant::now(),
                },
            ),
            SceneUpdate::Record
        );

        continuous.set(true);
        assert_eq!(wakes.get(), 2);
        let mut recording = table.recording();
        let _ = crate::draw::Content::record(&size, |recorder| {
            let next = scene.build_scene(recorder, &mut recording, 40.0, 20.0);
            assert_eq!(next, NextFrame::Perpetual);
        });
        let _held = recording.finish();
        assert_eq!(
            scene.update_scene(
                &table,
                SceneFrame {
                    width: 40.0,
                    height: 20.0,
                    time: Instant::now(),
                },
            ),
            SceneUpdate::Retained {
                next: NextFrame::Perpetual
            }
        );
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn signals_reach_the_render_side_and_wake_the_host() {
        use core::sync::atomic::{AtomicUsize, Ordering};

        let continuous = Binding::container(false);
        let param0 = Binding::container(1.0_f32);
        let param1 = Binding::container(2.0_f32);
        let shared = Arc::new(Shared::default());
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&wakes);
        shared
            .redraw
            .store(Some(Arc::new(RedrawHandle::new(move || {
                count.fetch_add(1, Ordering::Relaxed);
            }))));
        let posting = Posting::new(
            continuous.computed(),
            vec![param0.computed(), param1.computed()],
            Arc::clone(&shared),
        );
        assert_eq!(&shared.posted.load().params[..3], &[1.0, 2.0, 0.0]);
        assert!(!shared.posted.load().continuous);

        param0.set(3.0);
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        continuous.set(true);
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
        posting.post();
        assert_eq!(&shared.posted.load().params[..3], &[3.0, 2.0, 0.0]);
        assert!(shared.posted.load().continuous);
    }

    #[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
    #[test]
    fn gpu_error_scope_catches_invalid_pipeline_without_uncaptured_error() {
        let instance = wgpu::Instance::default();
        let Some(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            }))
            .ok()
        else {
            return;
        };
        let Ok((device, _queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        else {
            return;
        };
        device.on_uncaptured_error(Arc::new(|err| {
            panic!("validation error escaped error scope: {err}");
        }));

        assert!(
            compile_program(&device, wgpu::TextureFormat::Rgba8UnormSrgb, FRAGMENT).is_ok(),
            "valid fragment must compile inside device scope"
        );
        assert!(
            compile_program(&device, wgpu::TextureFormat::Rgba8UnormSrgb, "invalid wgsl").is_err(),
            "invalid module must fail inside validation scope without triggering on_uncaptured_error"
        );
    }
}
