#![allow(
    clippy::cast_possible_truncation,
    reason = "intentional lossy numeric cast in rendering/layout code"
)]
#![allow(
    clippy::too_many_lines,
    clippy::significant_drop_tightening,
    reason = "GPU render methods are linear sequences that hold render-state guards for their full duration"
)]
//! GPU renderer for particle simulation and visualization.

use crate::{
    EmitterShape,
    config::{BlendMode, ParticleShape},
    gpu::{CollisionUniforms, GpuCircleObstacle, GpuParticle, InteractionUniforms, Uniforms},
    shaders::{COMPUTE_SHADER, RENDER_SHADER},
};
use encase::{ShaderSize, StorageBuffer};
use num_traits::ToPrimitive;
use std::borrow::Cow;
use std::mem::offset_of;
use waterui_graphics::{
    color::ResolvedColor,
    gpu_surface::{GpuContext, GpuFrame, GpuView},
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
    pub interaction_enabled: bool,
    pub interaction_radius: f32,
    pub interaction_strength: f32,
    pub collision_enabled: bool,
    pub collision_bounds: [f32; 4],
    pub collision_restitution: f32,
    pub collision_surface_friction: f32,
    pub collision_circle_obstacles: Vec<[f32; 3]>,
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

#[derive(Clone, Copy, Debug)]
struct InteractionGrid {
    width: u32,
    height: u32,
    cell_count: u32,
}

const PARTICLE_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 8] = [
    wgpu::VertexAttribute {
        offset: offset_of!(GpuParticle, pos) as u64,
        shader_location: 0,
        format: wgpu::VertexFormat::Float32x2,
    },
    wgpu::VertexAttribute {
        offset: offset_of!(GpuParticle, vel) as u64,
        shader_location: 1,
        format: wgpu::VertexFormat::Float32x2,
    },
    wgpu::VertexAttribute {
        offset: offset_of!(GpuParticle, life) as u64,
        shader_location: 2,
        format: wgpu::VertexFormat::Float32,
    },
    wgpu::VertexAttribute {
        offset: offset_of!(GpuParticle, max_life) as u64,
        shader_location: 3,
        format: wgpu::VertexFormat::Float32,
    },
    wgpu::VertexAttribute {
        offset: offset_of!(GpuParticle, size) as u64,
        shader_location: 4,
        format: wgpu::VertexFormat::Float32,
    },
    wgpu::VertexAttribute {
        offset: offset_of!(GpuParticle, rotation) as u64,
        shader_location: 5,
        format: wgpu::VertexFormat::Float32,
    },
    wgpu::VertexAttribute {
        offset: offset_of!(GpuParticle, rot_speed) as u64,
        shader_location: 6,
        format: wgpu::VertexFormat::Float32,
    },
    wgpu::VertexAttribute {
        offset: offset_of!(GpuParticle, color) as u64,
        shader_location: 7,
        format: wgpu::VertexFormat::Float32x4,
    },
];

const fn encode_emitter_size(shape: EmitterShape) -> glam::Vec2 {
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

fn interaction_grid(config: &ResolvedParticleConfig) -> InteractionGrid {
    if !config.interaction_enabled || config.interaction_radius <= 0.0 {
        return InteractionGrid {
            width: 1,
            height: 1,
            cell_count: 1,
        };
    }

    let ideal_dim = f32_to_u32_ceil(1.0 / config.interaction_radius);
    let max_dim = f32_to_u32_ceil(u32_to_f32(config.max_particles).sqrt().max(1.0));
    let dim = ideal_dim.clamp(1, max_dim);

    InteractionGrid {
        width: dim,
        height: dim,
        cell_count: dim * dim,
    }
}

const fn particle_shape_code(shape: ParticleShape) -> u32 {
    match shape {
        ParticleShape::Circle => 0,
        ParticleShape::Rect => 1,
    }
}

fn particle_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    assert_eq!(
        core::mem::size_of::<GpuParticle>() as u64,
        <GpuParticle as ShaderSize>::SHADER_SIZE.get(),
        "GpuParticle Rust layout must match shader storage stride"
    );

    wgpu::VertexBufferLayout {
        array_stride: core::mem::size_of::<GpuParticle>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &PARTICLE_VERTEX_ATTRIBUTES,
    }
}

const fn blend_state(blend_mode: BlendMode) -> wgpu::BlendState {
    match blend_mode {
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
    }
}

