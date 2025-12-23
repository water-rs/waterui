//! SVG renderer using resvg and GpuSurface.
//!
//! This module provides `SvgRenderer`, an implementation of `GpuRenderer`
//! that renders SVG content using the resvg library and wgpu.

use alloc::vec::Vec;

use crate::gpu_surface::{GpuContext, GpuFrame, GpuRenderer};

/// A GPU renderer for SVG content using resvg.
///
/// This renderer parses SVG content using usvg, rasterizes it with resvg,
/// and uploads the result to a GPU texture for display.
///
/// # Caching
///
/// The renderer caches the rasterized result and only re-renders when:
/// - The surface size changes
/// - `needs_redraw` is set to true
///
/// This makes static SVGs very efficient (just a texture blit per frame).
pub struct SvgRenderer {
    /// The parsed SVG tree.
    svg_tree: usvg::Tree,
    /// Cached GPU texture containing the rasterized SVG.
    cached_texture: Option<wgpu::Texture>,
    /// Bind group for sampling the texture.
    bind_group: Option<wgpu::BindGroup>,
    /// Render pipeline for blitting the texture.
    pipeline: Option<wgpu::RenderPipeline>,
    /// Bind group layout for the pipeline.
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    /// Sampler for the texture.
    sampler: Option<wgpu::Sampler>,
    /// Current cached width.
    current_width: u32,
    /// Current cached height.
    current_height: u32,
    /// Whether to force a re-render.
    needs_redraw: bool,
}

impl core::fmt::Debug for SvgRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SvgRenderer")
            .field("current_width", &self.current_width)
            .field("current_height", &self.current_height)
            .field("needs_redraw", &self.needs_redraw)
            .finish_non_exhaustive()
    }
}

impl SvgRenderer {
    /// Creates a new SVG renderer from SVG content.
    ///
    /// # Panics
    ///
    /// Panics if the SVG content cannot be parsed.
    #[must_use]
    pub fn new(svg_content: &str) -> Self {
        let tree = usvg::Tree::from_str(svg_content, &usvg::Options::default())
            .expect("Failed to parse SVG");
        Self {
            svg_tree: tree,
            cached_texture: None,
            bind_group: None,
            pipeline: None,
            bind_group_layout: None,
            sampler: None,
            current_width: 0,
            current_height: 0,
            needs_redraw: true,
        }
    }

    /// Creates a new SVG renderer from SVG path data.
    ///
    /// This wraps the path data in a minimal SVG document with the specified dimensions.
    ///
    /// # Panics
    ///
    /// Panics if the resulting SVG cannot be parsed.
    #[must_use]
    pub fn from_path(path_data: &str, width: f32, height: f32) -> Self {
        let svg_content = alloc::format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}"><path d="{}"/></svg>"#,
            width, height, path_data
        );
        Self::new(&svg_content)
    }

    /// Marks the renderer for re-render on the next frame.
    pub fn set_needs_redraw(&mut self) {
        self.needs_redraw = true;
    }

    /// Rasterizes the SVG at the given dimensions.
    fn rasterize(&self, width: u32, height: u32) -> Vec<u8> {
        let svg_size = self.svg_tree.size();

        // Create pixmap at target size
        let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(width, height) else {
            return alloc::vec![0u8; (width * height * 4) as usize];
        };

        // Calculate transform to fit SVG in target size
        let scale_x = width as f32 / svg_size.width();
        let scale_y = height as f32 / svg_size.height();
        let scale = scale_x.min(scale_y);

        // Center the SVG if aspect ratios don't match
        let offset_x = (width as f32 - svg_size.width() * scale) / 2.0;
        let offset_y = (height as f32 - svg_size.height() * scale) / 2.0;

        let transform = resvg::tiny_skia::Transform::from_translate(offset_x, offset_y)
            .pre_scale(scale, scale);

        resvg::render(&self.svg_tree, transform, &mut pixmap.as_mut());

        pixmap.take()
    }

    /// Creates the blit shader module.
    fn create_shader(device: &wgpu::Device) -> wgpu::ShaderModule {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SVG Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("svg_blit.wgsl").into()),
        })
    }

    /// Creates or updates the cached texture with new pixel data.
    fn update_texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
        let pixels = self.rasterize(width, height);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SVG Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        // Create bind group for the new texture
        if let (Some(layout), Some(sampler)) = (&self.bind_group_layout, &self.sampler) {
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SVG Bind Group"),
                layout,
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
            }));
        }

        self.cached_texture = Some(texture);
        self.current_width = width;
        self.current_height = height;
        self.needs_redraw = false;
    }
}

impl GpuRenderer for SvgRenderer {
    fn setup(&mut self, ctx: &GpuContext) {
        let shader = Self::create_shader(ctx.device);

        // Create sampler
        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("SVG Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        self.sampler = Some(sampler);

        // Create bind group layout
        let bind_group_layout = ctx.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SVG Bind Group Layout"),
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        self.bind_group_layout = Some(bind_group_layout.clone());

        // Create pipeline layout
        let pipeline_layout = ctx.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SVG Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create render pipeline
        let pipeline = ctx.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SVG Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
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
            cache: None,
        });
        self.pipeline = Some(pipeline);
    }

    fn render(&mut self, frame: &GpuFrame) {
        // Check if we need to re-rasterize
        if self.needs_redraw || self.current_width != frame.width || self.current_height != frame.height {
            self.update_texture(frame.device, frame.queue, frame.width, frame.height);
        }

        // Skip render if no pipeline or bind group
        let Some(pipeline) = &self.pipeline else { return };
        let Some(bind_group) = &self.bind_group else { return };

        // Create render pass and draw the texture
        let mut encoder = frame.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("SVG Encoder"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SVG Render Pass"),
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

            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..6, 0..1); // Full-screen quad (2 triangles)
        }

        frame.queue.submit([encoder.finish()]);
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width != self.current_width || height != self.current_height {
            self.needs_redraw = true;
        }
    }
}
