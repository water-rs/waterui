use crate::audio::{AudioCapture, SAMPLES_COUNT};
use crate::theme::WaveformTheme;
use encase::{ShaderSize, ShaderType, UniformBuffer};
use std::borrow::Cow;
use waterui_core::{
    Binding, IntoSignal, IntoSignalF32, Signal, binding, env::Environment, view::View,
};
use waterui_graphics::{GpuContext, GpuFrame, GpuSurface, GpuView, color::Color};

/// Resolved configuration for GPU rendering.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedWaveform {
    pub bg_color: [f32; 3],
    pub line_color: [f32; 3],
    pub glow_color: [f32; 3],
    pub line_width: f32,
    pub glow_intensity: f32,
    pub sensitivity: f64,
}

/// Uniform buffer struct for waveform visualizer.
/// Uses encase for automatic WGSL-compatible alignment.
#[derive(Copy, Clone, Debug, ShaderType)]
struct Uniforms {
    /// Viewport resolution [width, height].
    resolution: glam::Vec2,
    /// Elapsed time in seconds.
    time: f32,
    /// Sensitivity (amplitude multiplier).
    sensitivity: f32,
    /// Background color in linear RGB.
    bg_color: glam::Vec3,
    /// Line width in pixels.
    line_width: f32,
    /// Glow intensity (0.0 to 1.0).
    glow_intensity: f32,
    /// Line color in linear RGB.
    line_color: glam::Vec3,
    /// Glow color in linear RGB.
    glow_color: glam::Vec3,
}

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
#[derive(Clone)]
pub struct Waveform {
    theme: Binding<WaveformTheme>,
    sensitivity: Binding<f64>,
    capture: AudioCapture,
}

impl Waveform {
    /// Create a new Waveform visualizer.
    pub fn new(capture: AudioCapture) -> Self {
        Self {
            theme: binding(WaveformTheme::default()),
            sensitivity: binding(1.0),
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
    pub fn theme(self, theme: impl Into<Binding<WaveformTheme>>) -> Self {
        Self {
            theme: theme.into(),
            ..self
        }
    }

    /// Set the sensitivity (amplitude multiplier).
    pub fn sensitivity(self, sensitivity: impl Into<Binding<f64>>) -> Self {
        Self {
            sensitivity: sensitivity.into(),
            ..self
        }
    }

    /// Set the background color.
    pub fn bg_color(self, color: impl IntoSignal<Color> + 'static) -> Self {
        let color = color.into_signal().get();
        let mut theme = self.theme.get();
        theme.bg_color = color;
        self.theme.set(theme);
        self
    }

    /// Set the primary line color.
    pub fn line_color(self, color: impl IntoSignal<Color> + 'static) -> Self {
        let color = color.into_signal().get();
        let mut theme = self.theme.get();
        theme.line_color = color;
        self.theme.set(theme);
        self
    }

    /// Set the glow color.
    pub fn glow_color(self, color: impl IntoSignal<Color> + 'static) -> Self {
        let color = color.into_signal().get();
        let mut theme = self.theme.get();
        theme.glow_color = color;
        self.theme.set(theme);
        self
    }

    /// Set the line width.
    pub fn line_width(self, width: impl IntoSignalF32 + 'static) -> Self {
        let width = width.into_signal_f32().get();
        let mut theme = self.theme.get();
        theme.line_width = width;
        self.theme.set(theme);
        self
    }

    /// Set the glow intensity (0.0 to 1.0).
    pub fn glow(self, intensity: impl IntoSignalF32 + 'static) -> Self {
        let intensity = intensity.into_signal_f32().get();
        let mut theme = self.theme.get();
        theme.glow_intensity = intensity;
        self.theme.set(theme);
        self
    }
}

impl View for Waveform {
    fn body(self, env: &Environment) -> impl View {
        let theme = self.theme.get();
        let sensitivity = self.sensitivity.get();

        // Resolve colors
        let bg_rgb = theme.bg_color.resolve(env).get().linear_with_headroom();
        let line_rgb = theme.line_color.resolve(env).get().linear_with_headroom();
        let glow_rgb = theme.glow_color.resolve(env).get().linear_with_headroom();

        let resolved = ResolvedWaveform {
            bg_color: bg_rgb,
            line_color: line_rgb,
            glow_color: glow_rgb,
            line_width: theme.line_width,
            glow_intensity: theme.glow_intensity,
            sensitivity,
        };

        GpuSurface::new(WaveformRenderer::new(resolved, self.capture))
    }
}

// ----------------------------------------------------------------------------
// Internal Renderer
// ----------------------------------------------------------------------------

struct WaveformRenderer {
    config: ResolvedWaveform,
    capture: AudioCapture,
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    samples_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    start_time: std::time::Instant,
    smoothed_samples: Vec<f32>,
}

impl WaveformRenderer {
    fn new(config: ResolvedWaveform, capture: AudioCapture) -> Self {
        Self {
            config,
            capture,
            pipeline: None,
            uniform_buffer: None,
            samples_buffer: None,
            bind_group: None,
            start_time: std::time::Instant::now(),
            smoothed_samples: vec![0.0; SAMPLES_COUNT],
        }
    }
}

impl GpuView for WaveformRenderer {
    async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        let device = &ctx.device;

        // 1. Create Shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Waveform Visualizer Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("shader_waveform.wgsl"))),
        });

        // 2. Create Buffers using encase size calculation
        let uniform_size = <Uniforms as ShaderSize>::SHADER_SIZE.get() as u64;
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Waveform Uniforms"),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let samples_size = (SAMPLES_COUNT * std::mem::size_of::<f32>()) as u64;
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
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
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
                module: &shader,
                entry_point: Some("vs_main"), // Standard VS in shader
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.surface_format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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
        let (Some(pipeline), Some(bind_group), Some(uniform_buffer), Some(samples_buffer)) = (
            &self.pipeline,
            &self.bind_group,
            &self.uniform_buffer,
            &self.samples_buffer,
        ) else {
            return;
        };

        // 1. Update Audio Samples with Smoothing
        {
            if let Ok(audio_samples) = self.capture.samples.lock() {
                let smoothing_factor = 0.3;
                for (i, &sample) in audio_samples.iter().enumerate() {
                    if i < SAMPLES_COUNT {
                        self.smoothed_samples[i] +=
                            (sample - self.smoothed_samples[i]) * smoothing_factor;
                    }
                }
            }

            frame.queue.write_buffer(
                samples_buffer,
                0,
                bytemuck::cast_slice(&self.smoothed_samples),
            );
        }

        // 2. Update Uniforms using encase
        let time = self.start_time.elapsed().as_secs_f32();
        let uniforms = Uniforms {
            resolution: glam::Vec2::new(frame.width as f32, frame.height as f32),
            time,
            sensitivity: self.config.sensitivity as f32,
            bg_color: glam::Vec3::from_array(self.config.bg_color),
            line_width: self.config.line_width,
            glow_intensity: self.config.glow_intensity,
            line_color: glam::Vec3::from_array(self.config.line_color),
            glow_color: glam::Vec3::from_array(self.config.glow_color),
        };
        let mut uniform_data = UniformBuffer::new(Vec::new());
        uniform_data
            .write(&uniforms)
            .expect("Failed to write uniform buffer");
        frame
            .queue
            .write_buffer(uniform_buffer, 0, uniform_data.as_ref());

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
                            r: self.config.bg_color[0] as f64,
                            g: self.config.bg_color[1] as f64,
                            b: self.config.bg_color[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
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
#[must_use]
pub fn waveform(capture: AudioCapture) -> Waveform {
    Waveform::new(capture)
}