/// GPU renderer for particle systems.
pub struct ParticleRenderer {
    config: ResolvedParticleConfig,
    clear_grid_pipeline: Option<wgpu::ComputePipeline>,
    build_grid_pipeline: Option<wgpu::ComputePipeline>,
    simulate_pipeline: Option<wgpu::ComputePipeline>,
    render_pipeline: Option<wgpu::RenderPipeline>,
    particle_buffers: [Option<wgpu::Buffer>; 2],
    collision_buffer: Option<wgpu::Buffer>,
    grid_heads_buffer: Option<wgpu::Buffer>,
    particle_links_buffer: Option<wgpu::Buffer>,
    uniform_buffer: Option<wgpu::Buffer>,
    compute_bind_groups: [Option<wgpu::BindGroup>; 2],
    render_bind_group: Option<wgpu::BindGroup>,
    current_particle_buffer_index: usize,
}

impl ParticleRenderer {
    /// Create a new particle renderer with resolved configuration.
    pub fn new(config: ResolvedParticleConfig) -> Self {
        Self {
            config,
            clear_grid_pipeline: None,
            build_grid_pipeline: None,
            simulate_pipeline: None,
            render_pipeline: None,
            particle_buffers: std::array::from_fn(|_| None),
            collision_buffer: None,
            grid_heads_buffer: None,
            particle_links_buffer: None,
            uniform_buffer: None,
            compute_bind_groups: std::array::from_fn(|_| None),
            render_bind_group: None,
            current_particle_buffer_index: 0,
        }
    }

    const fn particle_buffer(&self, index: usize) -> &wgpu::Buffer {
        self.particle_buffers[index]
            .as_ref()
            .expect("particle buffer must exist before render")
    }

    const fn compute_bind_group(&self, index: usize) -> &wgpu::BindGroup {
        self.compute_bind_groups[index]
            .as_ref()
            .expect("compute bind group must exist before render")
    }

    fn update_uniforms(
        &self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        elapsed: std::time::Duration,
        delta: std::time::Duration,
    ) {
        if let Some(buffer) = &self.uniform_buffer {
            let time = elapsed.as_secs_f32();
            let dt = delta.as_secs_f32().min(0.1);
            let grid = interaction_grid(&self.config);

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
                interaction: InteractionUniforms::new(
                    self.config.interaction_enabled,
                    grid.width,
                    grid.height,
                    self.config.interaction_radius,
                    self.config.interaction_strength,
                ),
                collision: CollisionUniforms::new(
                    self.config.collision_enabled,
                    self.config.collision_restitution,
                    self.config.collision_surface_friction,
                    glam::Vec4::from_array(self.config.collision_bounds),
                    u32::try_from(self.config.collision_circle_obstacles.len())
                        .expect("particle collision obstacle count must fit into u32"),
                ),
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

            let mut uniform_data = StorageBuffer::new(Vec::new());
            uniform_data
                .write(&uniforms)
                .expect("failed to write particle uniform buffer");
            queue.write_buffer(buffer, 0, uniform_data.as_ref());
        }
    }

    fn encode_simulation_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source_index: usize,
    ) -> usize {
        let target_index = 1 - source_index;
        let grid = interaction_grid(&self.config);
        let bind_group = self.compute_bind_group(source_index);

        if let Some(pipeline) = &self.clear_grid_pipeline {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Particle Clear Grid Pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, bind_group, &[]);
            cpass.dispatch_workgroups(grid.cell_count.div_ceil(64), 1, 1);
        }

        if let Some(pipeline) = &self.build_grid_pipeline {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Particle Build Grid Pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, bind_group, &[]);
            cpass.dispatch_workgroups(self.config.max_particles.div_ceil(64), 1, 1);
        }

        if let Some(pipeline) = &self.simulate_pipeline {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Particle Simulate Pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, bind_group, &[]);
            cpass.dispatch_workgroups(self.config.max_particles.div_ceil(64), 1, 1);
        }

        target_index
    }
}

