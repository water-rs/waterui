//! GPU renderer for packed barcode matrices.

use bytemuck::{Pod, Zeroable};
use core::fmt;
use nami::signal::IntoComputed;
use waterui_core::{Computed, Environment, layout::UnitPoint};
use wgpu::util::DeviceExt;

use crate::{BarcodeSource, shaders::QR_RENDER, view::BarcodeFill};
use waterui_graphics::{
    GpuContext, GpuFrame, GpuView,
    color::{Color, ResolvedColor, Srgb},
    reactive_color::ReactiveColor,
};

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
/// The matrix is generated on CPU and packed into a bit buffer once, then
/// rendered fully on GPU each frame by sampling that bit buffer directly in a
/// fragment shader. Colors stay reactive for the renderer lifetime; changes
/// resolve through the setup environment and wake the surface precisely.
pub struct BarcodeRenderer {
    source: BarcodeSource,
    fill: BarcodeFill,
    light_color: Computed<Color>,
    render_pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    matrix_dim: u32,
    reactive_colors: Option<ReactiveBarcodeColors>,
}

#[derive(Clone, Copy, Debug)]
struct ResolvedColors {
    fill_mode: u32,
    solid_dark_color: [f32; 4],
    light_color: [f32; 4],
    gradient_start_color: [f32; 4],
    gradient_end_color: [f32; 4],
    gradient_start_point: [f32; 2],
    gradient_end_point: [f32; 2],
}

enum ReactiveBarcodeFill {
    Solid(ReactiveColor),
    LinearGradient {
        start: ReactiveColor,
        end: ReactiveColor,
        start_point: UnitPoint,
        end_point: UnitPoint,
    },
}

struct ReactiveBarcodeColors {
    light: ReactiveColor,
    fill: ReactiveBarcodeFill,
}

impl ReactiveBarcodeColors {
    fn new(fill: &BarcodeFill, light: &Computed<Color>, env: &Environment) -> Self {
        let fill = match fill {
            BarcodeFill::Solid(color) => ReactiveBarcodeFill::Solid(ReactiveColor::new(color, env)),
            BarcodeFill::LinearGradient {
                start_color,
                end_color,
                start_point,
                end_point,
            } => ReactiveBarcodeFill::LinearGradient {
                start: ReactiveColor::new(start_color, env),
                end: ReactiveColor::new(end_color, env),
                start_point: *start_point,
                end_point: *end_point,
            },
        };
        Self {
            light: ReactiveColor::new(light, env),
            fill,
        }
    }

    fn install(&mut self, redraw: &waterui_graphics::RedrawHandle) {
        self.light.install(redraw);
        match &mut self.fill {
            ReactiveBarcodeFill::Solid(color) => color.install(redraw),
            ReactiveBarcodeFill::LinearGradient { start, end, .. } => {
                start.install(redraw);
                end.install(redraw);
            }
        }
    }

    fn resolve(&self) -> ResolvedColors {
        let light_color = resolved_color_to_array(&self.light.get());
        match &self.fill {
            ReactiveBarcodeFill::Solid(color) => ResolvedColors {
                fill_mode: 0,
                solid_dark_color: resolved_color_to_array(&color.get()),
                light_color,
                gradient_start_color: [0.0; 4],
                gradient_end_color: [0.0; 4],
                gradient_start_point: [0.0, 0.0],
                gradient_end_point: [1.0, 1.0],
            },
            ReactiveBarcodeFill::LinearGradient {
                start,
                end,
                start_point,
                end_point,
            } => ResolvedColors {
                fill_mode: 1,
                solid_dark_color: [0.0; 4],
                light_color,
                gradient_start_color: resolved_color_to_array(&start.get()),
                gradient_end_color: resolved_color_to_array(&end.get()),
                gradient_start_point: unit_point_to_array(*start_point),
                gradient_end_point: unit_point_to_array(*end_point),
            },
        }
    }
}

const fn resolved_color_to_array(color: &ResolvedColor) -> [f32; 4] {
    [color.red, color.green, color.blue, color.opacity]
}

