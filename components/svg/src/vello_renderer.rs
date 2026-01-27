//! GPU-based SVG renderer using Vello.
//!
//! This module provides `VelloSvgRenderer`, an implementation of `GpuRenderer`
//! that renders SVG content directly on the GPU using Vello.
//!
//! Note: Vello does NOT support HDR (requires Rgba8Unorm format), but provides
//! potentially better quality and performance for complex vector graphics.

extern crate alloc;

use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer};

/// A GPU renderer for SVG content using Vello.
///
/// This renderer uses Vello for direct GPU vector rendering instead of
/// CPU rasterization. It may provide better quality and performance for
/// complex SVGs, but does not support HDR output.
///
/// # Note
///
/// Vello requires the `Rgba8Unorm` or `Rgba8UnormSrgb` texture format,
/// so HDR colors (values > 1.0) are not supported.
pub struct VelloSvgRenderer {
    /// The parsed usvg tree.
    svg_tree: usvg::Tree,
    /// Vello renderer.
    renderer: Option<vello::Renderer>,
    /// Intermediate texture for Vello rendering.
    texture: Option<wgpu::Texture>,
    /// Bind group for blitting.
    bind_group: Option<wgpu::BindGroup>,
    /// Blit pipeline.
    blit_pipeline: Option<wgpu::RenderPipeline>,
    /// Bind group layout.
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    /// Sampler.
    sampler: Option<wgpu::Sampler>,
    /// Current cached width.
    current_width: u32,
    /// Current cached height.
    current_height: u32,
}

impl core::fmt::Debug for VelloSvgRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VelloSvgRenderer")
            .field("current_width", &self.current_width)
            .field("current_height", &self.current_height)
            .finish_non_exhaustive()
    }
}

impl VelloSvgRenderer {
    /// Creates a new Vello SVG renderer from SVG content.
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
            renderer: None,
            texture: None,
            bind_group: None,
            blit_pipeline: None,
            bind_group_layout: None,
            sampler: None,
            current_width: 0,
            current_height: 0,
        }
    }

    /// Creates a new Vello SVG renderer from SVG path data.
    #[must_use]
    pub fn from_path(path_data: &str, width: f32, height: f32) -> Self {
        let svg_content = alloc::format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}"><path d="{}"/></svg>"#,
            width,
            height,
            path_data
        );
        Self::new(&svg_content)
    }

    /// Converts usvg tree to vello scene.
    fn build_scene(&self, width: f32, height: f32) -> vello::Scene {
        use vello::kurbo::{Affine, Rect};
        use vello::peniko::{Brush, Color, Fill};

        let mut scene = vello::Scene::new();
        let svg_size = self.svg_tree.size();

        // Calculate transform to fit SVG in target size
        let scale_x = width / svg_size.width();
        let scale_y = height / svg_size.height();
        let scale = scale_x.min(scale_y);

        // Center the SVG
        let offset_x = f64::from((width - svg_size.width() * scale) / 2.0);
        let offset_y = f64::from((height - svg_size.height() * scale) / 2.0);

        let transform = Affine::translate((offset_x, offset_y)) * Affine::scale(f64::from(scale));

        // Render usvg tree to vello scene
        // Note: This is a simplified implementation. A full implementation would
        // traverse the usvg tree and convert each element to vello primitives.
        // For now, we render a placeholder rectangle.
        // TODO: Implement full usvg to vello conversion using vello_svg crate

        // Placeholder: render a rect with the SVG bounds
        let rect = Rect::new(
            0.0,
            0.0,
            f64::from(svg_size.width()),
            f64::from(svg_size.height()),
        );
        scene.fill(
            Fill::NonZero,
            transform,
            &Brush::Solid(Color::rgba8(200, 200, 200, 128)),
            None,
            &rect,
        );

        scene
    }

    /// Creates or updates the texture and renders the scene.
    fn render_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        // Create texture if needed
        if self.texture.is_none() || self.current_width != width || self.current_height != height {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Vello SVG Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });

            // Update bind group
            if let (Some(layout), Some(sampler)) = (&self.bind_group_layout, &self.sampler) {
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Vello SVG Bind Group"),
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

            self.texture = Some(texture);
            self.current_width = width;
            self.current_height = height;
        }

        // Render scene with Vello
        if let (Some(renderer), Some(texture)) = (&mut self.renderer, &self.texture) {
            #[allow(clippy::cast_precision_loss)]
            let scene = self.build_scene(width as f32, height as f32);
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let render_params = vello::RenderParams {
                base_color: vello::peniko::Color::TRANSPARENT,
                width,
                height,
                antialiasing_method: vello::AaConfig::Msaa16,
            };

            renderer
                .render_to_texture(device, queue, &scene, &view, &render_params)
                .expect("Vello render failed");
        }
    }
}

impl GpuRenderer for VelloSvgRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl core::future::Future<Output = ()> {
        // Create Vello renderer
        let renderer = vello::Renderer::new(
            ctx.device,
            vello::RendererOptions {
                surface_format: Some(wgpu::TextureFormat::Rgba8Unorm),
                use_cpu: false,
                antialiasing_support: vello::AaSupport::all(),
                num_init_threads: None,
            },
        )
        .expect("Failed to create Vello renderer");
        self.renderer = Some(renderer);

        // Create sampler
        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Vello SVG Sampler"),
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
        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Vello SVG Bind Group Layout"),
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

        // Create blit pipeline
        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(waterui_graphics::shaders::BLIT.label),
                source: wgpu::ShaderSource::Wgsl(
                    waterui_graphics::shaders::BLIT.source.clone().into(),
                ),
            });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Vello SVG Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let blend = if ctx.is_hdr() {
            None
        } else {
            Some(wgpu::BlendState::ALPHA_BLENDING)
        };

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Vello SVG Blit Pipeline"),
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
                        blend,
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
                cache: ctx.pipeline_cache,
            });
        self.blit_pipeline = Some(pipeline);

        async {} // Sync renderer - immediately ready
    }

    fn render(&mut self, frame: &GpuFrame) {
        // Render SVG to texture
        self.render_to_texture(frame.device, frame.queue, frame.width, frame.height);

        // Blit texture to frame
        let Some(pipeline) = &self.blit_pipeline else {
            return;
        };
        let Some(bind_group) = &self.bind_group else {
            return;
        };

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Vello SVG Encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Vello SVG Blit Pass"),
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
            pass.draw(0..6, 0..1);
        }

        frame.queue.submit([encoder.finish()]);
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width != self.current_width || height != self.current_height {
            self.current_width = 0; // Force texture recreation
            self.current_height = 0;
        }
    }
}
