use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer};
use crate::BarcodeSource;
use filtrate_core::{RenderContext, Source};

/// A renderer that displays a barcode source.
pub struct BarcodeRenderer {
    source: BarcodeSource,
    pipeline: Option<wgpu::RenderPipeline>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    sampler: Option<wgpu::Sampler>,
}

impl core::fmt::Debug for BarcodeRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BarcodeRenderer")
            .field("source", &self.source)
            .finish()
    }
}

impl BarcodeRenderer {
    /// Creates a new renderer from a barcode source.
    #[must_use]
    pub fn new(source: BarcodeSource) -> Self {
        Self { 
            source,
            pipeline: None,
            bind_group_layout: None,
            sampler: None,
        }
    }
}

impl GpuRenderer for BarcodeRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl std::future::Future<Output = ()> {
        let device = ctx.device;

        // Shader for blitting texture
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("barcode blit shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(r#"
                struct VertexOutput {
                    @builtin(position) position: vec4<f32>,
                    @location(0) uv: vec2<f32>,
                }

                @vertex
                fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
                    var positions = array<vec2<f32>, 6>(
                        vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0, -1.0), vec2<f32>(-1.0,  1.0),
                        vec2<f32>(-1.0,  1.0), vec2<f32>( 1.0, -1.0), vec2<f32>( 1.0,  1.0),
                    );
                    let pos = positions[vertex_index];
                    var output: VertexOutput;
                    output.position = vec4<f32>(pos, 0.0, 1.0);
                    output.uv = (pos + 1.0) * 0.5;
                    output.uv.y = 1.0 - output.uv.y; // Flip Y for typical image/GUI coords
                    return output;
                }

                @group(0) @binding(0) var t_diffuse: texture_2d<f32>;
                @group(0) @binding(1) var s_diffuse: sampler;

                @fragment
                fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
                    return textureSample(t_diffuse, s_diffuse, in.uv);
                }
            "#)),
        });

        // Bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("barcode bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("barcode pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("barcode render pipeline"),
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
                    format: ctx.surface_format,
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
            cache: None, // Disable cache
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("barcode sampler"),
            mag_filter: wgpu::FilterMode::Nearest, // Barcodes should be crisp
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        self.pipeline = Some(pipeline);
        self.bind_group_layout = Some(bind_group_layout);
        self.sampler = Some(sampler);

        async {}
    }

    fn render(&mut self, frame: &GpuFrame) {
        let (Some(pipeline), Some(bgl), Some(sampler)) = (
            &self.pipeline,
            &self.bind_group_layout,
            &self.sampler
        ) else {
            return; 
        };

        let mut encoder = frame.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("barcode renderer encoder"),
        });

        let mut source_view = None;

        {
            let mut ctx = RenderContext {
                device: frame.device,
                queue: frame.queue,
                encoder: &mut encoder,
                pipeline_cache: None,
            };

            // Resolve the source to get the texture view
            source_view = Some(self.source.resolve(&mut ctx));
        }
        
        // We know source_view exists after scope
        if let Some(view) = source_view {
            // Create bind group for this frame
            let bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("barcode frame bind group"),
                layout: bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            
            // Render pass
            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("barcode render pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                
                render_pass.set_pipeline(pipeline);
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.draw(0..6, 0..1);
            }
        }

        frame.queue.submit([encoder.finish()]);
    }
    
    fn resize(&mut self, width: u32, height: u32) {
        // Update source size to match surface, maintaining aspect ratio
        self.source.set_size(width.min(height));
    }
}
