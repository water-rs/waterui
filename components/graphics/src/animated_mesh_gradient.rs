//! GPU-animated mesh gradient with configurable parameters.

extern crate alloc;

use alloc::vec::Vec;

use encase::{ShaderSize, ShaderType, UniformBuffer};

use crate::color::ResolvedColor;
use crate::gpu_surface::{GpuContext, GpuFrame, GpuSurface, GpuView};
use crate::include_shader;
use waterui_core::View;

static ANIMATED_MESH_SHADER: crate::prewarm::PrewarmedShader =
    include_shader!("shaders/animated_mesh_gradient.wgsl");

/// Number of colors in the mesh palette (4x4 grid).
pub const ANIMATED_MESH_PALETTE_LEN: usize = 16;

/// Configuration for the animated mesh gradient.
#[derive(Debug, Clone)]
pub struct AnimatedMeshGradientConfig {
    /// Animation speed multiplier.
    pub speed: f32,
    /// UV warp strength (controls flow intensity).
    pub warp: f32,
    /// Mesh palette (4x4 grid, row-major).
    pub palette: [ResolvedColor; ANIMATED_MESH_PALETTE_LEN],
}

impl AnimatedMeshGradientConfig {
    /// Sets the animation speed (must be >= 0.0).
    #[must_use]
    pub fn speed(mut self, speed: f32) -> Self {
        assert!(speed >= 0.0, "AnimatedMeshGradient speed must be >= 0.0");
        self.speed = speed;
        self
    }

    /// Sets the warp strength (recommended 0.0..0.5).
    #[must_use]
    pub fn warp(mut self, warp: f32) -> Self {
        assert!(warp >= 0.0, "AnimatedMeshGradient warp must be >= 0.0");
        self.warp = warp;
        self
    }

    /// Sets the 4x4 palette (row-major).
    #[must_use]
    pub fn palette(mut self, palette: [ResolvedColor; ANIMATED_MESH_PALETTE_LEN]) -> Self {
        self.palette = palette;
        self
    }

    /// Aqua + lavender pastel palette with soft contrast.
    #[must_use]
    pub fn aqua_bloom() -> Self {
        Self {
            speed: 0.6,
            warp: 0.24,
            palette: [
                palette_color(0.60, 0.92, 0.98),
                palette_color(0.70, 0.90, 0.98),
                palette_color(0.78, 0.84, 0.96),
                palette_color(0.84, 0.92, 0.98),
                palette_color(0.38, 0.72, 0.92),
                palette_color(0.46, 0.64, 0.90),
                palette_color(0.82, 0.64, 0.92),
                palette_color(0.90, 0.74, 0.92),
                palette_color(0.30, 0.56, 0.86),
                palette_color(0.42, 0.52, 0.86),
                palette_color(0.78, 0.56, 0.86),
                palette_color(0.94, 0.70, 0.88),
                palette_color(0.22, 0.44, 0.78),
                palette_color(0.36, 0.46, 0.80),
                palette_color(0.62, 0.54, 0.80),
                palette_color(0.86, 0.66, 0.84),
            ],
        }
    }

    /// Soft pastel palette with gentle cyan/pink transitions.
    #[must_use]
    pub fn pastel_lagoon() -> Self {
        Self {
            speed: 0.55,
            warp: 0.16,
            palette: [
                palette_color(0.72, 0.94, 0.98),
                palette_color(0.62, 0.90, 0.98),
                palette_color(0.72, 0.82, 0.96),
                palette_color(0.88, 0.80, 0.96),
                palette_color(0.56, 0.86, 0.97),
                palette_color(0.64, 0.80, 0.96),
                palette_color(0.80, 0.74, 0.94),
                palette_color(0.96, 0.82, 0.92),
                palette_color(0.48, 0.80, 0.95),
                palette_color(0.60, 0.72, 0.94),
                palette_color(0.82, 0.70, 0.92),
                palette_color(0.96, 0.78, 0.90),
                palette_color(0.42, 0.72, 0.92),
                palette_color(0.54, 0.66, 0.92),
                palette_color(0.76, 0.64, 0.88),
                palette_color(0.92, 0.74, 0.86),
            ],
        }
    }

    /// Bright white base with blush lavender accents.
    #[must_use]
    pub fn soft_blush() -> Self {
        Self {
            speed: 0.45,
            warp: 0.12,
            palette: [
                palette_color(0.98, 0.98, 1.00),
                palette_color(0.96, 0.96, 0.99),
                palette_color(0.96, 0.94, 0.99),
                palette_color(0.98, 0.96, 0.98),
                palette_color(0.94, 0.92, 0.98),
                palette_color(0.92, 0.90, 0.97),
                palette_color(0.90, 0.88, 0.96),
                palette_color(0.94, 0.90, 0.96),
                palette_color(0.90, 0.86, 0.95),
                palette_color(0.88, 0.84, 0.94),
                palette_color(0.86, 0.82, 0.94),
                palette_color(0.90, 0.84, 0.94),
                palette_color(0.88, 0.80, 0.92),
                palette_color(0.86, 0.78, 0.92),
                palette_color(0.84, 0.76, 0.90),
                palette_color(0.88, 0.78, 0.90),
            ],
        }
    }

