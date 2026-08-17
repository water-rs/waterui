//! GPU renderer for particle simulation and visualization.

use crate::{
    EmitterShape,
    config::{BlendMode, ParticleConfig, ParticleShape},
    gpu::{CollisionUniforms, GpuCircleObstacle, GpuParticle, InteractionUniforms, Uniforms},
    shaders::{COMPUTE_SHADER, RENDER_SHADER},
};
use encase::{ShaderSize, StorageBuffer};
use num_traits::ToPrimitive;
use shaderloom::ShaderStage;
use std::cell::Cell;
use std::mem::offset_of;
use std::rc::Rc;
use waterui_core::{Computed, Environment, Signal, reactive::watcher::BoxWatcherGuard};
use waterui_graphics::{
    GpuContext, GpuFrame, GpuView,
    color::ResolvedColor,
    gpu_surface::RedrawHandle,
    reactive_color::ReactiveColor,
    shader_types::{ShaderVec2, ShaderVec4},
};

/// Resolved particle configuration ready for GPU.
#[derive(Clone, Debug, Default)]
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

struct ReactiveParticleConfig {
    max_particles: u32,
    config: ParticleConfig,
    color_start: ReactiveColor,
    color_end: ReactiveColor,
    watcher_guards: Vec<BoxWatcherGuard>,
    obstacles_dirty: Rc<Cell<bool>>,
}

impl ReactiveParticleConfig {
    fn new(max_particles: u32, config: ParticleConfig, env: &Environment) -> Self {
        Self {
            max_particles,
            color_start: ReactiveColor::new(&config.particle.color_start, env),
            color_end: ReactiveColor::new(&config.particle.color_end, env),
            config,
            watcher_guards: Vec::new(),
            obstacles_dirty: Rc::new(Cell::new(true)),
        }
    }

    fn get(&self) -> ResolvedParticleConfig {
        let mut resolved = ResolvedParticleConfig {
            max_particles: self.max_particles,
            collision_circle_obstacles: Vec::with_capacity(self.obstacle_count()),
            ..ResolvedParticleConfig::default()
        };
        self.update(&mut resolved);
        resolved
    }

    fn update(&self, resolved: &mut ResolvedParticleConfig) -> bool {
        resolved.emitter_pos = self.config.emitter.position.get();
        resolved.emitter_shape = self.config.emitter.shape.get();
        resolved.emit_rate = self.config.emitter.rate.get();
        resolved.gravity = self.config.environment.gravity.get();
        resolved.wind = self.config.environment.wind.get();
        resolved.turbulence = self.config.environment.turbulence.get();
        resolved.drag = self.config.environment.drag.get();
        resolved.collision_enabled = self.config.collision.enabled;
        resolved.collision_bounds = self.config.collision.bounds.get();
        resolved.collision_restitution = self.config.collision.restitution.get();
        resolved.collision_surface_friction = self.config.collision.surface_friction.get();
        let obstacles_changed = self.obstacles_dirty.replace(false);
        if obstacles_changed {
            resolved.collision_circle_obstacles.clear();
            resolved.collision_circle_obstacles.extend(
                self.config
                    .collision
                    .circle_obstacles
                    .iter()
                    .map(|obstacle| obstacle.value.get()),
            );
        }
        resolved.interaction_enabled = self.config.interaction.enabled;
        resolved.interaction_radius = self.config.interaction.radius.get();
        resolved.interaction_strength = self.config.interaction.strength.get();
        resolved.life_range = self.config.particle.life.get();
        resolved.speed_range = self.config.particle.speed.get();
        resolved.angle_range = self.config.particle.angle.get();
        resolved.size_range = self.config.particle.size.get();
        resolved.spin_range = self.config.particle.spin.get();
        resolved.color_start = self.color_start.get();
        resolved.color_end = self.color_end.get();
        resolved.stretch_with_velocity = self.config.particle.stretch_with_velocity;
        resolved.blend_mode = self.config.blend_mode;
        resolved.softness = self.config.particle.softness.get();
        resolved.shape = self.config.particle.shape;
        obstacles_changed
    }