impl GpuView for ParticleRenderer {
    #[allow(clippy::too_many_lines)]
    fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        _env: &mut waterui_core::Environment,
    ) -> impl core::future::Future<Output = ()> {
        let device = ctx.device;
        let grid = interaction_grid(&self.config);
        let particle_size = <GpuParticle as ShaderSize>::SHADER_SIZE.get();
        let buffer_size = particle_size * u64::from(self.config.max_particles);
        let collision_stride = <GpuCircleObstacle as ShaderSize>::SHADER_SIZE.get();
        let collision_count = self.config.collision_circle_obstacles.len().max(1);
        let grid_heads_size = u64::from(grid.cell_count) * core::mem::size_of::<u32>() as u64;
        let particle_links_size =
            u64::from(self.config.max_particles) * core::mem::size_of::<u32>() as u64;

        let particle_buffers = std::array::from_fn(|index| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(match index {
                    0 => "Particle Buffer A",
                    _ => "Particle Buffer B",
                }),
                size: buffer_size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: true,
            });
            buffer.slice(..).get_mapped_range_mut().fill(0);
            buffer.unmap();
            Some(buffer)
        });

        let uniform_size = <Uniforms as ShaderSize>::SHADER_SIZE.get();
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle Uniforms"),
            size: uniform_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let collision_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle Collision Obstacles"),
            size: collision_stride * collision_count as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let obstacle_data = if self.config.collision_circle_obstacles.is_empty() {
            vec![GpuCircleObstacle::default()]
        } else {
            self.config
                .collision_circle_obstacles
                .iter()
                .map(|obstacle| {
                    GpuCircleObstacle::new(glam::Vec2::new(obstacle[0], obstacle[1]), obstacle[2])
                })
                .collect()
        };
        let mut collision_data = StorageBuffer::new(Vec::new());
        collision_data
            .write(&obstacle_data)
            .expect("failed to encode particle collision obstacle buffer");
        ctx.queue
            .write_buffer(&collision_buffer, 0, collision_data.as_ref());
        let grid_heads_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle Interaction Grid Heads"),
            size: grid_heads_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        grid_heads_buffer
            .slice(..)
            .get_mapped_range_mut()
            .fill(0xff);
        grid_heads_buffer.unmap();
        let particle_links_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle Interaction Links"),
            size: particle_links_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        particle_links_buffer
            .slice(..)
            .get_mapped_range_mut()
            .fill(0xff);
        particle_links_buffer.unmap();

        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Particle Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(COMPUTE_SHADER)),
        });
        let compute_shader_error = waterui_graphics::pop_error_scope_now(
            device,
            "particle::setup::compute_shader_error_scope",
        );
        assert!(
            compute_shader_error.is_none(),
            "particle compute shader creation failed: {compute_shader_error:?}"
        );
        let compute_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Particle Compute BGL"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
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
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let clear_grid_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Particle Clear Grid Pipeline"),
                layout: Some(&compute_pipeline_layout),
                module: &compute_shader,
                entry_point: Some("clear_grid"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: ctx.pipeline_cache,
            });
        let build_grid_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Particle Build Grid Pipeline"),
                layout: Some(&compute_pipeline_layout),
                module: &compute_shader,
                entry_point: Some("build_grid"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: ctx.pipeline_cache,
            });
        let simulate_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Particle Simulate Pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: Some("simulate_particles"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: ctx.pipeline_cache,
        });
        let compute_pipeline_error = waterui_graphics::pop_error_scope_now(
            device,
            "particle::setup::compute_pipeline_error_scope",
        );
        assert!(
            compute_pipeline_error.is_none(),
            "particle compute pipeline creation failed: {compute_pipeline_error:?}"
        );
        let compute_bind_groups = std::array::from_fn(|source_index| {
            let target_index = 1 - source_index;
            Some(
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(match source_index {
                        0 => "Particle Compute BG A->B",
                        _ => "Particle Compute BG B->A",
                    }),
                    layout: &compute_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: particle_buffers[source_index]
                                .as_ref()
                                .expect("source particle buffer must exist")
                                .as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: particle_buffers[target_index]
                                .as_ref()
                                .expect("target particle buffer must exist")
                                .as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: collision_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: grid_heads_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: particle_links_buffer.as_entire_binding(),
                        },
                    ],
                }),
            )
        });

        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Particle Render Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(RENDER_SHADER)),
        });
        let render_shader_error = waterui_graphics::pop_error_scope_now(
            device,
            "particle::setup::render_shader_error_scope",
        );
        assert!(
            render_shader_error.is_none(),
            "particle render shader creation failed: {render_shader_error:?}"
        );
        let render_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Particle Render BGL"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Particle Render PL"),
                bind_group_layouts: &[&render_bind_group_layout],
                push_constant_ranges: &[],
            });
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Particle Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &render_shader,
                entry_point: Some("vs_main"),
                buffers: &[particle_vertex_layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.surface_format,
                    blend: Some(blend_state(self.config.blend_mode)),
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
        let render_pipeline_error = waterui_graphics::pop_error_scope_now(
            device,
            "particle::setup::render_pipeline_error_scope",
        );
        assert!(
            render_pipeline_error.is_none(),
            "particle render pipeline creation failed: {render_pipeline_error:?}"
        );
        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Particle Render BG"),
            layout: &render_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        self.particle_buffers = particle_buffers;
        self.collision_buffer = Some(collision_buffer);
        self.grid_heads_buffer = Some(grid_heads_buffer);
        self.particle_links_buffer = Some(particle_links_buffer);
        self.uniform_buffer = Some(uniform_buffer);
        self.clear_grid_pipeline = Some(clear_grid_pipeline);
        self.build_grid_pipeline = Some(build_grid_pipeline);
        self.simulate_pipeline = Some(simulate_pipeline);
        self.render_pipeline = Some(render_pipeline);
        self.compute_bind_groups = compute_bind_groups;
        self.render_bind_group = Some(render_bind_group);
        core::future::ready(())
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        self.update_uniforms(
            frame.queue,
            frame.width,
            frame.height,
            frame.elapsed(),
            frame.delta(),
        );

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Particle Encoder"),
            });
        let render_buffer_index =
            self.encode_simulation_passes(&mut encoder, self.current_particle_buffer_index);

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
            rpass.set_vertex_buffer(0, self.particle_buffer(render_buffer_index).slice(..));
            rpass.draw(0..6, 0..self.config.max_particles);
        }

        frame.queue.submit(std::iter::once(encoder.finish()));
        self.current_particle_buffer_index = render_buffer_index;
        frame.request_redraw();
    }
}


