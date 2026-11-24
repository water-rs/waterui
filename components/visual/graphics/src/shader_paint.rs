//! A user shader as a view: a WGSL fragment shader filling the view's frame.
//!
//! The fragment is WGSL against a prelude — `uniforms.time`,
//! `uniforms.resolution`, a `uv` in `[0, 1]` (top-left origin) and
//! `@fragment fn main` — with user uniforms passed as a flat `f32` list that
//! follows a signal. It writes premultiplied linear Display P3; the engine's
//! colour helpers (`cherenkov_linear_srgb`, `cherenkov_srgb`, ...) are
//! available to convert from sRGB.
//!
//! Whether the shader animates follows a signal too: while it is `true` the
//! content asks for a frame after every frame so `uniforms.time` advances,
//! and while it is `false` the shader renders once per change of its
//! uniforms or size and the host stays idle.

extern crate alloc;

use alloc::borrow::Cow;
#[cfg(feature = "gpu")]
use alloc::boxed::Box;
use alloc::string::{String, ToString as _};
#[cfg(feature = "gpu")]
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
#[cfg(feature = "gpu")]
use core::mem::size_of_val;

#[cfg(feature = "gpu")]
use arc_swap::{ArcSwap, ArcSwapOption};
use nami::watcher::BoxWatcherGuard;
use nami::{Computed, Signal, SignalExt};
use waterui_core::layout::StretchAxis;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, Native, NativeView, Str, View, contract_violation};

use crate::draw::{Draw as _, Paint, Recorder, ShaderId, ShaderPaint, kurbo::Rect};
#[cfg(feature = "gpu")]
use crate::gpu::{Context, Frame, GpuContent, GpuView, RedrawHandle};
use crate::scene::resources::{RecordingResources, Registered, SceneResources};
use crate::scene_view::{
    NextFrame, SceneContent, SceneFrame, SceneInvalidator, SceneUpdate, invalidate_on_change,
};
use crate::source::ShaderSource;

/// The most `f32` uniforms a shader reads: `params` is sixteen `vec4`s.
const UNIFORM_CAPACITY: usize = 64;

/// The fragment's prelude: uniforms, `params` and the full-screen vertex stage.
#[cfg(feature = "gpu")]
const PRELUDE: &str = include_str!("shader_paint.wgsl");

/// A view painted by a WGSL fragment shader.
///
/// # Layout Behavior
///
/// Stretches on both axes; constrain it with `.frame()`.
pub struct ShaderPaintView {
    fragment: Cow<'static, str>,
    animated: Computed<bool>,
    uniforms: Computed<Vec<f32>>,
    label: Computed<Option<Str>>,
    value: Computed<Option<Str>>,
}

impl fmt::Debug for ShaderPaintView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShaderPaintView").finish_non_exhaustive()
    }
}

impl ShaderPaintView {
    /// A static shader from its fragment body.
    #[must_use]
    pub fn new(fragment: impl Into<Cow<'static, str>>) -> Self {
        Self {
            fragment: fragment.into(),
            animated: nami::constant(false).computed(),
            uniforms: nami::constant(Vec::new()).computed(),
            label: Computed::constant(None),
            value: Computed::constant(None),
        }
    }

    /// Whether the shader re-renders every frame so `uniforms.time` advances,
    /// following a signal.
    #[must_use]
    pub fn animated(mut self, animated: impl IntoComputed<bool>) -> Self {
        self.animated = animated.into_computed();
        self
    }