const fn unit_point_to_array(point: UnitPoint) -> [f32; 2] {
    [point.x, point.y]
}

impl fmt::Debug for BarcodeRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarcodeRenderer")
            .field("source", &self.source)
            .field("matrix_dim", &self.matrix_dim)
            .finish_non_exhaustive()
    }
}

impl BarcodeRenderer {
    /// Creates a new renderer from a barcode source.
    ///
    /// Defaults to a solid black fill on a white background. Use
    /// [`Self::with_fill`] and [`Self::with_light_color`] to override.
    #[must_use]
    pub fn new(source: BarcodeSource) -> Self {
        Self {
            source,
            fill: BarcodeFill::default(),
            light_color: Computed::constant(Color::from(Srgb::WHITE)),
            render_pipeline: None,
            uniform_buffer: None,
            bind_group: None,
            matrix_dim: 0,
            reactive_colors: None,
        }
    }

    /// Sets the fill style for dark modules.
    #[must_use]
    pub fn with_fill(mut self, fill: BarcodeFill) -> Self {
        self.fill = fill;
        self
    }

    /// Sets the light module/background color.
    #[must_use]
    pub fn with_light_color(mut self, color: impl IntoComputed<Color>) -> Self {
        self.light_color = color.into_computed();
        self
    }

    fn create_render_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
        let (vertex_shader, fragment_shader) =
            QR_RENDER.create_render_stages(device, "vs_main", "fs_main");

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
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("QR render pipeline"),
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
            multiview_mask: None,
            cache: None,
        });

        (pipeline, bind_group_layout)
    }

    fn create_buffers_and_bind_group(
        &mut self,
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) {
        let matrix = self.source.matrix();
        self.matrix_dim = matrix.dimension;
        let matrix_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("QR matrix storage buffer"),
            contents: bytemuck::cast_slice(&matrix.packed_data),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("QR uniforms"),
            size: core::mem::size_of::<QrUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
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
        self.uniform_buffer = Some(uniform_buffer);
        self.bind_group = Some(bind_group);
    }
}

impl GpuView for BarcodeRenderer {
    fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        env: &mut waterui_core::Environment,
    ) -> impl core::future::Future<Output = ()> {
        let (pipeline, bgl) = Self::create_render_pipeline(ctx.device, ctx.surface_format);
        self.render_pipeline = Some(pipeline);
        self.create_buffers_and_bind_group(ctx.device, &bgl);
        let mut reactive_colors = ReactiveBarcodeColors::new(&self.fill, &self.light_color, env);
        reactive_colors.install(&ctx.redraw_handle);
        self.reactive_colors = Some(reactive_colors);
        core::future::ready(())
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        let pipeline = self
            .render_pipeline
            .as_ref()
            .expect("BarcodeRenderer render called before setup");
        let bind_group = self
            .bind_group
            .as_ref()
            .expect("BarcodeRenderer render called before setup");
        let uniform_buffer = self
            .uniform_buffer
            .as_ref()
            .expect("BarcodeRenderer render called before setup");
        let resolved = self
            .reactive_colors
            .as_ref()
            .expect("BarcodeRenderer render called before setup")
            .resolve();

        let uniforms = QrUniforms {
            matrix_dim: self.matrix_dim,
            quiet_zone: self.source.quiet_zone(),
            output_width: frame.width,
            output_height: frame.height,
            fill_mode: resolved.fill_mode,
            _pad0: [0; 7],
            solid_dark_color: resolved.solid_dark_color,
            light_color: resolved.light_color,
            gradient_start_color: resolved.gradient_start_color,
            gradient_end_color: resolved.gradient_end_color,
            gradient_start_point: resolved.gradient_start_point,
            gradient_end_point: resolved.gradient_end_point,
        };
        frame
            .queue
            .write_buffer(uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

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
                multiview_mask: None,
            });
            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }

        frame.queue.submit([encoder.finish()]);
    }
}