fn f32_to_u32_ceil(value: f32) -> u32 {
    value
        .ceil()
        .to_u32()
        .expect("particle dimension must be representable as u32")
}

fn u32_to_f32(value: u32) -> f32 {
    value
        .to_f32()
        .expect("particle count must be representable as f32")
}

#[cfg(test)]
mod tests {
    use super::{
        ParticleRenderer, ResolvedParticleConfig, blend_state, encode_emitter_size,
        resolved_linear_color,
    };
    use crate::{
        EmitterShape, ParticleShape,
        config::BlendMode,
        gpu::{CollisionUniforms, GpuParticle, InteractionUniforms, Uniforms},
        shaders::{COMPUTE_SHADER, RENDER_SHADER},
    };
    use encase::{ShaderSize, StorageBuffer};
    use waterui_graphics::{GpuView, color::ResolvedColor};

    fn opaque_white() -> ResolvedColor {
        ResolvedColor {
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            headroom: 0.0,
            opacity: 1.0,
        }
    }

    fn particle_test_config(max_particles: u32) -> ResolvedParticleConfig {
        ResolvedParticleConfig {
            max_particles,
            emitter_pos: [0.5, 0.5],
            emitter_shape: EmitterShape::Point,
            emit_rate: 0.0,
            gravity: [0.0, 0.0],
            wind: [0.0, 0.0],
            turbulence: 0.0,
            drag: 1.0,
            interaction_enabled: false,
            interaction_radius: 0.0,
            interaction_strength: 0.0,
            collision_enabled: false,
            collision_bounds: [0.0, 0.0, 1.0, 1.0],
            collision_restitution: 1.0,
            collision_surface_friction: 1.0,
            collision_circle_obstacles: Vec::new(),
            life_range: [1.0, 1.0],
            speed_range: [0.0, 0.0],
            angle_range: [0.0, 0.0],
            size_range: [0.1, 0.1],
            spin_range: [0.0, 0.0],
            color_start: opaque_white(),
            color_end: opaque_white(),
            stretch_with_velocity: false,
            blend_mode: BlendMode::Alpha,
            softness: 0.0,
            shape: ParticleShape::Circle,
        }
    }

    #[test]
    fn alpha_mode_uses_premultiplied_blending() {
        assert_eq!(
            blend_state(BlendMode::Alpha),
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
        );
    }

