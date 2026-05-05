//! GPU mask effect for filling QR dark modules with arbitrary GPU content.

use bytemuck::{Pod, Zeroable};
use core::fmt;
use wgpu::util::DeviceExt;

use crate::BarcodeSource;
use waterui_graphics::{
    EffectRenderer, ViewEffectContext, ViewEffectInput, ViewEffectOutput, color::ResolvedColor,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MaskUniforms {
    matrix_dim: u32,
    quiet_zone: u32,
    output_width: u32,
    output_height: u32,
    light_color: [f32; 4],
}

/// Applies a QR mask over an input texture:
/// - dark modules sample from input texture
/// - light modules output configured light color
pub struct BarcodeMaskEffect {
    source: BarcodeSource,
    pipeline: Option<wgpu::RenderPipeline>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    uniform_buffer: Option<wgpu::Buffer>,
    matrix_buffer: Option<wgpu::Buffer>,
    sampler: Option<wgpu::Sampler>,
    current_matrix_dim: u32,
    current_matrix_words: usize,
    light_color: [f32; 4],
}

impl fmt::Debug for BarcodeMaskEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarcodeMaskEffect")
            .field("current_matrix_dim", &self.current_matrix_dim)
            .finish_non_exhaustive()
    }
}

impl BarcodeMaskEffect {
    /// Creates a new mask effect from a barcode source and light module color.
    ///
    /// `light_color` is taken as a [`ResolvedColor`] because the
    /// `EffectRenderer` setup phase does not have access to an environment.
    /// Callers obtain the resolved color from `Color::resolve(env).get()` at
    /// view-body time.
    #[must_use]
    pub fn new(source: BarcodeSource, light_color: ResolvedColor) -> Self {
        Self {
            source,
            pipeline: None,
            bind_group_layout: None,
            uniform_buffer: None,
            matrix_buffer: None,
            sampler: None,
            current_matrix_dim: 0,
            current_matrix_words: 0,
            light_color: [
                light_color.red,
                light_color.green,
                light_color.blue,
                light_color.opacity,
            ],
        }
    }

    fn create_pipeline(
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("QR mask effect shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/qr_mask_effect.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("QR mask effect bind group layout"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("QR mask effect pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("QR mask effect pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            cache: pipeline_cache,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("QR mask effect sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        (pipeline, bind_group_layout, sampler)
    }

    fn ensure_uniform_buffer(&mut self, device: &wgpu::Device) {
        if self.uniform_buffer.is_some() {
            return;
        }
        self.uniform_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("QR mask effect uniforms"),
            size: core::mem::size_of::<MaskUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }

    fn ensure_matrix_buffer(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let matrix = self.source.matrix();
        let recreate = self.matrix_buffer.is_none()
            || self.current_matrix_dim != matrix.dimension
            || self.current_matrix_words != matrix.packed_data.len();

        if recreate {
            self.matrix_buffer = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("QR mask matrix buffer"),
                    contents: bytemuck::cast_slice(&matrix.packed_data),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                },
            ));
            self.current_matrix_dim = matrix.dimension;
            self.current_matrix_words = matrix.packed_data.len();
        } else if let Some(buffer) = &self.matrix_buffer {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&matrix.packed_data));
        }
    }
}

impl EffectRenderer for BarcodeMaskEffect {
    fn setup(&mut self, ctx: &ViewEffectContext) -> impl core::future::Future<Output = ()> {
        let (pipeline, bgl, sampler) =
            Self::create_pipeline(ctx.device, ctx.output_format, ctx.pipeline_cache);
        self.pipeline = Some(pipeline);
        self.bind_group_layout = Some(bgl);
        self.sampler = Some(sampler);
        self.ensure_uniform_buffer(ctx.device);

        async {}
    }

    fn render(&mut self, input: &ViewEffectInput, output: &ViewEffectOutput) {
        self.ensure_uniform_buffer(output.device);
        self.ensure_matrix_buffer(output.device, output.queue);

        let Some(pipeline) = &self.pipeline else {
            return;
        };
        let Some(bind_group_layout) = &self.bind_group_layout else {
            return;
        };
        let Some(sampler) = &self.sampler else {
            return;
        };
        let Some(uniform_buffer) = &self.uniform_buffer else {
            return;
        };
        let Some(matrix_buffer) = &self.matrix_buffer else {
            return;
        };

        let uniforms = MaskUniforms {
            matrix_dim: self.current_matrix_dim,
            quiet_zone: self.source.quiet_zone(),
            output_width: output.width,
            output_height: output.height,
            light_color: self.light_color,
        };
        output
            .queue
            .write_buffer(uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = output.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("QR mask effect bind group"),
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: matrix_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&input.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        let mut encoder = output
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("QR mask effect encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("QR mask effect pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.view,
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        output.queue.submit([encoder.finish()]);
    }
}
