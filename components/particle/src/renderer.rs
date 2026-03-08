//! GPU renderer for particle simulation and visualization.

use crate::{
    EmitterShape,
    config::{BlendMode, ParticleShape},
    gpu::{GpuParticle, Uniforms},
    shaders::{COMPUTE_SHADER, RENDER_SHADER},
};
use encase::{ShaderSize, UniformBuffer};
use std::borrow::Cow;
use waterui_graphics::{
    color::ResolvedColor,
    gpu_surface::{GpuContext, GpuFrame, GpuView},
    impl_gpu_subview, wgpu,
};

/// Resolved particle configuration ready for GPU.
#[derive(Clone, Debug)]
pub struct ResolvedParticleConfig {
    pub max_particles: u32,
    pub emitter_pos: [f32; 2],
    pub emitter_shape: EmitterShape,
    pub emit_rate: f32,
    pub gravity: [f32; 2],
    pub wind: [f32; 2],
    pub turbulence: f32,
    pub drag: f32,
    pub life_range: [f32; 2],
    pub speed_range: [f32; 2],
    pub angle_range: [f32; 2],
    pub size_range: [f32; 2],
    pub spin_range: [f32; 2],
    pub color_start: ResolvedColor,
    pub color_end: ResolvedColor,
    pub stretch_with_velocity: bool,
    pub blend_mode: BlendMode,
    pub softness: f32,
    pub shape: ParticleShape,
}

fn encode_emitter_size(shape: EmitterShape) -> glam::Vec2 {
    match shape {
        EmitterShape::Point => glam::Vec2::ZERO,
        EmitterShape::Rect { width, height } => glam::Vec2::new(width, height),
        EmitterShape::Circle { radius } => glam::Vec2::new(radius, -1.0),
    }
}

fn resolved_linear_color(color: ResolvedColor) -> glam::Vec4 {
    let [red, green, blue] = color.linear_with_headroom();
    glam::Vec4::new(red, green, blue, color.opacity)
}

const fn particle_shape_code(shape: ParticleShape) -> u32 {
    match shape {
        ParticleShape::Circle => 0,
        ParticleShape::Rect => 1,
    }
}

fn blend_state(blend_mode: BlendMode, hdr: bool) -> Option<wgpu::BlendState> {
    if hdr {
        None
    } else {
        Some(match blend_mode {
            BlendMode::Alpha => wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            BlendMode::Additive => wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            },
        })
    }
}

/// GPU renderer for particle systems.
pub struct ParticleRenderer {
    config: ResolvedParticleConfig,
    compute_pipeline: Option<wgpu::ComputePipeline>,
    render_pipeline: Option<wgpu::RenderPipeline>,
    particle_buffer: Option<wgpu::Buffer>,
    uniform_buffer: Option<wgpu::Buffer>,
    compute_bind_group: Option<wgpu::BindGroup>,
    render_bind_group: Option<wgpu::BindGroup>,
    start_time: std::time::Instant,
    last_frame_time: std::time::Instant,
}

impl ParticleRenderer {
    /// Create a new particle renderer with resolved configuration.
    pub fn new(config: ResolvedParticleConfig) -> Self {
        Self {
            config,
            compute_pipeline: None,
            render_pipeline: None,
            particle_buffer: None,
            uniform_buffer: None,
            compute_bind_group: None,
            render_bind_group: None,
            start_time: std::time::Instant::now(),
            last_frame_time: std::time::Instant::now(),
        }
    }

    fn update_uniforms(&mut self, queue: &wgpu::Queue, width: u32, height: u32) {
        if let Some(buffer) = &self.uniform_buffer {
            let now = std::time::Instant::now();
            let time = now.duration_since(self.start_time).as_secs_f32();
            let dt = now
                .duration_since(self.last_frame_time)
                .as_secs_f32()
                .min(0.1);
            self.last_frame_time = now;

            let uniforms = Uniforms {
                time,
                dt,
                seed: fastrand::u32(..),
                max_particles: self.config.max_particles,
                gravity: glam::Vec2::from_array(self.config.gravity),
                wind: glam::Vec2::from_array(self.config.wind),
                emitter_pos: glam::Vec2::from_array(self.config.emitter_pos),
                emitter_size: encode_emitter_size(self.config.emitter_shape),
                emit_rate: self.config.emit_rate,
                turbulence: self.config.turbulence,
                drag: self.config.drag,
                stretch_factor: if self.config.stretch_with_velocity {
                    1.0
                } else {
                    0.0
                },
                softness: self.config.softness,
                life_range: glam::Vec2::from_array(self.config.life_range),
                speed_range: glam::Vec2::from_array(self.config.speed_range),
                angle_range: glam::Vec2::from_array(self.config.angle_range),
                size_range: glam::Vec2::from_array(self.config.size_range),
                spin_range: glam::Vec2::from_array(self.config.spin_range),
                color_start: resolved_linear_color(self.config.color_start),
                color_end: resolved_linear_color(self.config.color_end),
                shape: particle_shape_code(self.config.shape),
                viewport_width: width,
                viewport_height: height,
            };

            let mut uniform_data = UniformBuffer::new(Vec::new());
            uniform_data
                .write(&uniforms)
                .expect("failed to write particle uniform buffer");
            queue.write_buffer(buffer, 0, uniform_data.as_ref());
        }
    }
}