    fn install(&mut self, redraw: &RedrawHandle) {
        self.color_start.install(redraw);
        self.color_end.install(redraw);
        self.watcher_guards = vec![
            redraw_on_change(&self.config.emitter.position, redraw),
            redraw_on_change(&self.config.emitter.shape, redraw),
            redraw_on_change(&self.config.emitter.rate, redraw),
            redraw_on_change(&self.config.environment.gravity, redraw),
            redraw_on_change(&self.config.environment.wind, redraw),
            redraw_on_change(&self.config.environment.turbulence, redraw),
            redraw_on_change(&self.config.environment.drag, redraw),
            redraw_on_change(&self.config.collision.bounds, redraw),
            redraw_on_change(&self.config.collision.restitution, redraw),
            redraw_on_change(&self.config.collision.surface_friction, redraw),
            redraw_on_change(&self.config.interaction.radius, redraw),
            redraw_on_change(&self.config.interaction.strength, redraw),
            redraw_on_change(&self.config.particle.life, redraw),
            redraw_on_change(&self.config.particle.speed, redraw),
            redraw_on_change(&self.config.particle.angle, redraw),
            redraw_on_change(&self.config.particle.size, redraw),
            redraw_on_change(&self.config.particle.spin, redraw),
            redraw_on_change(&self.config.particle.softness, redraw),
        ];
        self.watcher_guards.extend(
            self.config
                .collision
                .circle_obstacles
                .iter()
                .map(|obstacle| {
                    let redraw = redraw.clone();
                    let dirty = Rc::clone(&self.obstacles_dirty);
                    obstacle.value.watch(move |_| {
                        dirty.set(true);
                        redraw.request_redraw();
                    })
                }),
        );
    }

    const fn obstacle_count(&self) -> usize {
        self.config.collision.circle_obstacles.len()
    }

    const fn blend_mode(&self) -> BlendMode {
        self.config.blend_mode
    }
}

fn redraw_on_change<T: 'static>(signal: &Computed<T>, redraw: &RedrawHandle) -> BoxWatcherGuard {
    let redraw = redraw.clone();
    signal.watch(move |_| redraw.request_redraw())
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

#[expect(
    clippy::cast_possible_truncation,
    reason = "the statically defined particle uniform is far smaller than usize on supported targets"
)]
const PARTICLE_UNIFORM_SIZE: usize = <Uniforms as ShaderSize>::SHADER_SIZE.get() as usize;

const fn encode_emitter_size(shape: EmitterShape) -> ShaderVec2 {
    match shape {
        EmitterShape::Point => ShaderVec2::ZERO,
        EmitterShape::Rect { width, height } => ShaderVec2::new(width, height),
        EmitterShape::Circle { radius } => ShaderVec2::new(radius, -1.0),
    }
}

fn resolved_linear_color(color: ResolvedColor) -> ShaderVec4 {
    let [red, green, blue] = color.linear_with_headroom();
    ShaderVec4::new(red, green, blue, color.opacity)
}

