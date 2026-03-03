//! GPU renderer for packed barcode matrices.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::{BarcodeSource, view::BarcodeFill};
use waterui_graphics::{GpuContext, GpuFrame, GpuView, color::Srgb, impl_gpu_subview};

/// Uniforms consumed by `qr_render.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct QrUniforms {
    matrix_dim: u32,
    quiet_zone: u32,
    output_width: u32,
    output_height: u32,
    fill_mode: u32,
    // Uniform buffer layout must match WGSL alignment rules:
    // after `fill_mode` WGSL inserts 12 bytes to align the next vec3,
    // then vec3 itself takes 12 bytes, so the next vec4 starts at byte 48.
    _pad0: [u32; 7],
    solid_dark_color: [f32; 4],
    light_color: [f32; 4],
    gradient_start_color: [f32; 4],
    gradient_end_color: [f32; 4],
    gradient_start_point: [f32; 2],
    gradient_end_point: [f32; 2],
}

/// A GPU renderer that displays a packed barcode matrix.
///
/// The QR matrix is generated on CPU and packed into a bit buffer once,
/// then rendered fully on GPU each frame by sampling that bit buffer
/// directly in a fragment shader.
pub struct BarcodeRenderer {
    source: BarcodeSource,
    render_pipeline: Option<wgpu::RenderPipeline>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    uniform_buffer: Option<wgpu::Buffer>,
    matrix_buffer: Option<wgpu::Buffer>,
    current_matrix_dim: u32,
    current_matrix_words: usize,
    fill_mode: u32,
    solid_dark_color: [f32; 4],
    light_color: [f32; 4],
    gradient_start_color: [f32; 4],
    gradient_end_color: [f32; 4],
    gradient_start_point: [f32; 2],
    gradient_end_point: [f32; 2],
}

impl core::fmt::Debug for BarcodeRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BarcodeRenderer")
            .field("source", &self.source)
            .field("current_matrix_dim", &self.current_matrix_dim)
            .finish()
    }
}

impl BarcodeRenderer {
    /// Creates a new renderer from a barcode source.
    #[must_use]
    pub fn new(source: BarcodeSource) -> Self {
        Self::new_with_fill(source, BarcodeFill::Solid(Srgb::BLACK), Srgb::WHITE)
    }

    /// Creates a new renderer with custom dark and light module colors.
    #[must_use]
    pub fn new_with_colors(source: BarcodeSource, dark: Srgb, light: Srgb) -> Self {
        Self::new_with_fill(source, BarcodeFill::Solid(dark), light)
    }

    /// Creates a new renderer with custom fill and light colors.
    #[must_use]
    pub fn new_with_fill(source: BarcodeSource, fill: BarcodeFill, light: Srgb) -> Self {
        let (
            fill_mode,
            solid_dark_color,
            gradient_start_color,
            gradient_end_color,
            gradient_start_point,
            gradient_end_point,
        ) = match fill {
            BarcodeFill::Solid(dark) => (
                0,
                Self::resolve_srgb(dark),
                [0.0; 4],
                [0.0; 4],
                [0.0, 0.0],
                [1.0, 1.0],
            ),
            BarcodeFill::LinearGradient {
                start_color,
                end_color,
                start_point,
                end_point,
            } => (
                1,
                [0.0; 4],
                Self::resolve_srgb(start_color),
                Self::resolve_srgb(end_color),
                start_point,
                end_point,
            ),
        };

        Self {
            source,
            render_pipeline: None,
            bind_group_layout: None,
            uniform_buffer: None,
            matrix_buffer: None,
            current_matrix_dim: 0,
            current_matrix_words: 0,
            fill_mode,
            solid_dark_color,
            light_color: Self::resolve_srgb(light),
            gradient_start_color,
            gradient_end_color,
            gradient_start_point,
            gradient_end_point,
        }
    }

    fn resolve_srgb(color: Srgb) -> [f32; 4] {
        let resolved = color.resolve();
        [
            resolved.red,
            resolved.green,
            resolved.blue,
            resolved.opacity,
        ]
    }

    fn create_render_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("QR render shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/qr_render.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("QR render bind group layout"),
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
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("QR render pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("QR render pipeline"),
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
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: pipeline_cache,
        });

        (pipeline, bind_group_layout)
    }

    fn ensure_matrix_buffer(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let matrix = self.source.matrix();
        let needs_recreate = self.matrix_buffer.is_none()
            || self.current_matrix_dim != matrix.dimension
            || self.current_matrix_words != matrix.packed_data.len();

        if needs_recreate {
            self.matrix_buffer = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("QR matrix storage buffer"),
                    contents: bytemuck::cast_slice(&matrix.packed_data),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                },
            ));
            self.current_matrix_dim = matrix.dimension;
            self.current_matrix_words = matrix.packed_data.len();
        } else if let Some(buffer) = &self.matrix_buffer {
            // Keep content fresh in case a future source updates matrix in place.
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&matrix.packed_data));
        }
    }

    fn ensure_uniform_buffer(&mut self, device: &wgpu::Device) {
        if self.uniform_buffer.is_some() {
            return;
        }
        self.uniform_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("QR uniforms"),
            size: core::mem::size_of::<QrUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
}

impl GpuView for BarcodeRenderer {
    async fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        _env: &mut waterui_core::Environment,
    ) {
        let (pipeline, bgl) =
            Self::create_render_pipeline(ctx.device, ctx.surface_format, ctx.pipeline_cache);
        self.render_pipeline = Some(pipeline);
        self.bind_group_layout = Some(bgl);
        self.ensure_uniform_buffer(ctx.device);
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        self.ensure_uniform_buffer(frame.device);
        self.ensure_matrix_buffer(frame.device, frame.queue);

        let Some(pipeline) = &self.render_pipeline else {
            return;
        };
        let Some(bind_group_layout) = &self.bind_group_layout else {
            return;
        };

        let Some(uniform_buffer) = &self.uniform_buffer else {
            return;
        };
        let Some(matrix_buffer) = &self.matrix_buffer else {
            return;
        };

        let uniforms = QrUniforms {
            matrix_dim: self.current_matrix_dim,
            quiet_zone: self.source.quiet_zone(),
            output_width: frame.width,
            output_height: frame.height,
            fill_mode: self.fill_mode,
            _pad0: [0; 7],
            solid_dark_color: self.solid_dark_color,
            light_color: self.light_color,
            gradient_start_color: self.gradient_start_color,
            gradient_end_color: self.gradient_end_color,
            gradient_start_point: self.gradient_start_point,
            gradient_end_point: self.gradient_end_point,
        };
        frame
            .queue
            .write_buffer(uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("QR render bind group"),
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
            ],
        });

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("QR renderer encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("QR render pass"),
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
            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }

        frame.queue.submit([encoder.finish()]);
    }
}

impl_gpu_subview!(BarcodeRenderer);