    #[test]
    fn hdr_surfaces_keep_particle_blending() {
        assert_eq!(
            blend_state(BlendMode::Alpha),
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
        );
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
    fn compute_shader_contains_bounds_collision_response() {
        assert!(COMPUTE_SHADER.contains("fn apply_bounds_collision"));
        assert!(COMPUTE_SHADER.contains("uniforms.collision.enabled == 0u"));
        assert!(COMPUTE_SHADER.contains("uniforms.collision.surface_friction"));
    }

    #[test]
    fn compute_shader_contains_circle_obstacle_collision_response() {
        assert!(COMPUTE_SHADER.contains("fn apply_circle_obstacle_collision"));
        assert!(COMPUTE_SHADER.contains("fn apply_circle_obstacle_collisions"));
        assert!(COMPUTE_SHADER.contains("uniforms.collision.circle_obstacle_count"));
        assert!(COMPUTE_SHADER.contains("circle_obstacles[i]"));
    }

    #[test]
    fn compute_shader_contains_neighbor_grid_interaction() {
        assert!(COMPUTE_SHADER.contains("fn clear_grid"));
        assert!(COMPUTE_SHADER.contains("fn build_grid"));
        assert!(COMPUTE_SHADER.contains("fn simulate_particles"));
        assert!(COMPUTE_SHADER.contains("cell_heads"));
        assert!(COMPUTE_SHADER.contains("particle_links"));
        assert!(COMPUTE_SHADER.contains("apply_particle_neighbor_interaction"));
    }

    #[test]
    fn render_shader_contains_local_aspect_correction() {
        assert!(RENDER_SHADER.contains("fn aspect_correct_offset"));
        assert!(
            RENDER_SHADER.contains("let world_pos = pos + aspect_correct_offset(local_offset);")
        );
    }

    #[test]
    fn render_pass_draws_when_particle_buffer_is_prefilled() {
        use waterui_graphics::{
            GpuFrame, GpuSurface, GpuView, OffscreenRenderConfig, OffscreenSize,
            gpu_surface::GpuContext, shared_context,
        };

        struct PrefilledParticleRenderer {
            inner: ParticleRenderer,
        }

        impl GpuView for PrefilledParticleRenderer {
            #[expect(
                clippy::future_not_send,
                reason = "GpuView runs on the render thread; this future borrows the non-Send `GpuContext`/`Environment`"
            )]
            async fn setup(&mut self, ctx: &GpuContext<'_>, env: &mut waterui_core::Environment) {
                self.inner.setup(ctx, env).await;

                let mut particle = GpuParticle::default();
                particle.pos = glam::Vec2::new(0.5, 0.5);
                particle.vel = glam::Vec2::ZERO;
                particle.life = 1.0;
                particle.max_life = 1.0;
                particle.size = 0.25;
                particle.rotation = 0.0;
                particle.rot_speed = 0.0;
                particle.color = glam::Vec4::ONE;

                let mut particle_data = StorageBuffer::new(Vec::new());
                particle_data
                    .write(&vec![particle])
                    .expect("prefilled particle buffer encoding must succeed");
                ctx.queue
                    .write_buffer(self.inner.particle_buffer(0), 0, particle_data.as_ref());
            }

            fn render(&mut self, frame: &mut GpuFrame) {
                self.inner.render(frame);
            }
        }


        let renderer = PrefilledParticleRenderer {
            inner: ParticleRenderer::new(ResolvedParticleConfig {
                size_range: [0.25, 0.25],
                color_start: ResolvedColor {
                    red: 1.0,
                    green: 0.5,
                    blue: 0.0,
                    headroom: 0.0,
                    opacity: 1.0,
                },
                color_end: ResolvedColor {
                    red: 1.0,
                    green: 0.5,
                    blue: 0.0,
                    headroom: 0.0,
                    opacity: 1.0,
                },
                ..particle_test_config(1)
            }),
        };

        shared_context::init_shared_context().expect("shared gpu context must initialize");
        let size = OffscreenSize::try_from_pixels(256, 256).expect("test size must be valid");
        let config = OffscreenRenderConfig::new(size);
        let mut env = waterui_core::Environment::new();
        let output = GpuSurface::new(renderer)
            .render_offscreen(config, &mut env)
            .expect("prefilled particle offscreen render should succeed");

