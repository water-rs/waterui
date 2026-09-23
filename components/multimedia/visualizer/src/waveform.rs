use crate::audio::{AudioCapture, SAMPLES_COUNT};
use crate::theme::WaveformTheme;
use encase::{ShaderSize, UniformBuffer};
use num_traits::ToPrimitive;
use waterui_core::{
    Computed, IntoSignal, IntoSignalF32, Signal, SignalExt,
    env::Environment,
    reactive::{signal::IntoComputed, watcher::BoxWatcherGuard},
    view::View,
};
use waterui_graphics::{
    GpuContext, GpuFrame, GpuSurface, GpuView, color::Color, gpu_surface::RedrawHandle,
    reactive_color::ReactiveColor,
};

/// Resolved configuration for GPU rendering.
#[derive(Debug, Clone, Copy)]
struct ResolvedWaveform {
    bg_color: [f32; 3],
    line_color: [f32; 3],
    glow_color: [f32; 3],
    line_width: f32,
    glow_intensity: f32,
    sensitivity: f64,
}

mod shader_types {
    use encase::ShaderType;
    use waterui_graphics::shader_types::{ShaderVec2, ShaderVec3};

    /// Uniform buffer struct for waveform visualizer.
    /// Uses encase for automatic WGSL-compatible alignment.
    #[derive(Copy, Clone, Debug, ShaderType)]
    pub(super) struct Uniforms {
        /// Viewport resolution [width, height].
        pub(super) resolution: ShaderVec2,
        /// Elapsed time in seconds.
        pub(super) time: f32,
        /// Sensitivity (amplitude multiplier).
        pub(super) sensitivity: f32,
        /// Background color in linear RGB.
        pub(super) bg_color: ShaderVec3,
        /// Line width in pixels.
        pub(super) line_width: f32,
        /// Glow intensity (0.0 to 1.0).
        pub(super) glow_intensity: f32,
        /// Line color in linear RGB.
        pub(super) line_color: ShaderVec3,
        /// Glow color in linear RGB.
        pub(super) glow_color: ShaderVec3,
    }
}

use shader_types::Uniforms;
use waterui_graphics::shader_types::{ShaderVec2, ShaderVec3};

/// A real-time waveform oscilloscope visualizer.
///
/// # Example
///
/// ```rust
/// use waterui_core::Binding;
/// use waterui_graphics::color::Color;
/// use waterui_visualizer::{AudioCapture, Waveform};
///
/// let capture = AudioCapture::new();
/// let _waveform = Waveform::new(capture)
///     .line_color(Color::cyan())
///     .bg_color(Color::srgb(0, 0, 0))
///     .glow(0.8)
///     .sensitivity(Binding::f64(1.5));
/// ```
#[derive(Clone, Debug)]
#[must_use = "a `Waveform` does nothing unless it is rendered as part of a view"]
pub struct Waveform {
    theme: Computed<WaveformTheme>,
    sensitivity: Computed<f64>,
    bg_color: Option<Computed<Color>>,
    line_color: Option<Computed<Color>>,
    glow_color: Option<Computed<Color>>,
    line_width: Option<Computed<f32>>,
    glow_intensity: Option<Computed<f32>>,
    capture: AudioCapture,
}

impl Waveform {
    /// Create a new Waveform visualizer.
    pub fn new(capture: AudioCapture) -> Self {
        Self {
            theme: Computed::constant(WaveformTheme::default()),
            sensitivity: Computed::constant(1.0),
            bg_color: None,
            line_color: None,
            glow_color: None,
            line_width: None,
            glow_intensity: None,
            capture,
        }
    }

    /// Replace the audio capture source.
    ///
    /// Prefer constructing one `AudioCapture` and cloning it into multiple
    /// waveform views when you need to share microphone input.
    pub fn audio_capture(self, capture: AudioCapture) -> Self {
        Self { capture, ..self }
    }

    /// Set the visual theme.
    pub fn theme(mut self, theme: impl IntoComputed<WaveformTheme>) -> Self {
        self.theme = theme.into_computed();
        self
    }

    /// Set the sensitivity (amplitude multiplier).
    pub fn sensitivity(mut self, sensitivity: impl IntoComputed<f64>) -> Self {
        self.sensitivity = sensitivity.into_computed();
        self
    }

    /// Set the background color.
    pub fn bg_color(mut self, color: impl IntoSignal<Color> + 'static) -> Self {
        self.bg_color = Some(color.into_signal().computed());
        self
    }

    /// Set the primary line color.
    pub fn line_color(mut self, color: impl IntoSignal<Color> + 'static) -> Self {
        self.line_color = Some(color.into_signal().computed());
        self
    }

    /// Set the glow color.
    pub fn glow_color(mut self, color: impl IntoSignal<Color> + 'static) -> Self {
        self.glow_color = Some(color.into_signal().computed());
        self
    }