    /// The user uniforms the fragment reads as `params`, following a signal.
    ///
    /// At most 64 floats; a longer list is a contract violation and the extra
    /// floats are dropped.
    #[must_use]
    pub fn uniforms(mut self, uniforms: impl IntoComputed<Vec<f32>>) -> Self {
        self.uniforms = uniforms.into_computed();
        self
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
            animated: self.animated,
            uniforms: self.uniforms,
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
    pub fn into_gpu_view(self) -> GpuView<Box<dyn GpuContent>> {
        let shared = Arc::new(Shared::default());
        let posting = Posting::new(self.animated, self.uniforms, Arc::clone(&shared));
        GpuView::new(ShaderContent {
            fragment: self.fragment,
            shared,
            gpu: None,
        })
        // The hook also keeps the posting's watchers alive with the view.
        .on_frame(move || posting.post())
        .offered_label(self.label)
        .offered_value(self.value)
        .into_boxed()
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

fn clamp_uniforms(raw: &[f32]) -> &[f32] {
    if raw.len() > UNIFORM_CAPACITY {
        contract_violation!(
            "shader paint accepts at most {UNIFORM_CAPACITY} uniform floats, got {}; the extra floats are dropped",
            raw.len()
        );
    }
    &raw[..raw.len().min(UNIFORM_CAPACITY)]
}

struct RecordedState {
    animated: bool,
    uniforms_bits: Vec<u32>,
    width_bits: u32,
    height_bits: u32,
}

impl RecordedState {
    fn matches(&self, animated: bool, uniforms: &[f32], width: f32, height: f32) -> bool {
        self.animated == animated
            && self.width_bits == width.to_bits()
            && self.height_bits == height.to_bits()
            && self.uniforms_bits.len() == uniforms.len()
            && self
                .uniforms_bits
                .iter()
                .zip(uniforms)
                .all(|(&recorded, current)| recorded == current.to_bits())
    }
}

struct ShaderPaintScene {
    fragment: Cow<'static, str>,
    animated: Computed<bool>,
    uniforms: Computed<Vec<f32>>,
    label: Computed<Option<Str>>,
    value: Computed<Option<Str>>,
    watchers: Option<[BoxWatcherGuard; 4]>,
    registered: Option<(bool, Registered<ShaderId>)>,
    recorded: Option<RecordedState>,
}

impl SceneContent for ShaderPaintScene {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> NextFrame {
        let animated = self.animated.snapshot();
        let raw_uniforms = self.uniforms.snapshot();
        let kept = clamp_uniforms(&raw_uniforms);
        let uniforms_bits: Vec<u32> = kept.iter().map(|v| v.to_bits()).collect();
        let next = if animated {
            NextFrame::Perpetual
        } else {
            NextFrame::Idle
        };

        if self
            .registered
            .as_ref()
            .is_none_or(|(was_animated, _)| *was_animated != animated)
        {
            let source = ShaderSource::wgsl(self.fragment.clone());
            let source = if animated { source.animated() } else { source };
            match resources.shader(source) {
                Ok(handle) => self.registered = Some((animated, handle)),
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
                    uniforms: kept.to_vec(),
                }),
            );
        }

        self.recorded = Some(RecordedState {
            animated,
            uniforms_bits,
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
        let animated = self.animated.snapshot();
        let raw_uniforms = self.uniforms.snapshot();
        let kept = &raw_uniforms[..raw_uniforms.len().min(UNIFORM_CAPACITY)];
        if recorded.matches(animated, kept, frame.width, frame.height)
            && raw_uniforms.len() <= UNIFORM_CAPACITY
        {
            SceneUpdate::Retained {
                next: if animated {
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
            [
                invalidate_on_change(&inv, &self.animated),
                invalidate_on_change(&inv, &self.uniforms),
                invalidate_on_change(&inv, &self.label),
                invalidate_on_change(&inv, &self.value),
            ]
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
    animated: bool,
    params: [f32; UNIFORM_CAPACITY],
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
                animated: false,
                params: [0.0; UNIFORM_CAPACITY],
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
    animated: Computed<bool>,
    uniforms: Computed<Vec<f32>>,
    shared: Arc<Shared>,
    _watchers: [BoxWatcherGuard; 2],
}

#[cfg(feature = "gpu")]
impl Posting {
    fn new(animated: Computed<bool>, uniforms: Computed<Vec<f32>>, shared: Arc<Shared>) -> Self {
        let wake_on = |signal: &dyn Fn(Arc<Shared>) -> BoxWatcherGuard| signal(Arc::clone(&shared));
        let posting = Self {
            _watchers: [
                wake_on(&|shared| alloc::boxed::Box::new(animated.watch(move |_| shared.wake()))),
                wake_on(&|shared| alloc::boxed::Box::new(uniforms.watch(move |_| shared.wake()))),
            ],
            animated,
            uniforms,
            shared,
        };
        posting.post();
        posting
    }

    fn post(&self) {
        let uniforms = self.uniforms.snapshot();
        let kept = clamp_uniforms(&uniforms);
        let mut params = [0.0; UNIFORM_CAPACITY];
        params[..kept.len()].copy_from_slice(kept);
        self.shared.posted.store(Arc::new(Posted {
            animated: self.animated.snapshot(),
            params,
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
impl GpuContent for ShaderContent {
    fn setup(&mut self, gpu: &Context<'_>) {
        self.shared
            .redraw
            .store(Some(Arc::new(gpu.redraw().clone())));
        let device = gpu.device();
        let globals_size = size_of_val(&[0.0_f32; 4]) as u64;
        let params_size = size_of_val(&[0.0_f32; UNIFORM_CAPACITY]) as u64;
        let module = cherenkov_gpu::interop::shader_module(
            device,
            "shader paint",
            &alloc::format!("{PRELUDE}\n{}", self.fragment),
        );
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
                entry_point: Some("main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.format(),
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
        self.gpu = Some(Program {
            pipeline,
            bind_group,
            globals,
            params,
        });
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        let gpu = self.gpu.as_ref().expect(
            "the engine runs `GpuContent::setup`, which installs `gpu`, before every render",
        );
        let posted = self.shared.posted.load();
        #[expect(clippy::cast_precision_loss, reason = "texture sides fit f32 exactly")]
        let globals = [
            frame.elapsed.as_secs_f32(),
            0.0,
            frame.width as f32,
            frame.height as f32,
        ];
        frame
            .queue
            .write_buffer(&gpu.globals, 0, bytemuck::cast_slice(&globals));
        frame
            .queue
            .write_buffer(&gpu.params, 0, bytemuck::cast_slice(&posted.params));
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shader paint"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shader paint"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.view,
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
        frame.queue.submit([encoder.finish()]);
        if posted.animated {
            frame.request_redraw();
        }
    }
}

/// A [`ShaderPaintView`] from a `.wgsl` file beside the calling source file.
#[macro_export]
macro_rules! shader {
    ($path:literal) => {
        $crate::shader_paint::ShaderPaintView::new(include_str!($path))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::rc::Rc;
    use core::cell::Cell;

    use nami::Binding;
    use waterui_core::AnyView;

    use crate::draw::{
        FontId, ImageData, ImageId, ImageLimits, ImageStorage, Instant, LayoutSize, ResourceError,
        Rgba8, Rgba16F,
    };
    use crate::scene::resources::{Handle, ResourceHandle, SceneBackend, ShaderBackend};
    use crate::source::FontSource;

    const FRAGMENT: &str = "@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return vec4<f32>(uv, 0.0, 1.0); }";

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
        let view = ShaderPaintView::new(FRAGMENT);
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
    fn scene_content_retains_static_recordings_and_invalidates_on_signal_changes() {
        let animated = Binding::container(false);
        let uniforms = Binding::container(vec![1.0_f32, 2.0]);
        let mut scene = ShaderPaintView::new(FRAGMENT)
            .animated(animated.clone())
            .uniforms(uniforms.clone())
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

        uniforms.set(vec![3.0]);
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

        animated.set(true);
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

        let animated = Binding::container(false);
        let uniforms = Binding::container(vec![1.0_f32, 2.0]);
        let shared = Arc::new(Shared::default());
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&wakes);
        shared
            .redraw
            .store(Some(Arc::new(RedrawHandle::new(move || {
                count.fetch_add(1, Ordering::Relaxed);
            }))));
        let posting = Posting::new(
            animated.computed(),
            uniforms.computed(),
            Arc::clone(&shared),
        );
        assert_eq!(&shared.posted.load().params[..3], &[1.0, 2.0, 0.0]);
        assert!(!shared.posted.load().animated);

        uniforms.set(vec![3.0]);
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        animated.set(true);
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
        posting.post();
        assert_eq!(&shared.posted.load().params[..2], &[3.0, 0.0]);
        assert!(shared.posted.load().animated);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "at most 64 uniform floats"))]
    fn too_many_uniforms_are_cut_to_the_capacity() {
        let mut uniforms = vec![1.0_f32; 65];
        uniforms[64] = 9.0;
        let kept = clamp_uniforms(&uniforms);
        assert_eq!(kept.len(), UNIFORM_CAPACITY);
        assert!(
            kept.iter()
                .all(|param| param.to_bits() == 1.0_f32.to_bits())
        );
    }
}