    /// Deep blue palette with soft cyan transitions.
    #[must_use]
    pub fn deep_blue() -> Self {
        Self {
            speed: 0.6,
            warp: 0.2,
            palette: [
                palette_color(0.02, 0.06, 0.18),
                palette_color(0.04, 0.12, 0.28),
                palette_color(0.06, 0.18, 0.36),
                palette_color(0.08, 0.24, 0.42),
                palette_color(0.06, 0.20, 0.38),
                palette_color(0.10, 0.28, 0.48),
                palette_color(0.12, 0.36, 0.56),
                palette_color(0.14, 0.42, 0.62),
                palette_color(0.08, 0.26, 0.44),
                palette_color(0.12, 0.34, 0.54),
                palette_color(0.16, 0.44, 0.66),
                palette_color(0.18, 0.52, 0.74),
                palette_color(0.06, 0.22, 0.40),
                palette_color(0.10, 0.30, 0.50),
                palette_color(0.14, 0.40, 0.64),
                palette_color(0.18, 0.50, 0.78),
            ],
        }
    }
}

fn palette_color(r: f32, g: f32, b: f32) -> ResolvedColor {
    ResolvedColor {
        red: r,
        green: g,
        blue: b,
        opacity: 1.0,
        headroom: 0.0,
    }
}

impl Default for AnimatedMeshGradientConfig {
    fn default() -> Self {
        Self::aqua_bloom()
    }
}

#[derive(Debug, Clone, Copy, ShaderType)]
struct AnimatedMeshUniforms {
    time: f32,
    speed: f32,
    warp: f32,
    _pad0: f32,
    resolution: glam::Vec2,
    _pad1: glam::Vec2,
    palette: [glam::Vec4; ANIMATED_MESH_PALETTE_LEN],
}

struct AnimatedMeshRenderer {
    config: AnimatedMeshGradientConfig,
    palette_gpu: [glam::Vec4; ANIMATED_MESH_PALETTE_LEN],
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    pipeline_format: Option<wgpu::TextureFormat>,
    start_time: std::time::Instant,
}

impl AnimatedMeshRenderer {
    fn new(config: AnimatedMeshGradientConfig) -> Self {
        let mut palette_gpu = [glam::Vec4::ZERO; ANIMATED_MESH_PALETTE_LEN];
        for (dst, src) in palette_gpu.iter_mut().zip(config.palette.iter()) {
            *dst = glam::Vec4::new(src.red, src.green, src.blue, src.opacity);
        }

        Self {
            config,
            palette_gpu,
            pipeline: None,
            uniform_buffer: None,
            bind_group: None,
            pipeline_format: None,
            start_time: std::time::Instant::now(),
        }
    }
}

impl GpuView for AnimatedMeshRenderer {
    async fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        _env: &mut waterui_core::Environment,
    ) {
        let shader = crate::shared_context::create_cached_shader_module_prewarmed(
            ctx.device,
            &ANIMATED_MESH_SHADER,
        );

        let uniform_size = <AnimatedMeshUniforms as ShaderSize>::SHADER_SIZE.get() as u64;
        let uniform_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Animated Mesh Gradient Uniforms"),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Animated Mesh Gradient Bind Group Layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Animated Mesh Gradient Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Animated Mesh Gradient Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let blend = if ctx.is_hdr() {
            None
        } else {
            Some(wgpu::BlendState::REPLACE)
        };

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Animated Mesh Gradient Pipeline"),
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
                multiview: None,
                cache: ctx.pipeline_cache,
            });

        self.pipeline = Some(pipeline);
        self.uniform_buffer = Some(uniform_buffer);
        self.bind_group = Some(bind_group);
        self.pipeline_format = Some(ctx.surface_format);
        self.start_time = std::time::Instant::now();

    }

    fn render(&mut self, frame: &mut GpuFrame) {
        if let Some(pipeline_fmt) = self.pipeline_format
            && pipeline_fmt != frame.format
        {
            self.pipeline = None;
            self.pipeline_format = None;
        }

        let Some(pipeline) = &self.pipeline else {
            return;
        };
        let Some(uniform_buffer) = &self.uniform_buffer else {
            return;
        };
        let Some(bind_group) = &self.bind_group else {
            return;
        };

        let elapsed = self.start_time.elapsed().as_secs_f32();
        let uniforms = AnimatedMeshUniforms {
            time: elapsed,
            speed: self.config.speed,
            warp: self.config.warp,
            _pad0: 0.0,
            resolution: glam::Vec2::new(frame.width as f32, frame.height as f32),
            _pad1: glam::Vec2::ZERO,
            palette: self.palette_gpu,
        };

        let mut uniform_data = UniformBuffer::new(Vec::new());
        uniform_data
            .write(&uniforms)
            .expect("Failed to write uniform buffer");
        frame
            .queue
            .write_buffer(uniform_buffer, 0, uniform_data.as_ref());

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Animated Mesh Gradient Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Animated Mesh Gradient Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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

        frame.queue.submit(core::iter::once(encoder.finish()));
        if self.config.speed > 0.0 {
            frame.request_redraw();
        }
    }
}

crate::impl_gpu_subview!(AnimatedMeshRenderer);

/// GPU-animated mesh gradient view.
pub struct AnimatedMeshGradient {
    inner: GpuSurface,
}

impl core::fmt::Debug for AnimatedMeshGradient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnimatedMeshGradient")
            .finish_non_exhaustive()
    }
}

impl AnimatedMeshGradient {
    /// Creates a new animated mesh gradient with the given configuration.
    #[must_use]
    pub fn new(config: AnimatedMeshGradientConfig) -> Self {
        Self {
            inner: GpuSurface::new(AnimatedMeshRenderer::new(config)),
        }
    }
}

impl Default for AnimatedMeshGradient {
    fn default() -> Self {
        Self::new(AnimatedMeshGradientConfig::default())
    }
}

impl View for AnimatedMeshGradient {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.inner
    }
}