        assert_eq!(output.width, 256);
        assert_eq!(output.height, 256);
        assert_eq!(output.rgba8.len(), 256 * 256 * 4);
    }

    #[test]
    fn compute_pass_writes_live_particles_to_storage_buffer() {
        use std::sync::mpsc;
        use waterui_graphics::{GpuContext, gpu_surface::RedrawHandle, pollster, shared_context};

        let mut renderer = ParticleRenderer::new(ResolvedParticleConfig {
            emit_rate: 1_000_000.0,
            color_end: ResolvedColor {
                opacity: 0.0,
                ..opaque_white()
            },
            ..particle_test_config(256)
        });

        shared_context::init_shared_context().expect("shared gpu context must initialize");
        let shared = shared_context::shared_context();
        let guard = shared.read();
        let ctx = GpuContext {
            adapter: Some(&guard.adapter),
            device: guard.device.as_ref(),
            queue: guard.queue.as_ref(),
            surface_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            msaa_samples: 1,
            pipeline_cache: guard.pipeline_cache.as_ref(),
            redraw_handle: RedrawHandle::new(),
        };
        let mut env = waterui_core::Environment::new();
        pollster::block_on(renderer.setup(&ctx, &mut env));
        renderer.update_uniforms(
            ctx.queue,
            256,
            256,
            std::time::Duration::from_secs_f32(1.0 / 60.0),
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        );

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("particle_compute_buffer_test_encoder"),
            });
        let target_index =
            renderer.encode_simulation_passes(&mut encoder, renderer.current_particle_buffer_index);

        let buffer_size = <GpuParticle as ShaderSize>::SHADER_SIZE.get()
            * u64::from(renderer.config.max_particles);
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle_compute_buffer_test_readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(
            renderer.particle_buffer(target_index),
            0,
            &readback,
            0,
            buffer_size,
        );
        ctx.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .expect("particle compute readback callback dropped")
            .expect("particle compute readback mapping failed");

        let mapped = slice.get_mapped_range();
        assert!(
            mapped.iter().any(|byte| *byte != 0),
            "compute pass should write non-zero particle data"
        );

        let stride = <GpuParticle as ShaderSize>::SHADER_SIZE.get() as usize;
        let mut live_particles = 0usize;
        for chunk in mapped.chunks_exact(stride) {
            let life = f32::from_ne_bytes(chunk[16..20].try_into().expect("life bytes must exist"));
            let max_life =
                f32::from_ne_bytes(chunk[20..24].try_into().expect("max life bytes must exist"));
            let size = f32::from_ne_bytes(chunk[24..28].try_into().expect("size bytes must exist"));
            let pos_x = f32::from_ne_bytes(chunk[0..4].try_into().expect("pos x bytes must exist"));
            let pos_y = f32::from_ne_bytes(chunk[4..8].try_into().expect("pos y bytes must exist"));
            if life.is_finite() && max_life.is_finite() && size.is_finite() && life > 0.0 {
                assert!(max_life > 0.0, "live particle must have positive max_life");
                assert!(size > 0.0, "live particle must have positive size");
                assert!(
                    (0.0..=1.0).contains(&pos_x) && (0.0..=1.0).contains(&pos_y),
                    "live particle position should stay normalized, got ({pos_x}, {pos_y})"
                );
                live_particles += 1;
            }
        }
        assert!(
            live_particles > 0,
            "compute pass should spawn at least one live particle"
        );

        drop(mapped);
        readback.unmap();
    }

    #[test]
    fn compute_pass_applies_bounds_collision_on_gpu() {
        use std::sync::mpsc;
        use waterui_graphics::{GpuContext, gpu_surface::RedrawHandle, pollster, shared_context};

        let mut renderer = ParticleRenderer::new(ResolvedParticleConfig {
            collision_enabled: true,
            collision_bounds: [0.0, 0.0, 1.0, 1.0],
            collision_restitution: 0.5,
            collision_surface_friction: 0.25,
            ..particle_test_config(1)
        });

        shared_context::init_shared_context().expect("shared gpu context must initialize");
        let shared = shared_context::shared_context();
        let guard = shared.read();
        let ctx = GpuContext {
            adapter: Some(&guard.adapter),
            device: guard.device.as_ref(),
            queue: guard.queue.as_ref(),
            surface_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            msaa_samples: 1,
            pipeline_cache: guard.pipeline_cache.as_ref(),
            redraw_handle: RedrawHandle::new(),
        };
        let mut env = waterui_core::Environment::new();
        pollster::block_on(renderer.setup(&ctx, &mut env));

        let mut particle = GpuParticle::default();
        particle.pos = glam::Vec2::new(0.95, 0.5);
        particle.vel = glam::Vec2::new(2.0, 1.0);
        particle.life = 1.0;
        particle.max_life = 1.0;
        particle.size = 0.1;
        particle.rotation = 0.0;
        particle.rot_speed = 0.0;
        particle.color = glam::Vec4::ONE;

        let mut particle_data = StorageBuffer::new(Vec::new());
        particle_data
            .write(&vec![particle])
            .expect("prefilled collision particle encoding must succeed");
        ctx.queue
            .write_buffer(renderer.particle_buffer(0), 0, particle_data.as_ref());

        let uniforms = Uniforms {
            dt: 0.1,
            max_particles: 1,
            collision: CollisionUniforms::new(
                true,
                0.5,
                0.25,
                glam::Vec4::new(0.0, 0.0, 1.0, 1.0),
                0,
            ),
            size_range: glam::Vec2::new(0.1, 0.1),
            color_start: glam::Vec4::ONE,
            color_end: glam::Vec4::ONE,
            ..Uniforms::default()
        };
        let mut uniform_data = StorageBuffer::new(Vec::new());
        uniform_data
            .write(&uniforms)
            .expect("collision test uniform encoding must succeed");
        ctx.queue.write_buffer(
            renderer
                .uniform_buffer
                .as_ref()
                .expect("uniform buffer must exist after setup"),
            0,
            uniform_data.as_ref(),
        );

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("particle_collision_buffer_test_encoder"),
            });
        let target_index =
            renderer.encode_simulation_passes(&mut encoder, renderer.current_particle_buffer_index);

        let buffer_size = <GpuParticle as ShaderSize>::SHADER_SIZE.get();
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle_collision_buffer_test_readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(
            renderer.particle_buffer(target_index),
            0,
            &readback,
            0,
            buffer_size,
        );
        ctx.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .expect("particle collision readback callback dropped")
            .expect("particle collision readback mapping failed");

        let mapped = slice.get_mapped_range();
        let updated = bytemuck::pod_read_unaligned::<GpuParticle>(&mapped[..buffer_size as usize]);
        assert!(
            (updated.pos.x - 0.9).abs() < 0.0001,
            "particle should clamp against the right wall, got {}",
            updated.pos.x
        );
        assert!(
            (updated.vel.x + 1.0).abs() < 0.0001,
            "particle should bounce on the x axis, got {}",
            updated.vel.x
        );
        assert!(
            (updated.vel.y - 0.25).abs() < 0.0001,
            "particle tangential velocity should be damped on collision, got {}",
            updated.vel.y
        );

        drop(mapped);
        readback.unmap();
    }

    #[test]
    fn compute_pass_applies_circle_obstacle_collision_on_gpu() {
        use std::sync::mpsc;
        use waterui_graphics::{GpuContext, gpu_surface::RedrawHandle, pollster, shared_context};

        let mut renderer = ParticleRenderer::new(ResolvedParticleConfig {
            collision_circle_obstacles: vec![[0.3, 0.4, 0.08], [0.8, 0.5, 0.05]],
            collision_restitution: 0.5,
            collision_surface_friction: 0.25,
            ..particle_test_config(1)
        });

        shared_context::init_shared_context().expect("shared gpu context must initialize");
        let shared = shared_context::shared_context();
        let guard = shared.read();
        let ctx = GpuContext {
            adapter: Some(&guard.adapter),
            device: guard.device.as_ref(),
            queue: guard.queue.as_ref(),
            surface_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            msaa_samples: 1,
            pipeline_cache: guard.pipeline_cache.as_ref(),
            redraw_handle: RedrawHandle::new(),
        };
        let mut env = waterui_core::Environment::new();
        pollster::block_on(renderer.setup(&ctx, &mut env));

        let mut particle = GpuParticle::default();
        particle.pos = glam::Vec2::new(0.86, 0.5);
        particle.vel = glam::Vec2::new(-1.0, 0.4);
        particle.life = 1.0;
        particle.max_life = 1.0;
        particle.size = 0.05;
        particle.color = glam::Vec4::ONE;

        let mut particle_data = StorageBuffer::new(Vec::new());
        particle_data
            .write(&vec![particle])
            .expect("prefilled obstacle collision particle encoding must succeed");
        ctx.queue
            .write_buffer(renderer.particle_buffer(0), 0, particle_data.as_ref());

        let uniforms = Uniforms {
            dt: 0.0,
            max_particles: 1,
            collision: CollisionUniforms::new(
                false,
                0.5,
                0.25,
                glam::Vec4::new(0.0, 0.0, 1.0, 1.0),
                2,
            ),
            size_range: glam::Vec2::new(0.05, 0.05),
            color_start: glam::Vec4::ONE,
            color_end: glam::Vec4::ONE,
            ..Uniforms::default()
        };
        let mut uniform_data = StorageBuffer::new(Vec::new());
        uniform_data
            .write(&uniforms)
            .expect("obstacle collision test uniform encoding must succeed");
        ctx.queue.write_buffer(
            renderer
                .uniform_buffer
                .as_ref()
                .expect("uniform buffer must exist after setup"),
            0,
            uniform_data.as_ref(),
        );

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("particle_circle_collision_buffer_test_encoder"),
            });
        let target_index =
            renderer.encode_simulation_passes(&mut encoder, renderer.current_particle_buffer_index);

        let buffer_size = <GpuParticle as ShaderSize>::SHADER_SIZE.get();
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle_circle_collision_buffer_test_readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(
            renderer.particle_buffer(target_index),
            0,
            &readback,
            0,
            buffer_size,
        );
        ctx.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .expect("particle obstacle collision readback callback dropped")
            .expect("particle obstacle collision readback mapping failed");

        let mapped = slice.get_mapped_range();
        let updated = bytemuck::pod_read_unaligned::<GpuParticle>(&mapped[..buffer_size as usize]);
        assert!(
            (updated.pos.x - 0.9).abs() < 0.0001,
            "particle should clamp against the second obstacle surface, got {}",
            updated.pos.x
        );
        assert!(
            (updated.vel.x - 0.5).abs() < 0.0001,
            "particle should bounce away from the obstacle, got {}",
            updated.vel.x
        );
        assert!(
            (updated.vel.y - 0.1).abs() < 0.0001,
            "particle tangential velocity should be damped against the obstacle, got {}",
            updated.vel.y
        );

        drop(mapped);
        readback.unmap();
    }

    #[test]
    fn compute_pass_applies_particle_neighbor_interaction_on_gpu() {
        use std::sync::mpsc;
        use waterui_graphics::{GpuContext, gpu_surface::RedrawHandle, pollster, shared_context};

        let mut renderer = ParticleRenderer::new(ResolvedParticleConfig {
            interaction_enabled: true,
            interaction_radius: 0.02,
            interaction_strength: 20.0,
            ..particle_test_config(2)
        });

        shared_context::init_shared_context().expect("shared gpu context must initialize");
        let shared = shared_context::shared_context();
        let guard = shared.read();
        let ctx = GpuContext {
            adapter: Some(&guard.adapter),
            device: guard.device.as_ref(),
            queue: guard.queue.as_ref(),
            surface_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            msaa_samples: 1,
            pipeline_cache: guard.pipeline_cache.as_ref(),
            redraw_handle: RedrawHandle::new(),
        };
        let mut env = waterui_core::Environment::new();
        pollster::block_on(renderer.setup(&ctx, &mut env));

        let mut first = GpuParticle::default();
        first.pos = glam::Vec2::new(0.5, 0.5);
        first.life = 1.0;
        first.max_life = 1.0;
        first.size = 0.02;
        first.color = glam::Vec4::ONE;

        let mut second = GpuParticle::default();
        second.pos = glam::Vec2::new(0.53, 0.5);
        second.life = 1.0;
        second.max_life = 1.0;
        second.size = 0.02;
        second.color = glam::Vec4::ONE;

        let mut particle_data = StorageBuffer::new(Vec::new());
        particle_data
            .write(&vec![first, second])
            .expect("neighbor interaction particle encoding must succeed");
        ctx.queue
            .write_buffer(renderer.particle_buffer(0), 0, particle_data.as_ref());

        let uniforms = Uniforms {
            dt: 0.1,
            max_particles: 2,
            interaction: InteractionUniforms::new(true, 2, 2, 0.02, 20.0),
            size_range: glam::Vec2::new(0.02, 0.02),
            color_start: glam::Vec4::ONE,
            color_end: glam::Vec4::ONE,
            ..Uniforms::default()
        };
        let mut uniform_data = StorageBuffer::new(Vec::new());
        uniform_data
            .write(&uniforms)
            .expect("neighbor interaction uniform encoding must succeed");
        ctx.queue.write_buffer(
            renderer
                .uniform_buffer
                .as_ref()
                .expect("uniform buffer must exist after setup"),
            0,
            uniform_data.as_ref(),
        );

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("particle_neighbor_interaction_test_encoder"),
            });
        let target_index =
            renderer.encode_simulation_passes(&mut encoder, renderer.current_particle_buffer_index);

        let buffer_size = <GpuParticle as ShaderSize>::SHADER_SIZE.get() * 2;
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle_neighbor_interaction_test_readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(
            renderer.particle_buffer(target_index),
            0,
            &readback,
            0,
            buffer_size,
        );
        ctx.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .expect("particle neighbor interaction readback callback dropped")
            .expect("particle neighbor interaction readback mapping failed");

        let mapped = slice.get_mapped_range();
        let stride = <GpuParticle as ShaderSize>::SHADER_SIZE.get() as usize;
        let first_updated = bytemuck::pod_read_unaligned::<GpuParticle>(&mapped[..stride]);
        let second_updated =
            bytemuck::pod_read_unaligned::<GpuParticle>(&mapped[stride..(2 * stride)]);
        assert!(
            first_updated.vel.x < 0.0,
            "first particle should be pushed left, got {}",
            first_updated.vel.x
        );
        assert!(
            second_updated.vel.x > 0.0,
            "second particle should be pushed right, got {}",
            second_updated.vel.x
        );

        drop(mapped);
        readback.unmap();
    }
}