fn write_obstacles(queue: &wgpu::Queue, buffer: &wgpu::Buffer, obstacles: &[[f32; 3]]) {
    let obstacle_data: Vec<_> = if obstacles.is_empty() {
        vec![GpuCircleObstacle::default()]
    } else {
        obstacles
            .iter()
            .map(|obstacle| {
                GpuCircleObstacle::new(ShaderVec2::new(obstacle[0], obstacle[1]), obstacle[2])
            })
            .collect()
    };
    let mut collision_data = StorageBuffer::new(Vec::new());
    collision_data
        .write(&obstacle_data)
        .expect("failed to encode particle collision obstacle buffer");
    queue.write_buffer(buffer, 0, collision_data.as_ref());
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

fn max_interaction_grid_cells(max_particles: u32) -> u32 {
    let dimension = f32_to_u32_ceil(u32_to_f32(max_particles).sqrt().max(1.0));
    dimension * dimension
}

const fn particle_shape_code(shape: ParticleShape) -> u32 {
    match shape {
        ParticleShape::Circle => 0,
        ParticleShape::Rect => 1,
    }
}

fn particle_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    let particle_size = u64::try_from(core::mem::size_of::<GpuParticle>())
        .expect("GpuParticle size must fit into wgpu's u64 buffer addressing");
    assert_eq!(
        particle_size,
        <GpuParticle as ShaderSize>::SHADER_SIZE.get(),
        "GpuParticle Rust layout must match shader storage stride"
    );

    wgpu::VertexBufferLayout {
        array_stride: particle_size,
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
    config: ReactiveParticleConfig,
    resolved_config: Option<ResolvedParticleConfig>,
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
    /// Creates a renderer from a resolved test fixture.
    #[cfg(test)]
    pub fn new(config: ResolvedParticleConfig) -> Self {
        use crate::config::CircleObstacleConfig;
        use waterui_graphics::color::Color;

        let mut reactive = ParticleConfig::default();
        reactive.emitter.position = Computed::constant(config.emitter_pos);
        reactive.emitter.shape = Computed::constant(config.emitter_shape);
        reactive.emitter.rate = Computed::constant(config.emit_rate);
        reactive.environment.gravity = Computed::constant(config.gravity);
        reactive.environment.wind = Computed::constant(config.wind);
        reactive.environment.turbulence = Computed::constant(config.turbulence);
        reactive.environment.drag = Computed::constant(config.drag);
        reactive.collision.enabled = config.collision_enabled;
        reactive.collision.bounds = Computed::constant(config.collision_bounds);
        reactive.collision.restitution = Computed::constant(config.collision_restitution);
        reactive.collision.surface_friction = Computed::constant(config.collision_surface_friction);
        reactive.collision.circle_obstacles = config
            .collision_circle_obstacles
            .into_iter()
            .map(|value| CircleObstacleConfig {
                value: Computed::constant(value),
            })
            .collect();
        reactive.interaction.enabled = config.interaction_enabled;
        reactive.interaction.radius = Computed::constant(config.interaction_radius);
        reactive.interaction.strength = Computed::constant(config.interaction_strength);
        reactive.particle.life = Computed::constant(config.life_range);
        reactive.particle.speed = Computed::constant(config.speed_range);
        reactive.particle.angle = Computed::constant(config.angle_range);
        reactive.particle.size = Computed::constant(config.size_range);
        reactive.particle.spin = Computed::constant(config.spin_range);
        reactive.particle.color_start = Computed::constant(Color::new(config.color_start));
        reactive.particle.color_end = Computed::constant(Color::new(config.color_end));
        reactive.particle.stretch_with_velocity = config.stretch_with_velocity;
        reactive.particle.softness = Computed::constant(config.softness);
        reactive.particle.shape = config.shape;
        reactive.blend_mode = config.blend_mode;

        Self::reactive(config.max_particles, reactive, &Environment::new())
    }

    pub(crate) fn reactive(max_particles: u32, config: ParticleConfig, env: &Environment) -> Self {
        Self::with_config(ReactiveParticleConfig::new(max_particles, config, env))
    }

    fn with_config(config: ReactiveParticleConfig) -> Self {
        Self {
            config,
            resolved_config: None,
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
        config: &ResolvedParticleConfig,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        elapsed: std::time::Duration,
        delta: std::time::Duration,
    ) {
        let buffer = self
            .uniform_buffer
            .as_ref()
            .expect("uniform buffer must exist before render");
        let time = elapsed.as_secs_f32();
        let dt = delta.as_secs_f32().min(0.1);
        let grid = interaction_grid(config);

        let uniforms = Uniforms {
            time,
            dt,
            seed: fastrand::u32(..),
            max_particles: config.max_particles,
            gravity: config.gravity.into(),
            wind: config.wind.into(),
            emitter_pos: config.emitter_pos.into(),
            emitter_size: encode_emitter_size(config.emitter_shape),
            emit_rate: config.emit_rate,
            turbulence: config.turbulence,
            drag: config.drag,
            stretch_factor: if config.stretch_with_velocity {
                1.0
            } else {
                0.0
            },
            softness: config.softness,
            interaction: InteractionUniforms::new(
                config.interaction_enabled,
                grid.width,
                grid.height,
                config.interaction_radius,
                config.interaction_strength,
            ),
            collision: CollisionUniforms::new(
                config.collision_enabled,
                config.collision_restitution,
                config.collision_surface_friction,
                config.collision_bounds.into(),
                u32::try_from(config.collision_circle_obstacles.len())
                    .expect("particle collision obstacle count must fit into u32"),
            ),
            life_range: config.life_range.into(),
            speed_range: config.speed_range.into(),
            angle_range: config.angle_range.into(),
            size_range: config.size_range.into(),
            spin_range: config.spin_range.into(),
            color_start: resolved_linear_color(config.color_start),
            color_end: resolved_linear_color(config.color_end),
            shape: particle_shape_code(config.shape),
            viewport_width: width,
            viewport_height: height,
        };

        let mut uniform_data = StorageBuffer::new([0; PARTICLE_UNIFORM_SIZE]);
        uniform_data
            .write(&uniforms)
            .expect("failed to write particle uniform buffer");
        queue.write_buffer(buffer, 0, uniform_data.as_ref());
    }

    fn encode_simulation_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source_index: usize,
        config: &ResolvedParticleConfig,
    ) -> usize {
        let target_index = 1 - source_index;
        let grid = interaction_grid(config);
        let bind_group = self.compute_bind_group(source_index);

        {
            let pipeline = self
                .clear_grid_pipeline
                .as_ref()
                .expect("clear-grid pipeline must exist before render");
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Particle Clear Grid Pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, bind_group, &[]);
            cpass.dispatch_workgroups(grid.cell_count.div_ceil(64), 1, 1);
        }

        {
            let pipeline = self
                .build_grid_pipeline
                .as_ref()
                .expect("build-grid pipeline must exist before render");
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Particle Build Grid Pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, bind_group, &[]);
            cpass.dispatch_workgroups(config.max_particles.div_ceil(64), 1, 1);
        }

        {
            let pipeline = self
                .simulate_pipeline
                .as_ref()
                .expect("simulation pipeline must exist before render");
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Particle Simulate Pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(pipeline);
            cpass.set_bind_group(0, bind_group, &[]);
            cpass.dispatch_workgroups(config.max_particles.div_ceil(64), 1, 1);
        }

        target_index
    }
}