    /// Set the line width.
    pub fn line_width(mut self, width: impl IntoSignalF32 + 'static) -> Self {
        self.line_width = Some(width.into_signal_f32().computed());
        self
    }

    /// Set the glow intensity (0.0 to 1.0).
    pub fn glow(mut self, intensity: impl IntoSignalF32 + 'static) -> Self {
        self.glow_intensity = Some(intensity.into_signal_f32().computed());
        self
    }
}

impl View for Waveform {
    fn body(self, env: &Environment) -> impl View {
        let Self {
            theme,
            sensitivity,
            bg_color,
            line_color,
            glow_color,
            line_width,
            glow_intensity,
            capture,
        } = self;
        let config = ReactiveWaveform::new(
            &theme,
            sensitivity,
            bg_color,
            line_color,
            glow_color,
            line_width,
            glow_intensity,
            env,
        );
        GpuSurface::new(WaveformRenderer::new(config, capture))
    }
}

// ----------------------------------------------------------------------------
// Internal Renderer
// ----------------------------------------------------------------------------

struct ReactiveWaveform {
    bg_color: ReactiveColor,
    line_color: ReactiveColor,
    glow_color: ReactiveColor,
    line_width: Computed<f32>,
    glow_intensity: Computed<f32>,
    sensitivity: Computed<f64>,
    watcher_guards: Vec<BoxWatcherGuard>,
}

impl ReactiveWaveform {
    #[expect(
        clippy::too_many_arguments,
        reason = "the arguments are the complete waveform style signal set"
    )]
    fn new(
        theme: &Computed<WaveformTheme>,
        sensitivity: Computed<f64>,
        bg_color: Option<Computed<Color>>,
        line_color: Option<Computed<Color>>,
        glow_color: Option<Computed<Color>>,
        line_width: Option<Computed<f32>>,
        glow_intensity: Option<Computed<f32>>,
        env: &Environment,
    ) -> Self {
        let bg_color = bg_color.unwrap_or_else(|| theme.map(|value| value.bg_color).computed());
        let line_color =
            line_color.unwrap_or_else(|| theme.map(|value| value.line_color).computed());
        let glow_color =
            glow_color.unwrap_or_else(|| theme.map(|value| value.glow_color).computed());
        let line_width =
            line_width.unwrap_or_else(|| theme.map(|value| value.line_width).computed());
        let glow_intensity =
            glow_intensity.unwrap_or_else(|| theme.map(|value| value.glow_intensity).computed());

        Self {
            bg_color: ReactiveColor::new(&bg_color, env),
            line_color: ReactiveColor::new(&line_color, env),
            glow_color: ReactiveColor::new(&glow_color, env),
            line_width,
            glow_intensity,
            sensitivity,
            watcher_guards: Vec::new(),
        }
    }

    fn install(&mut self, redraw: &RedrawHandle) {
        self.bg_color.install(redraw);
        self.line_color.install(redraw);
        self.glow_color.install(redraw);
        self.watcher_guards = vec![
            redraw_on_change(&self.line_width, redraw),
            redraw_on_change(&self.glow_intensity, redraw),
            redraw_on_change(&self.sensitivity, redraw),
        ];
    }

    fn get(&self) -> ResolvedWaveform {
        ResolvedWaveform {
            bg_color: self.bg_color.get().linear_with_headroom(),
            line_color: self.line_color.get().linear_with_headroom(),
            glow_color: self.glow_color.get().linear_with_headroom(),
            line_width: self.line_width.get(),
            glow_intensity: self.glow_intensity.get(),
            sensitivity: self.sensitivity.get(),
        }
    }
}

fn redraw_on_change<T: 'static>(signal: &Computed<T>, redraw: &RedrawHandle) -> BoxWatcherGuard {
    let redraw = redraw.clone();
    signal.watch(move |_| redraw.request_redraw())
}

struct WaveformRenderer {
    config: ReactiveWaveform,
    capture: AudioCapture,
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    samples_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    smoothed_samples: Box<[f32; SAMPLES_COUNT]>,
    uniform_data: UniformBuffer<Vec<u8>>,
}

impl WaveformRenderer {
    fn new(config: ReactiveWaveform, capture: AudioCapture) -> Self {
        Self {
            config,
            capture,
            pipeline: None,
            uniform_buffer: None,
            samples_buffer: None,
            bind_group: None,
            smoothed_samples: Box::new([0.0; SAMPLES_COUNT]),
            uniform_data: UniformBuffer::new(Vec::new()),
        }
    }
}

