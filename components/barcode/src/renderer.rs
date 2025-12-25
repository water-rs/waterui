//! GPU renderer for QR codes.

use crate::BarcodeSource;
use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer};

/// A GPU renderer that displays a QR code.
///
/// Generates the QR code as a texture and blits it to the screen
/// using the standard blit shader.
pub struct BarcodeRenderer {
    source: BarcodeSource,
    // Blit pipeline resources
    blit_pipeline: Option<wgpu::RenderPipeline>,
    blit_bind_group_layout: Option<wgpu::BindGroupLayout>,
    sampler: Option<wgpu::Sampler>,
    // Per-frame resources
    qr_texture: Option<wgpu::Texture>,
    // Cached state
    current_matrix_dim: u32,
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
            blit_pipeline: None,
            blit_bind_group_layout: None,
            sampler: None,
            qr_texture: None,
            current_matrix_dim: 0,
        }
    }

    /// Creates the blit pipeline for rendering the QR texture to screen.
    fn create_blit_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(waterui_graphics::shaders::BLIT.label),
            source: wgpu::ShaderSource::Wgsl(waterui_graphics::shaders::BLIT.source.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("QR blit bind group layout"),
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
            label: Some("QR blit pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("QR blit pipeline"),
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
                    blend: None,
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("QR sampler"),
            mag_filter: wgpu::FilterMode::Nearest, // Crisp QR codes
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        (pipeline, bind_group_layout, sampler)
    }

    /// Generate the QR code as a texture with quiet zone included.
    fn generate_qr_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        matrix: &crate::QrMatrix,
        output_size: u32,
    ) -> wgpu::Texture {
        let matrix_dim = matrix.dimension;
        let quiet_zone = 4u32; // QR spec: 4 modules quiet zone
        let total_dim = matrix_dim + quiet_zone * 2;

        // Calculate module size (pixels per module)
        let module_size = output_size / total_dim;
        let actual_size = module_size * total_dim;

        // Generate RGBA texture data
        let mut texture_data = vec![255u8; (actual_size * actual_size * 4) as usize];

        for py in 0..actual_size {
            for px in 0..actual_size {
                let module_x = px / module_size;
                let module_y = py / module_size;

                let is_dark = if module_x >= quiet_zone
                    && module_x < quiet_zone + matrix_dim
                    && module_y >= quiet_zone
                    && module_y < quiet_zone + matrix_dim
                {
                    let qr_x = module_x - quiet_zone;
                    let qr_y = module_y - quiet_zone;
                    let linear_idx = (qr_y * matrix_dim + qr_x) as usize;
                    let word_idx = linear_idx / 32;
                    let bit_idx = linear_idx % 32;
                    (matrix.packed_data[word_idx] >> bit_idx) & 1 == 1
                } else {
                    false // Quiet zone is white
                };

                if is_dark {
                    let pixel_idx = ((py * actual_size + px) * 4) as usize;
                    texture_data[pixel_idx] = 0;     // R
                    texture_data[pixel_idx + 1] = 0; // G
                    texture_data[pixel_idx + 2] = 0; // B
                    // A stays 255
                }
            }
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("QR texture"),
            size: wgpu::Extent3d {
                width: actual_size,
                height: actual_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texture_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(actual_size * 4),
                rows_per_image: Some(actual_size),
            },
            wgpu::Extent3d {
                width: actual_size,
                height: actual_size,
                depth_or_array_layers: 1,
            },
        );

        texture
    }
}

impl GpuRenderer for BarcodeRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl std::future::Future<Output = ()> {
        // Create blit pipeline
        let (blit_pipeline, blit_bgl, sampler) =
            Self::create_blit_pipeline(ctx.device, ctx.surface_format, ctx.pipeline_cache);
        self.blit_pipeline = Some(blit_pipeline);
        self.blit_bind_group_layout = Some(blit_bgl);
        self.sampler = Some(sampler);

        async {}
    }

    fn render(&mut self, frame: &GpuFrame) {
        let Some(blit_pipeline) = &self.blit_pipeline else {
            return;
        };
        let Some(blit_bgl) = &self.blit_bind_group_layout else {
            return;
        };
        let Some(sampler) = &self.sampler else {
            return;
        };

        // Get output size first (immutable borrow)
        let output_size = self.source.size();

        // Get matrix data (mutable borrow for lazy generation)
        let matrix = self.source.matrix();
        let matrix_dim = matrix.dimension;

        // Check if we need to recreate the texture
        if self.current_matrix_dim != matrix_dim || self.qr_texture.is_none() {
            self.qr_texture = Some(Self::generate_qr_texture(
                frame.device,
                frame.queue,
                matrix,
                output_size,
            ));
            self.current_matrix_dim = matrix_dim;
        }

        let qr_texture = self.qr_texture.as_ref().expect("QR texture exists");

        // Create bind group
        let texture_view = qr_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("QR blit bind group"),
            layout: blit_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("QR renderer encoder"),
            });

        // Blit QR texture to frame
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("QR blit pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            render_pass.set_pipeline(blit_pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }

        frame.queue.submit([encoder.finish()]);
    }

    fn resize(&mut self, width: u32, height: u32) {
        // Update source size to match surface, maintaining square aspect
        let new_size = width.min(height);
        if new_size != self.source.size() {
            self.source.set_size(new_size);
            // Force texture regeneration on next render
            self.qr_texture = None;
        }
    }
}