impl GpuView for ParticleRenderer {
    async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        let device = ctx.device;
        let particle_size = <GpuParticle as ShaderSize>::SHADER_SIZE.get() as u64;
        let buffer_size = particle_size * u64::from(self.config.max_particles);

        let particle_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        particle_buffer.slice(..).get_mapped_range_mut().fill(0);
        particle_buffer.unmap();

        let uniform_size = <Uniforms as ShaderSize>::SHADER_SIZE.get() as u64;
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle Uniforms"),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Particle Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(COMPUTE_SHADER)),
        });
        let compute_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Particle Compute BGL"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Particle Compute PL"),
                bind_group_layouts: &[&compute_bind_group_layout],
                push_constant_ranges: &[],
            });
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Particle Compute Pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: ctx.pipeline_cache,
        });
        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Particle Compute BG"),
            layout: &compute_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: particle_buffer.as_entire_binding(),
                },
            ],
        });

        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Particle Render Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(RENDER_SHADER)),
        });
        let render_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Particle Render BGL"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Particle Render PL"),
                bind_group_layouts: &[&render_bind_group_layout],
                push_constant_ranges: &[],
            });
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Particle Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &render_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.surface_format,
                    blend: blend_state(self.config.blend_mode, ctx.is_hdr()),
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
            cache: ctx.pipeline_cache,
        });
        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Particle Render BG"),
            layout: &render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: particle_buffer.as_entire_binding(),
                },
            ],
        });

        self.particle_buffer = Some(particle_buffer);
        self.uniform_buffer = Some(uniform_buffer);
        self.compute_pipeline = Some(compute_pipeline);
        self.render_pipeline = Some(render_pipeline);
        self.compute_bind_group = Some(compute_bind_group);
        self.render_bind_group = Some(render_bind_group);

        let now = std::time::Instant::now();
        self.start_time = now;
        self.last_frame_time = now - std::time::Duration::from_secs_f32(1.0 / 60.0);
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        self.update_uniforms(frame.queue, frame.width, frame.height);

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Particle Encoder"),
            });

        if let (Some(pipeline), Some(bind_group)) =
            (&self.compute_pipeline, &self.compute_bind_group)
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Particle Compute Pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, bind_group, &[]);
            cpass.dispatch_workgroups(self.config.max_particles.div_ceil(64), 1, 1);
        }

        if let (Some(pipeline), Some(bind_group)) = (&self.render_pipeline, &self.render_bind_group)
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Particle Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
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
            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, bind_group, &[]);
            rpass.draw(0..6, 0..self.config.max_particles);
        }

        frame.queue.submit(std::iter::once(encoder.finish()));
        frame.request_redraw();
    }
}

impl_gpu_subview!(ParticleRenderer);

#[cfg(test)]
mod tests {
    use super::{blend_state, encode_emitter_size, resolved_linear_color};
    use crate::{
        EmitterShape,
        config::BlendMode,
        shaders::{COMPUTE_SHADER, RENDER_SHADER},
    };
    use waterui_graphics::{color::ResolvedColor, wgpu};

    #[test]
    fn alpha_mode_uses_premultiplied_blending() {
        assert_eq!(
            blend_state(BlendMode::Alpha, false),
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
        );
    }

    #[test]
    fn hdr_surfaces_do_not_force_blending() {
        assert_eq!(blend_state(BlendMode::Alpha, true), None);
    }

    #[test]
    fn circle_emitters_use_disk_encoding() {
        assert_eq!(
            encode_emitter_size(EmitterShape::Circle { radius: 0.25 }),
            glam::Vec2::new(0.25, -1.0)
        );
        assert_eq!(
            encode_emitter_size(EmitterShape::Rect {
                width: 0.4,
                height: 0.2,
            }),
            glam::Vec2::new(0.4, 0.2)
        );
    }

    #[test]
    fn resolved_colors_keep_hdr_headroom() {
        let color = ResolvedColor {
            red: 0.25,
            green: 0.5,
            blue: 0.75,
            headroom: 1.0,
            opacity: 0.4,
        };

        assert_eq!(
            resolved_linear_color(color),
            glam::Vec4::new(0.5, 1.0, 1.5, 0.4)
        );
    }

    #[test]
    fn compute_shader_contains_frame_rate_normalized_drag() {
        assert!(COMPUTE_SHADER.contains("pow(uniforms.drag, uniforms.dt * 60.0)"));
    }

    #[test]
    fn compute_shader_contains_disk_sampling() {
        assert!(COMPUTE_SHADER.contains("uniforms.emitter_size.y >= 0.0"));
        assert!(COMPUTE_SHADER.contains("sqrt(rand(seed)) * uniforms.emitter_size.x"));
    }

    #[test]
    fn render_shader_contains_local_aspect_correction() {
        assert!(RENDER_SHADER.contains("fn aspect_correct_offset"));
        assert!(
            RENDER_SHADER.contains("let world_pos = p.pos + aspect_correct_offset(local_offset);")
        );
    }
}
