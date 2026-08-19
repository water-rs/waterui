//! GPU mask effect for filling QR dark modules with arbitrary GPU content.

use bytemuck::{Pod, Zeroable};
use core::fmt;
use waterui_core::{
    Computed, Signal,
    reactive::{signal::IntoComputed, watcher::BoxWatcherGuard},
};
use wgpu::util::DeviceExt;

use crate::qr::ReactiveBarcodeContent;
use crate::{BarcodeSource, BarcodeSymbology, shaders::QR_MASK_EFFECT};
use waterui_core::Str;
use waterui_graphics::{
    EffectRenderer, ViewEffectContext, ViewEffectInput, ViewEffectOutput, color::ResolvedColor,
    view_effect::ViewEffectRedrawCallback,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MaskUniforms {
    matrix_width: u32,
    matrix_height: u32,
    quiet_zone_x: u32,
    quiet_zone_y: u32,
    output_width: u32,
    output_height: u32,
    preserve_square_modules: u32,
    _padding: u32,
    light_color: [f32; 4],
}

/// Applies a QR mask over an input texture:
/// - dark modules sample from input texture
/// - light modules output configured light color
pub struct BarcodeMaskEffect {
    source: BarcodeSource,
    reactive_content: Option<ReactiveBarcodeContent>,
    pipeline: Option<wgpu::RenderPipeline>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    uniform_buffer: Option<wgpu::Buffer>,
    matrix_buffer: Option<wgpu::Buffer>,
    sampler: Option<wgpu::Sampler>,
    matrix_width: u32,
    matrix_height: u32,
    light_color: Computed<ResolvedColor>,
    light_color_guard: Option<BoxWatcherGuard>,
    redraw_callback: Option<ViewEffectRedrawCallback>,
}

impl fmt::Debug for BarcodeMaskEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarcodeMaskEffect")
            .field("matrix_width", &self.matrix_width)
            .field("matrix_height", &self.matrix_height)
            .finish_non_exhaustive()
    }
}

impl BarcodeMaskEffect {
    /// Creates a new mask effect from a barcode source and light module color.
    ///
    /// The resolved color remains reactive for the effect lifetime.
    #[must_use]
    pub fn new(source: BarcodeSource, light_color: impl IntoComputed<ResolvedColor>) -> Self {
        Self {
            source,
            reactive_content: None,
            pipeline: None,
            bind_group_layout: None,
            uniform_buffer: None,
            matrix_buffer: None,
            sampler: None,
            matrix_width: 0,
            matrix_height: 0,
            light_color: light_color.into_computed(),
            light_color_guard: None,
            redraw_callback: None,
        }
    }

    /// Creates a mask effect whose barcode content follows a signal.
    ///
    /// # Panics
    ///
    /// Panics when the signal's current or any later value cannot be encoded
    /// for `symbology`; pre-validate runtime user input with
    /// [`BarcodeSource::qr`] / [`BarcodeSource::code128`].
    #[must_use]
    pub fn reactive(
        symbology: BarcodeSymbology,
        content: impl IntoComputed<Str>,
        light_color: impl IntoComputed<ResolvedColor>,
    ) -> Self {
        let reactive_content = ReactiveBarcodeContent::new(symbology, content.into_computed());
        let source = reactive_content.initial_source();
        let mut effect = Self::new(source, light_color);
        effect.reactive_content = Some(reactive_content);
        effect
    }

    fn create_pipeline(
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
        let (vertex_shader, fragment_shader) =
            QR_MASK_EFFECT.create_render_stages(device, "vs_main", "fs_main");

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
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("QR mask effect pipeline"),
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
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("QR mask effect sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
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

    fn create_matrix_buffer(&mut self, device: &wgpu::Device) {
        let matrix = self.source.matrix();
        self.matrix_width = matrix.width;
        self.matrix_height = matrix.height;
        self.matrix_buffer = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("QR mask matrix buffer"),
                contents: bytemuck::cast_slice(&matrix.packed_data),
                usage: wgpu::BufferUsages::STORAGE,
            }),
        );
    }
}

impl EffectRenderer for BarcodeMaskEffect {
    fn set_redraw_callback(&mut self, callback: ViewEffectRedrawCallback) {
        assert!(
            self.redraw_callback.replace(callback).is_none(),
            "BarcodeMaskEffect redraw callback was installed more than once"
        );
    }

    fn setup(&mut self, ctx: &ViewEffectContext) -> impl core::future::Future<Output = ()> {
        let redraw = self
            .redraw_callback
            .as_ref()
            .expect("BarcodeMaskEffect requires a redraw callback before setup")
            .clone();
        self.light_color_guard = Some(self.light_color.watch({
            let redraw = redraw.clone();
            move |_| redraw()
        }));
        if let Some(reactive_content) = &mut self.reactive_content {
            let redraw = redraw.clone();
            reactive_content.install(move || redraw());
        }
        let (pipeline, bgl, sampler) = Self::create_pipeline(ctx.device, ctx.output_format);
        self.pipeline = Some(pipeline);
        self.bind_group_layout = Some(bgl);
        self.sampler = Some(sampler);
        self.ensure_uniform_buffer(ctx.device);
        self.create_matrix_buffer(ctx.device);

        async {}
    }

    fn render(&mut self, input: &ViewEffectInput, output: &ViewEffectOutput) {
        if let Some(source) = self
            .reactive_content
            .as_mut()
            .and_then(ReactiveBarcodeContent::take_reencoded)
        {
            self.source = source;
            self.create_matrix_buffer(output.device);
        }
        let pipeline = self
            .pipeline
            .as_ref()
            .expect("BarcodeMaskEffect render called before setup");
        let bind_group_layout = self
            .bind_group_layout
            .as_ref()
            .expect("BarcodeMaskEffect render called before setup");
        let sampler = self
            .sampler
            .as_ref()
            .expect("BarcodeMaskEffect render called before setup");
        let uniform_buffer = self
            .uniform_buffer
            .as_ref()
            .expect("BarcodeMaskEffect render called before setup");
        let matrix_buffer = self
            .matrix_buffer
            .as_ref()
            .expect("BarcodeMaskEffect matrix buffer was not created");

        let light_color = self.light_color.get();

        let uniforms = MaskUniforms {
            matrix_width: self.matrix_width,
            matrix_height: self.matrix_height,
            quiet_zone_x: self.source.quiet_zone(),
            quiet_zone_y: self.source.vertical_quiet_zone(),
            output_width: output.width,
            output_height: output.height,
            preserve_square_modules: u32::from(self.source.preserves_square_modules()),
            _padding: 0,
            light_color: [
                light_color.red,
                light_color.green,
                light_color.blue,
                light_color.opacity,
            ],
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
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        output.queue.submit([encoder.finish()]);
    }
}