impl GpuView for WaveformRenderer {
    #[expect(
        clippy::future_not_send,
        reason = "GpuView runs on the render thread; this future borrows the non-Send `GpuContext`/`Environment`"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "linear GPU pipeline and resource setup reads clearest as one sequence"
    )]
    async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        self.capture.activate();
        self.config.install(&ctx.redraw_handle);
        let device = ctx.device;

        let (vertex_shader, fragment_shader) =
            crate::WAVEFORM_SHADER.create_render_stages(device, "vs_main", "fs_main");

        // 2. Create Buffers using encase size calculation
        let uniform_size = <Uniforms as ShaderSize>::SHADER_SIZE.get();
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Waveform Uniforms"),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let samples_size = u64::try_from(SAMPLES_COUNT * std::mem::size_of::<f32>())
            .expect("waveform sample buffer size must fit into u64");
        let samples_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Audio Samples Buffer"),
            size: samples_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // 3. Create Bind Group Layout and Pipeline
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Waveform Bind Group Layout"),
            entries: &[
                // Uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Samples (Storage Buffer)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Waveform Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let blend = if ctx.is_hdr() {
            None
        } else {
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
        };

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Waveform Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: vertex_shader.module(),
                entry_point: Some(vertex_shader.entry_point()),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: fragment_shader.module(),
                entry_point: Some(fragment_shader.entry_point()),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.surface_format,
                    blend,
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
            multiview_mask: None,
            cache: None,
        });

        // 4. Create Bind Group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Waveform Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: samples_buffer.as_entire_binding(),
                },
            ],
        });

        self.pipeline = Some(render_pipeline);
        self.uniform_buffer = Some(uniform_buffer);
        self.samples_buffer = Some(samples_buffer);
        self.bind_group = Some(bind_group);
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        let pipeline = self
            .pipeline
            .as_ref()
            .expect("waveform pipeline must exist before render");
        let bind_group = self
            .bind_group
            .as_ref()
            .expect("waveform bind group must exist before render");
        let uniform_buffer = self
            .uniform_buffer
            .as_ref()
            .expect("waveform uniform buffer must exist before render");
        let samples_buffer = self
            .samples_buffer
            .as_ref()
            .expect("waveform samples buffer must exist before render");

        let config = self.config.get();

        // 1. Update Audio Samples with Smoothing
        {
            let audio_samples = self.capture.samples();
            let smoothing_factor = 0.3;
            for (smoothed, sample) in self.smoothed_samples.iter_mut().zip(audio_samples.iter()) {
                *smoothed += (*sample - *smoothed) * smoothing_factor;
            }
        }
        frame.queue.write_buffer(
            samples_buffer,
            0,
            bytemuck::cast_slice(self.smoothed_samples.as_slice()),
        );

        // 2. Update Uniforms using encase
        let uniforms = Uniforms {
            resolution: ShaderVec2::new(
                frame
                    .width
                    .to_f32()
                    .expect("waveform width must be representable as f32"),
                frame
                    .height
                    .to_f32()
                    .expect("waveform height must be representable as f32"),
            ),
            time: frame.elapsed().as_secs_f32(),
            sensitivity: config
                .sensitivity
                .to_f32()
                .expect("waveform sensitivity must be representable as f32"),
            bg_color: ShaderVec3::from_array(config.bg_color),
            line_width: config.line_width,
            glow_intensity: config.glow_intensity,
            line_color: ShaderVec3::from_array(config.line_color),
            glow_color: ShaderVec3::from_array(config.glow_color),
        };
        self.uniform_data.as_mut().clear();
        self.uniform_data
            .write(&uniforms)
            .expect("Failed to write uniform buffer");
        frame
            .queue
            .write_buffer(uniform_buffer, 0, self.uniform_data.as_ref());

        // 3. Render
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Waveform Visualizer Encoder"),
            });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Waveform Visualizer Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            // Use background color from config
                            r: f64::from(config.bg_color[0]),
                            g: f64::from(config.bg_color[1]),
                            b: f64::from(config.bg_color[2]),
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, bind_group, &[]);
            // Draw a single full-screen triangle. Vertex shader generates positions.
            rpass.draw(0..3, 0..1);
        }

        frame.queue.submit(std::iter::once(encoder.finish()));
        frame.request_redraw();
    }
}

/// Convenience constructor for [`Waveform`] from an [`AudioCapture`].
pub fn waveform(capture: AudioCapture) -> Waveform {
    Waveform::new(capture)
}

#[cfg(test)]
mod tests {
    use waterui_core::{Binding, Computed, Environment, SignalExt};
    use waterui_graphics::gpu_surface::RedrawHandle;

    use super::{ReactiveWaveform, WaveformTheme};

    #[test]
    fn style_signal_change_requests_redraw() {
        let width = Binding::f32(2.0);
        let mut config = ReactiveWaveform::new(
            &Computed::constant(WaveformTheme::default()),
            Computed::constant(1.0),
            None,
            None,
            None,
            Some(width.computed()),
            None,
            &Environment::new(),
        );
        let redraw = RedrawHandle::new();
        config.install(&redraw);

        width.set(4.0);

        assert!(redraw.take_dirty());
        assert!((config.get().line_width - 4.0).abs() < f32::EPSILON);
    }
}