fn fill_mapped_buffer(buffer: &wgpu::Buffer, value: u8) {
    let mut mapped = buffer.slice(..).get_mapped_range_mut();
    mapped.slice(..).fill(value);
    drop(mapped);
    buffer.unmap();
}

impl GpuView for ParticleRenderer {
    #[expect(
        clippy::too_many_lines,
        clippy::future_not_send,
        reason = "GPU setup is one render-thread resource graph whose local handles encode dependency order"
    )]
    async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        self.config.install(&ctx.redraw_handle);
        let config = self.config.get();
        let device = ctx.device;
        let particle_size = <GpuParticle as ShaderSize>::SHADER_SIZE.get();
        let buffer_size = particle_size * u64::from(config.max_particles);
        let collision_stride = <GpuCircleObstacle as ShaderSize>::SHADER_SIZE.get();
        let collision_count = self.config.obstacle_count().max(1);
        let u32_size = u64::try_from(core::mem::size_of::<u32>())
            .expect("u32 size must fit into wgpu's u64 buffer addressing");
        let grid_heads_size =
            u64::from(max_interaction_grid_cells(config.max_particles)) * u32_size;
        let particle_links_size = u64::from(config.max_particles) * u32_size;

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
            fill_mapped_buffer(&buffer, 0);
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
            size: collision_stride
                * u64::try_from(collision_count)
                    .expect("particle obstacle count must fit into wgpu buffer addressing"),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        write_obstacles(
            ctx.queue,
            &collision_buffer,
            &config.collision_circle_obstacles,
        );
        let grid_heads_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle Interaction Grid Heads"),
            size: grid_heads_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        fill_mapped_buffer(&grid_heads_buffer, 0xff);
        let particle_links_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Particle Interaction Links"),
            size: particle_links_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        fill_mapped_buffer(&particle_links_buffer, 0xff);

        let clear_grid_shader =
            COMPUTE_SHADER.create_entry_point(device, ShaderStage::Compute, "clear_grid");
        let build_grid_shader =
            COMPUTE_SHADER.create_entry_point(device, ShaderStage::Compute, "build_grid");
        let simulate_shader =
            COMPUTE_SHADER.create_entry_point(device, ShaderStage::Compute, "simulate_particles");
        let mut compute_bind_group_layouts = COMPUTE_SHADER.create_bind_group_layouts(device);
        assert_eq!(
            compute_bind_group_layouts.len(),
            1,
            "particle compute shader must use exactly one bind group"
        );
        let compute_bind_group_layout = compute_bind_group_layouts
            .pop()
            .expect("one particle compute bind group was asserted");
        let compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Particle Compute PL"),
                bind_group_layouts: &[Some(&compute_bind_group_layout)],
                immediate_size: 0,
            });
        let compute_pipeline_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let clear_grid_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Particle Clear Grid Pipeline"),
                layout: Some(&compute_pipeline_layout),
                module: clear_grid_shader.module(),
                entry_point: Some(clear_grid_shader.entry_point()),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        let build_grid_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Particle Build Grid Pipeline"),
                layout: Some(&compute_pipeline_layout),
                module: build_grid_shader.module(),
                entry_point: Some(build_grid_shader.entry_point()),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        let simulate_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Particle Simulate Pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: simulate_shader.module(),
            entry_point: Some(simulate_shader.entry_point()),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let compute_pipeline_error = compute_pipeline_scope.pop().await;
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

        let (vertex_shader, fragment_shader) =
            RENDER_SHADER.create_render_stages(device, "vs_main", "fs_main");
        let mut render_bind_group_layouts = RENDER_SHADER.create_bind_group_layouts(device);
        assert_eq!(
            render_bind_group_layouts.len(),
            1,
            "particle render shader must use exactly one bind group"
        );
        let render_bind_group_layout = render_bind_group_layouts
            .pop()
            .expect("one particle render bind group was asserted");
        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Particle Render PL"),
                bind_group_layouts: &[Some(&render_bind_group_layout)],
                immediate_size: 0,
            });
        let render_pipeline_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Particle Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: vertex_shader.module(),
                entry_point: Some(vertex_shader.entry_point()),
                buffers: &[particle_vertex_layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: fragment_shader.module(),
                entry_point: Some(fragment_shader.entry_point()),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.surface_format,
                    blend: Some(blend_state(self.config.blend_mode())),
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
        let render_pipeline_error = render_pipeline_scope.pop().await;
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
        self.resolved_config = Some(config);
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        let obstacles_changed = self.config.update(
            self.resolved_config
                .as_mut()
                .expect("particle render called before setup"),
        );
        let config = self
            .resolved_config
            .as_ref()
            .expect("particle render called before setup");
        if obstacles_changed {
            write_obstacles(
                frame.queue,
                self.collision_buffer
                    .as_ref()
                    .expect("collision buffer must exist before render"),
                &config.collision_circle_obstacles,
            );
        }
        self.update_uniforms(
            config,
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
            self.encode_simulation_passes(&mut encoder, self.current_particle_buffer_index, config);

        {
            let pipeline = self
                .render_pipeline
                .as_ref()
                .expect("render pipeline must exist before render");
            let bind_group = self
                .render_bind_group
                .as_ref()
                .expect("render bind group must exist before render");
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
                multiview_mask: None,
            });
            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, bind_group, &[]);
            rpass.set_vertex_buffer(0, self.particle_buffer(render_buffer_index).slice(..));
            rpass.draw(0..6, 0..config.max_particles);
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
        ParticleRenderer, ReactiveParticleConfig, ResolvedParticleConfig, blend_state,
        encode_emitter_size, resolved_linear_color,
    };
    use crate::{
        EmitterShape, ParticleShape,
        config::{BlendMode, ParticleConfig},
        gpu::{CollisionUniforms, GpuParticle, InteractionUniforms, Uniforms},
    };
    use encase::{ShaderSize, StorageBuffer};
    use waterui_core::{Binding, Environment, SignalExt};
    use waterui_graphics::{
        GpuContext, GpuRuntime, GpuView,
        color::ResolvedColor,
        gpu_surface::RedrawHandle,
        shader_types::{ShaderVec2, ShaderVec4},
    };

    fn test_gpu_runtime() -> GpuRuntime {
        pollster::block_on(GpuRuntime::new()).expect("particle GPU tests require a working runtime")
    }

    fn test_gpu_context(runtime: &GpuRuntime) -> GpuContext<'_> {
        let shared = runtime.context();
        GpuContext {
            adapter: &shared.adapter,
            device: shared.device.as_ref(),
            queue: shared.queue.as_ref(),
            surface_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            shader_cache: shared.shader_cache.as_ref(),
            scene_renderer: shared.scene_renderer(),
            msaa_samples: 1,
            redraw_handle: RedrawHandle::new(),
        }
    }

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

    fn simulate_and_read_particles(
        renderer: &ParticleRenderer,
        ctx: &GpuContext<'_>,
        particle_count: u32,
        label: &str,
    ) -> Vec<u8> {
        use std::sync::mpsc;

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        let config = renderer.config.get();
        let target_index = renderer.encode_simulation_passes(
            &mut encoder,
            renderer.current_particle_buffer_index,
            &config,
        );

        let buffer_size =
            <GpuParticle as ShaderSize>::SHADER_SIZE.get() * u64::from(particle_count);
        let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
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
        let bytes = mapped.to_vec();
        drop(mapped);
        readback.unmap();
        bytes
    }

    #[test]
    fn alpha_mode_uses_premultiplied_blending() {
        assert_eq!(
            blend_state(BlendMode::Alpha),
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
        );
    }

    #[test]
    fn circle_emitters_use_disk_encoding() {
        assert_eq!(
            encode_emitter_size(EmitterShape::Circle { radius: 0.25 }),
            ShaderVec2::new(0.25, -1.0)
        );
        assert_eq!(
            encode_emitter_size(EmitterShape::Rect {
                width: 0.4,
                height: 0.2,
            }),
            ShaderVec2::new(0.4, 0.2)
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
            ShaderVec4::new(0.5, 1.0, 1.5, 0.4)
        );
    }

    #[test]
    fn reactive_config_change_requests_redraw() {
        let rate = Binding::f32(100.0);
        let mut config = ParticleConfig::default();
        config.emitter.rate = rate.computed();
        let mut config = ReactiveParticleConfig::new(128, config, &Environment::new());
        let redraw = RedrawHandle::new();
        config.install(&redraw);

        rate.set(240.0);

        assert!(redraw.take_dirty());
        assert!((config.get().emit_rate - 240.0).abs() < f32::EPSILON);
    }

    #[test]
    fn render_pass_draws_when_particle_buffer_is_prefilled() {
        use waterui_graphics::{
            GpuFrame, GpuSurface, GpuView, OffscreenRenderConfig, OffscreenSize,
            gpu_surface::GpuContext,
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
                particle.pos = ShaderVec2::new(0.5, 0.5);
                particle.vel = ShaderVec2::ZERO;
                particle.life = 1.0;
                particle.max_life = 1.0;
                particle.size = 0.25;
                particle.rotation = 0.0;
                particle.rot_speed = 0.0;
                particle.color = ShaderVec4::ONE;

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

        let runtime = test_gpu_runtime();
        let size = OffscreenSize::try_from_pixels(256, 256).expect("test size must be valid");
        let config = OffscreenRenderConfig::new(size);
        let mut env = waterui_core::Environment::new();
        let output = pollster::block_on(
            GpuSurface::new(renderer).render_offscreen(&runtime, config, &mut env),
        )
        .expect("prefilled particle offscreen render should succeed");

        assert_eq!(output.width, 256);
        assert_eq!(output.height, 256);
        assert_eq!(output.rgba8.len(), 256 * 256 * 4);
    }

    #[test]
    fn compute_pass_writes_live_particles_to_storage_buffer() {
        let mut renderer = ParticleRenderer::new(ResolvedParticleConfig {
            emit_rate: 1_000_000.0,
            color_end: ResolvedColor {
                opacity: 0.0,
                ..opaque_white()
            },
            ..particle_test_config(256)
        });

        let runtime = test_gpu_runtime();
        let ctx = test_gpu_context(&runtime);
        let mut env = waterui_core::Environment::new();
        pollster::block_on(renderer.setup(&ctx, &mut env));
        let config = renderer.config.get();
        renderer.update_uniforms(
            &config,
            ctx.queue,
            256,
            256,
            std::time::Duration::from_secs_f32(1.0 / 60.0),
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        );

        let mapped = simulate_and_read_particles(
            &renderer,
            &ctx,
            config.max_particles,
            "particle_compute_buffer_test_readback",
        );
        assert!(
            mapped.iter().any(|byte| *byte != 0),
            "compute pass should write non-zero particle data"
        );

        let stride = usize::try_from(<GpuParticle as ShaderSize>::SHADER_SIZE.get())
            .expect("GpuParticle shader size must fit usize");
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
    }

    #[test]
    fn compute_pass_applies_bounds_collision_on_gpu() {
        let mut renderer = ParticleRenderer::new(ResolvedParticleConfig {
            collision_enabled: true,
            collision_bounds: [0.0, 0.0, 1.0, 1.0],
            collision_restitution: 0.5,
            collision_surface_friction: 0.25,
            ..particle_test_config(1)
        });

        let runtime = test_gpu_runtime();
        let ctx = test_gpu_context(&runtime);
        let mut env = waterui_core::Environment::new();
        pollster::block_on(renderer.setup(&ctx, &mut env));

        let mut particle = GpuParticle::default();
        particle.pos = ShaderVec2::new(0.95, 0.5);
        particle.vel = ShaderVec2::new(2.0, 1.0);
        particle.life = 1.0;
        particle.max_life = 1.0;
        particle.size = 0.1;
        particle.rotation = 0.0;
        particle.rot_speed = 0.0;
        particle.color = ShaderVec4::ONE;

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
                ShaderVec4::new(0.0, 0.0, 1.0, 1.0),
                0,
            ),
            size_range: ShaderVec2::new(0.1, 0.1),
            color_start: ShaderVec4::ONE,
            color_end: ShaderVec4::ONE,
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

        let buffer_size = <GpuParticle as ShaderSize>::SHADER_SIZE.get();
        let mapped = simulate_and_read_particles(
            &renderer,
            &ctx,
            1,
            "particle_collision_buffer_test_readback",
        );
        let updated = bytemuck::pod_read_unaligned::<GpuParticle>(
            &mapped[..usize::try_from(buffer_size).expect("particle buffer size must fit usize")],
        );
        assert!(
            (updated.pos.x() - 0.9).abs() < 0.0001,
            "particle should clamp against the right wall, got {}",
            updated.pos.x()
        );
        assert!(
            (updated.vel.x() + 1.0).abs() < 0.0001,
            "particle should bounce on the x axis, got {}",
            updated.vel.x()
        );
        assert!(
            (updated.vel.y() - 0.25).abs() < 0.0001,
            "particle tangential velocity should be damped on collision, got {}",
            updated.vel.y()
        );
    }

    #[test]
    fn compute_pass_applies_circle_obstacle_collision_on_gpu() {
        let mut renderer = ParticleRenderer::new(ResolvedParticleConfig {
            collision_circle_obstacles: vec![[0.3, 0.4, 0.08], [0.8, 0.5, 0.05]],
            collision_restitution: 0.5,
            collision_surface_friction: 0.25,
            ..particle_test_config(1)
        });

        let runtime = test_gpu_runtime();
        let ctx = test_gpu_context(&runtime);
        let mut env = waterui_core::Environment::new();
        pollster::block_on(renderer.setup(&ctx, &mut env));

        let mut particle = GpuParticle::default();
        particle.pos = ShaderVec2::new(0.86, 0.5);
        particle.vel = ShaderVec2::new(-1.0, 0.4);
        particle.life = 1.0;
        particle.max_life = 1.0;
        particle.size = 0.05;
        particle.color = ShaderVec4::ONE;

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
                ShaderVec4::new(0.0, 0.0, 1.0, 1.0),
                2,
            ),
            size_range: ShaderVec2::new(0.05, 0.05),
            color_start: ShaderVec4::ONE,
            color_end: ShaderVec4::ONE,
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

        let buffer_size = <GpuParticle as ShaderSize>::SHADER_SIZE.get();
        let mapped = simulate_and_read_particles(
            &renderer,
            &ctx,
            1,
            "particle_circle_collision_buffer_test_readback",
        );
        let updated = bytemuck::pod_read_unaligned::<GpuParticle>(
            &mapped[..usize::try_from(buffer_size).expect("particle buffer size must fit usize")],
        );
        assert!(
            (updated.pos.x() - 0.9).abs() < 0.0001,
            "particle should clamp against the second obstacle surface, got {}",
            updated.pos.x()
        );
        assert!(
            (updated.vel.x() - 0.5).abs() < 0.0001,
            "particle should bounce away from the obstacle, got {}",
            updated.vel.x()
        );
        assert!(
            (updated.vel.y() - 0.1).abs() < 0.0001,
            "particle tangential velocity should be damped against the obstacle, got {}",
            updated.vel.y()
        );
    }

    #[test]
    fn compute_pass_applies_particle_neighbor_interaction_on_gpu() {
        let mut renderer = ParticleRenderer::new(ResolvedParticleConfig {
            interaction_enabled: true,
            interaction_radius: 0.02,
            interaction_strength: 20.0,
            ..particle_test_config(2)
        });

        let runtime = test_gpu_runtime();
        let ctx = test_gpu_context(&runtime);
        let mut env = waterui_core::Environment::new();
        pollster::block_on(renderer.setup(&ctx, &mut env));

        let mut first = GpuParticle::default();
        first.pos = ShaderVec2::new(0.5, 0.5);
        first.life = 1.0;
        first.max_life = 1.0;
        first.size = 0.02;
        first.color = ShaderVec4::ONE;

        let mut second = GpuParticle::default();
        second.pos = ShaderVec2::new(0.53, 0.5);
        second.life = 1.0;
        second.max_life = 1.0;
        second.size = 0.02;
        second.color = ShaderVec4::ONE;

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
            size_range: ShaderVec2::new(0.02, 0.02),
            color_start: ShaderVec4::ONE,
            color_end: ShaderVec4::ONE,
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

        let mapped = simulate_and_read_particles(
            &renderer,
            &ctx,
            2,
            "particle_neighbor_interaction_test_readback",
        );
        let stride = usize::try_from(<GpuParticle as ShaderSize>::SHADER_SIZE.get())
            .expect("GpuParticle shader size must fit usize");
        let first_updated = bytemuck::pod_read_unaligned::<GpuParticle>(&mapped[..stride]);
        let second_updated =
            bytemuck::pod_read_unaligned::<GpuParticle>(&mapped[stride..(2 * stride)]);
        assert!(
            first_updated.vel.x() < 0.0,
            "first particle should be pushed left, got {}",
            first_updated.vel.x()
        );
        assert!(
            second_updated.vel.x() > 0.0,
            "second particle should be pushed right, got {}",
            second_updated.vel.x()
        );
    }
}
