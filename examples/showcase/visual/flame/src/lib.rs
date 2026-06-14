use std::time::Instant;

use waterui::app::App;
use waterui::graphics::{GpuContext, GpuFrame, GpuSurface, GpuView, bytemuck};
use waterui::prelude::*;
use waterui::preview;

#[preview]
fn main() -> impl View {
    vstack((
        text("Cinematic HDR Flame (GpuSurface)")
            .size(24)
            .foreground(Color::srgb(245, 247, 250)),
        text("HDR film buffer + bloom + ACES tonemap")
            .size(14)
            .foreground(Color::srgb(210, 216, 224)),
        GpuSurface::new(FlameRenderer::default()).size(400.0, 500.0),
        text("Rendered at 120fps")
            .size(12)
            .foreground(Color::srgb(210, 216, 224)),
    ))
    .background(Color::srgb(31, 35, 38))
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

const FILM_WGSL: &str = include_str!("shaders/film.wgsl");

struct FlameRenderer {
    last_tick: Instant,
    sim_time: f32,

    globals_buffer: Option<wgpu::Buffer>,
    globals_bind_group: Option<wgpu::BindGroup>,

    sample_layout: Option<wgpu::BindGroupLayout>,
    blur_layout: Option<wgpu::BindGroupLayout>,
    sampler: Option<wgpu::Sampler>,

    flame_pipeline: Option<wgpu::RenderPipeline>,
    downsample_pipeline: Option<wgpu::RenderPipeline>,
    blur_pipeline: Option<wgpu::RenderPipeline>,
    final_pipeline: Option<wgpu::RenderPipeline>,
    final_format: Option<wgpu::TextureFormat>,

    film_view: Option<wgpu::TextureView>,
    bloom_down_view: Option<wgpu::TextureView>,
    bloom_temp_view: Option<wgpu::TextureView>,
    bloom_blur_view: Option<wgpu::TextureView>,

    sample_bind_group: Option<wgpu::BindGroup>,
    final_bind_group: Option<wgpu::BindGroup>,
    blur_x_bind_group: Option<wgpu::BindGroup>,
    blur_y_bind_group: Option<wgpu::BindGroup>,

    size: (u32, u32),
}

impl Default for FlameRenderer {
    fn default() -> Self {
        Self {
            last_tick: Instant::now(),
            sim_time: 0.0,

            globals_buffer: None,
            globals_bind_group: None,

            sample_layout: None,
            blur_layout: None,
            sampler: None,

            flame_pipeline: None,
            downsample_pipeline: None,
            blur_pipeline: None,
            final_pipeline: None,
            final_format: None,

            film_view: None,
            bloom_down_view: None,
            bloom_temp_view: None,
            bloom_blur_view: None,

            sample_bind_group: None,
            final_bind_group: None,
            blur_x_bind_group: None,
            blur_y_bind_group: None,

            size: (0, 0),
        }
    }
}

impl FlameRenderer {
    const FILM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
    const GLOBALS_SIZE: u64 = std::mem::size_of::<[f32; 12]>() as u64;
    const BLUR_PARAMS_SIZE: u64 = std::mem::size_of::<[f32; 4]>() as u64;

    fn ensure_targets(&mut self, frame: &GpuFrame) {
        if self.size == (frame.width, frame.height)
            && self.film_view.is_some()
            && self.bloom_down_view.is_some()
            && self.bloom_temp_view.is_some()
            && self.bloom_blur_view.is_some()
            && self.sample_bind_group.is_some()
            && self.final_bind_group.is_some()
            && self.blur_x_bind_group.is_some()
            && self.blur_y_bind_group.is_some()
        {
            return;
        }

        self.size = (frame.width, frame.height);

        let bloom_w = (frame.width / 2).max(1);
        let bloom_h = (frame.height / 2).max(1);

        let film = frame.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Flame Film HDR"),
            size: wgpu::Extent3d {
                width: frame.width.max(1),
                height: frame.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FILM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let film_view = film.create_view(&wgpu::TextureViewDescriptor::default());

        let bloom_down = frame.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Flame Bloom Downsample"),
            size: wgpu::Extent3d {
                width: bloom_w,
                height: bloom_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FILM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bloom_down_view = bloom_down.create_view(&wgpu::TextureViewDescriptor::default());

        let bloom_temp = frame.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Flame Bloom Blur Temp"),
            size: wgpu::Extent3d {
                width: bloom_w,
                height: bloom_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FILM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bloom_temp_view = bloom_temp.create_view(&wgpu::TextureViewDescriptor::default());

        let bloom_blur = frame.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Flame Bloom Blurred"),
            size: wgpu::Extent3d {
                width: bloom_w,
                height: bloom_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FILM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bloom_blur_view = bloom_blur.create_view(&wgpu::TextureViewDescriptor::default());

        let Some(sample_layout) = &self.sample_layout else {
            return;
        };
        let Some(blur_layout) = &self.blur_layout else {
            return;
        };
        let Some(sampler) = &self.sampler else {
            return;
        };

        // Bind film + blurred bloom (bloom is ignored by the downsample pass).
        let sample_bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Sample Bind Group"),
            layout: sample_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&film_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    // Dummy: bind the film here so intermediate passes never sample the same
                    // texture they are writing to (wgpu exclusive COLOR_TARGET usage).
                    resource: wgpu::BindingResource::TextureView(&film_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        // Final composite needs the blurred bloom.
        let final_bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Final Bind Group"),
            layout: sample_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&film_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&bloom_blur_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        // Blur params (two bind groups with fixed directions).
        let blur_x_buffer = frame.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Flame Blur Params X"),
            size: Self::BLUR_PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blur_y_buffer = frame.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Flame Blur Params Y"),
            size: Self::BLUR_PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        #[allow(clippy::cast_precision_loss)]
        let texel_size = (1.0 / bloom_w as f32, 1.0 / bloom_h as f32);
        let blur_x: [f32; 4] = [texel_size.0, texel_size.1, 1.0, 0.0];
        let blur_y: [f32; 4] = [texel_size.0, texel_size.1, 0.0, 1.0];
        frame
            .queue
            .write_buffer(&blur_x_buffer, 0, bytemuck::bytes_of(&blur_x));
        frame
            .queue
            .write_buffer(&blur_y_buffer, 0, bytemuck::bytes_of(&blur_y));

        let blur_x_bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Blur X Bind Group"),
            layout: blur_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: blur_x_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&bloom_down_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        let blur_y_bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Blur Y Bind Group"),
            layout: blur_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: blur_y_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&bloom_temp_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        // Update stored views + bind groups.
        self.film_view = Some(film_view);
        self.bloom_down_view = Some(bloom_down_view);
        self.bloom_temp_view = Some(bloom_temp_view);
        self.bloom_blur_view = Some(bloom_blur_view);

        self.sample_bind_group = Some(sample_bind_group);
        self.final_bind_group = Some(final_bind_group);
        self.blur_x_bind_group = Some(blur_x_bind_group);
        self.blur_y_bind_group = Some(blur_y_bind_group);
    }
}

impl GpuView for FlameRenderer {
    async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui::Environment) {
        self.last_tick = Instant::now();
        self.sim_time = 0.0;

        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Flame Film Shader"),
                source: wgpu::ShaderSource::Wgsl(FILM_WGSL.into()),
            });

        let globals_size = Self::GLOBALS_SIZE;
        let globals_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Flame Globals"),
            size: globals_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Flame Globals Layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: core::num::NonZeroU64::new(globals_size),
                        },
                        count: None,
                    }],
                });

        let globals_bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Globals Bind Group"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Flame Linear Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let sample_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Flame Sample Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let blur_size = Self::BLUR_PARAMS_SIZE;
        let blur_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Flame Blur Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: core::num::NonZeroU64::new(blur_size),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let flame_pipeline_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Flame Pipeline Layout"),
                    bind_group_layouts: &[&globals_layout],
                    push_constant_ranges: &[],
                });

        let composite_pipeline_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Flame Composite Layout"),
                    bind_group_layouts: &[&globals_layout, &sample_layout],
                    push_constant_ranges: &[],
                });

        // Blur shader uses @group(2), so we must provide layouts for groups 0..=2.
        let blur_pipeline_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Flame Blur Pipeline Layout"),
                    bind_group_layouts: &[&globals_layout, &sample_layout, &blur_layout],
                    push_constant_ranges: &[],
                });

        let flame_pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Flame Pass Pipeline"),
                layout: Some(&flame_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_flame"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: Self::FILM_FORMAT,
                        blend: Some(wgpu::BlendState::REPLACE),
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

        let downsample_pipeline =
            ctx.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Flame Bloom Downsample Pipeline"),
                    layout: Some(&composite_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        buffers: &[],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_downsample"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: Self::FILM_FORMAT,
                            blend: Some(wgpu::BlendState::REPLACE),
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

        let blur_pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Flame Bloom Blur Pipeline"),
                layout: Some(&blur_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_blur"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: Self::FILM_FORMAT,
                        blend: Some(wgpu::BlendState::REPLACE),
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

        let final_pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Flame Final Pipeline"),
                layout: Some(&composite_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_final"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.surface_format,
                        blend: Some(wgpu::BlendState::REPLACE),
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

        self.globals_buffer = Some(globals_buffer);
        self.globals_bind_group = Some(globals_bind_group);

        self.sample_layout = Some(sample_layout);
        self.blur_layout = Some(blur_layout);
        self.sampler = Some(sampler);

        self.flame_pipeline = Some(flame_pipeline);
        self.downsample_pipeline = Some(downsample_pipeline);
        self.blur_pipeline = Some(blur_pipeline);
        self.final_pipeline = Some(final_pipeline);
        self.final_format = Some(ctx.surface_format);
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        if self.final_format != Some(frame.format) {
            // Surface format changed (unexpected) — force re-setup on next frame.
            self.flame_pipeline = None;
            self.downsample_pipeline = None;
            self.blur_pipeline = None;
            self.final_pipeline = None;
            self.final_format = None;
            return;
        }

        // Lazily create/recreate intermediate targets and bind groups.
        self.ensure_targets(frame);

        let Some(globals_buffer) = &self.globals_buffer else {
            return;
        };
        let Some(globals_bind_group) = &self.globals_bind_group else {
            return;
        };
        let Some(sample_bind_group) = &self.sample_bind_group else {
            return;
        };
        let Some(final_bind_group) = &self.final_bind_group else {
            return;
        };
        let Some(blur_x_bind_group) = &self.blur_x_bind_group else {
            return;
        };
        let Some(blur_y_bind_group) = &self.blur_y_bind_group else {
            return;
        };

        let Some(flame_pipeline) = &self.flame_pipeline else {
            return;
        };
        let Some(downsample_pipeline) = &self.downsample_pipeline else {
            return;
        };
        let Some(blur_pipeline) = &self.blur_pipeline else {
            return;
        };
        let Some(final_pipeline) = &self.final_pipeline else {
            return;
        };

        let Some(film_view) = &self.film_view else {
            return;
        };
        let Some(bloom_down_view) = &self.bloom_down_view else {
            return;
        };
        let Some(bloom_temp_view) = &self.bloom_temp_view else {
            return;
        };
        let Some(bloom_blur_view) = &self.bloom_blur_view else {
            return;
        };

        // Keep animation stable even if a frame stalls.
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_tick)
            .as_secs_f32()
            .min(1.0 / 30.0);
        self.last_tick = now;
        self.sim_time += dt;

        // Update globals.
        let elapsed = self.sim_time;
        let is_hdr = frame.is_hdr();
        #[allow(clippy::cast_precision_loss)]
        let (w, h) = (frame.width as f32, frame.height as f32);

        // HDR tuning: bigger highlight range + tighter bloom (to keep detail).
        let edr_gain = if is_hdr { 6.0 } else { 1.0 };
        let bloom_intensity = if is_hdr { 2.2 } else { 1.0 };
        let bloom_threshold = if is_hdr { 2.2 } else { 1.0 };
        let bloom_radius = if is_hdr { 2.2 } else { 1.6 };
        let flame_strength = if is_hdr { 2.20 } else { 1.0 };
        let wind = 0.12;

        let globals: [f32; 12] = [
            elapsed,
            1.15, // exposure
            bloom_threshold,
            bloom_intensity,
            edr_gain,
            bloom_radius,
            wind,
            flame_strength,
            w.max(1.0),
            h.max(1.0),
            1.0 / w.max(1.0),
            1.0 / h.max(1.0),
        ];
        frame
            .queue
            .write_buffer(globals_buffer, 0, bytemuck::bytes_of(&globals));

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Flame Film Encoder"),
            });

        // Pass 1: flame -> HDR film buffer.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Flame Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: film_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(flame_pipeline);
            pass.set_bind_group(0, globals_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        // Pass 2: threshold + downsample -> bloom_down.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Downsample Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: bloom_down_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(downsample_pipeline);
            pass.set_bind_group(0, globals_bind_group, &[]);
            pass.set_bind_group(1, sample_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        // Pass 3: blur X -> bloom_temp.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Blur X Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: bloom_temp_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(blur_pipeline);
            pass.set_bind_group(0, globals_bind_group, &[]);
            pass.set_bind_group(1, sample_bind_group, &[]);
            pass.set_bind_group(2, blur_x_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        // Pass 4: blur Y -> bloom_blur.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Blur Y Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: bloom_blur_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(blur_pipeline);
            pass.set_bind_group(0, globals_bind_group, &[]);
            pass.set_bind_group(1, sample_bind_group, &[]);
            pass.set_bind_group(2, blur_y_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        // Pass 5: composite + tonemap -> surface.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Final Composite Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(final_pipeline);
            pass.set_bind_group(0, globals_bind_group, &[]);
            pass.set_bind_group(1, final_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        frame.queue.submit(std::iter::once(encoder.finish()));
        frame.request_redraw();
    }
}
