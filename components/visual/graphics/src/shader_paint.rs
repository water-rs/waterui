//! A user shader as a view: a fragment shader drawn as [`GpuContent`] over
//! the whole view.
//!
//! The fragment is WGSL against a prelude — `uniforms.time`,
//! `uniforms.resolution`, a `uv` in `[0, 1]` (top-left origin) and
//! `@fragment fn main` — with user uniforms passed as a flat `f32` list that
//! follows a signal. Like every [`GpuContent`] it writes premultiplied linear
//! Display P3; the engine's colour helpers (`cherenkov_linear_srgb`,
//! `cherenkov_srgb`, ...) are available to convert from sRGB.
//!
//! Whether the shader animates follows a signal too: while it is `true` the
//! content asks for a frame after every frame so `uniforms.time` advances,
//! and while it is `false` the shader renders once per change of its
//! uniforms or size and the host stays idle.

extern crate alloc;

use alloc::borrow::Cow;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::mem::size_of_val;

use arc_swap::{ArcSwap, ArcSwapOption};
use nami::watcher::BoxWatcherGuard;
use nami::{Computed, Signal, SignalExt};
use waterui_core::layout::StretchAxis;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, View};

use crate::gpu::{Context, Frame, GpuContent, GpuContentView, RedrawHandle};

/// The most `f32` uniforms a shader reads: `params` is sixteen `vec4`s.
const UNIFORM_CAPACITY: usize = 64;

/// The fragment's prelude: uniforms, `params` and the full-screen vertex stage.
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
    /// At most 64 floats; a longer list panics when it is read.
    #[must_use]
    pub fn uniforms(mut self, uniforms: impl IntoComputed<Vec<f32>>) -> Self {
        self.uniforms = uniforms.into_computed();
        self
    }
}

impl View for ShaderPaintView {
    fn body(self, _env: &Environment) -> impl View {
        let shared = Arc::new(Shared::default());
        let posting = Posting::new(self.animated, self.uniforms, Arc::clone(&shared));
        GpuContentView::new(ShaderContent {
            fragment: self.fragment,
            shared,
            gpu: None,
        })
        // The hook also keeps the posting's watchers alive with the view.
        .on_frame(move || posting.post())
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// What the UI side last read from the signals.
struct Posted {
    animated: bool,
    params: [f32; UNIFORM_CAPACITY],
}

/// The UI side's mailbox to the render thread.
struct Shared {
    posted: ArcSwap<Posted>,
    /// The host's wake, known once the content is set up.
    redraw: ArcSwapOption<RedrawHandle>,
}

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

impl Shared {
    fn wake(&self) {
        if let Some(redraw) = &*self.redraw.load() {
            redraw.request_redraw();
        }
    }
}

/// The UI-thread half: reads the signals into the mailbox before each frame
/// and wakes an idle host when one changes.
struct Posting {
    animated: Computed<bool>,
    uniforms: Computed<Vec<f32>>,
    shared: Arc<Shared>,
    _watchers: [BoxWatcherGuard; 2],
}

impl Posting {
    fn new(animated: Computed<bool>, uniforms: Computed<Vec<f32>>, shared: Arc<Shared>) -> Self {
        let wake_on = |signal: &dyn Fn(Arc<Shared>) -> BoxWatcherGuard| signal(Arc::clone(&shared));
        let posting = Self {
            _watchers: [
                wake_on(&|shared| Box::new(animated.watch(move |_| shared.wake()))),
                wake_on(&|shared| Box::new(uniforms.watch(move |_| shared.wake()))),
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
        assert!(
            uniforms.len() <= UNIFORM_CAPACITY,
            "shader paint accepts at most {UNIFORM_CAPACITY} uniform floats, got {}",
            uniforms.len()
        );
        let mut params = [0.0; UNIFORM_CAPACITY];
        params[..uniforms.len()].copy_from_slice(&uniforms);
        self.shared.posted.store(Arc::new(Posted {
            animated: self.animated.snapshot(),
            params,
        }));
    }
}

/// The render-thread half: the fragment's pipeline on the engine's device.
struct ShaderContent {
    fragment: Cow<'static, str>,
    shared: Arc<Shared>,
    gpu: Option<Program>,
}

struct Program {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    params: wgpu::Buffer,
}

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

fn uniform_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

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
            &format!("{PRELUDE}\n{}", self.fragment),
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
        let gpu = self
            .gpu
            .as_ref()
            .expect("shader paint rendered before setup");
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
    use core::sync::atomic::{AtomicUsize, Ordering};

    use nami::Binding;
    use waterui_core::AnyView;

    const FRAGMENT: &str = "@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return vec4<f32>(uv, 0.0, 1.0); }";

    #[test]
    fn a_shader_view_is_a_stretching_gpu_content_view() {
        let view = ShaderPaintView::new(FRAGMENT);
        assert_eq!(view.stretch_axis(), StretchAxis::Both);
        let body = AnyView::new(view.body(&Environment::new()));
        assert!(body.downcast::<GpuContentView>().is_ok());
    }

    /// The mailbox follows the signals, and a change wakes an idle host.
    #[test]
    fn signals_reach_the_render_side_and_wake_the_host() {
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
    #[should_panic(expected = "at most 64 uniform floats")]
    fn too_many_uniforms_are_refused() {
        let _ = Posting::new(
            nami::constant(false).computed(),
            nami::constant(vec![0.0_f32; 65]).computed(),
            Arc::new(Shared::default()),
        );
    }
}
