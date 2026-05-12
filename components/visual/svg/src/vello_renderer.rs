//! GPU-based SVG renderer using Vello.
//!
//! This module provides `VelloSvgRenderer`, an implementation of `GpuView`
//! that renders SVG content directly on the GPU using Vello.

extern crate alloc;

use core::fmt;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_graphics::{GpuContext, GpuFrame, GpuView};

/// Parsed SVG data reused across renderer paths.
pub struct SvgSceneData {
    svg_tree: vello_svg::usvg::Tree,
    base_scene: vello::Scene,
}

impl SvgSceneData {
    /// Parses SVG content and builds a cached Vello scene.
    #[must_use]
    pub fn parse(svg_content: &str) -> Self {
        let svg_tree =
            vello_svg::usvg::Tree::from_str(svg_content, &vello_svg::usvg::Options::default())
                .expect("failed to parse SVG content");
        let svg_size = svg_tree.size();
        assert!(
            svg_size.width().is_finite()
                && svg_size.height().is_finite()
                && svg_size.width() > 0.0
                && svg_size.height() > 0.0,
            "SVG must have positive finite dimensions, got {}x{}",
            svg_size.width(),
            svg_size.height()
        );

        Self {
            base_scene: vello_svg::render_tree(&svg_tree),
            svg_tree,
        }
    }

    pub fn reparse(&mut self, svg_content: &str) {
        *self = Self::parse(svg_content);
    }

    #[must_use]
    pub fn build_scene(&self, width: f32, height: f32) -> vello::Scene {
        use kurbo::Affine;

        let svg_size = self.svg_tree.size();
        let svg_width = svg_size.width();
        let svg_height = svg_size.height();

        let scale_x = width / svg_width;
        let scale_y = height / svg_height;
        let scale = scale_x.min(scale_y);

        let offset_x = f64::from(svg_width.mul_add(-scale, width) / 2.0);
        let offset_y = f64::from(svg_height.mul_add(-scale, height) / 2.0);
        let transform = Affine::translate((offset_x, offset_y)) * Affine::scale(f64::from(scale));

        let mut scene = vello::Scene::new();
        scene.append(&self.base_scene, Some(transform));
        scene
    }
}

/// A GPU renderer for SVG content using Vello.
pub struct VelloSvgRenderer {
    scene_data: SvgSceneData,
    renderer: Option<vello::Renderer>,
    texture: Option<wgpu::Texture>,
    bind_group: Option<wgpu::BindGroup>,
    blit_pipeline: Option<wgpu::RenderPipeline>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    sampler: Option<wgpu::Sampler>,
    current_width: u32,
    current_height: u32,
}

impl fmt::Debug for VelloSvgRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VelloSvgRenderer")
            .field("current_width", &self.current_width)
            .field("current_height", &self.current_height)
            .finish_non_exhaustive()
    }
}

impl VelloSvgRenderer {
    /// Creates a new Vello SVG renderer from SVG content.
    #[must_use]
    pub fn new(svg_content: &str) -> Self {
        Self {
            scene_data: SvgSceneData::parse(svg_content),
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

    /// Replaces the parsed SVG content while keeping existing GPU resources.
    pub fn reparse_svg(&mut self, svg_content: &str) {
        self.scene_data.reparse(svg_content);
    }

    /// Creates a new Vello SVG renderer from SVG path data.
    #[must_use]
    pub fn from_path(path_data: &str, width: f32, height: f32) -> Self {
        let svg_content = alloc::format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}"><path d="{path_data}"/></svg>"#
        );
        Self::new(&svg_content)
    }

    /// Creates or updates the texture and renders the scene.
    fn render_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) {
        if self.texture.is_none() || self.current_width != width || self.current_height != height {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("vello_svg_texture"),
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

            if let (Some(layout), Some(sampler)) = (&self.bind_group_layout, &self.sampler) {
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vello_svg_bind_group"),
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

        #[allow(clippy::cast_precision_loss)]
        let scene = self.scene_data.build_scene(width as f32, height as f32);
        let renderer = self
            .renderer
            .as_mut()
            .expect("VelloSvgRenderer::render_to_texture called before setup");
        let texture = self
            .texture
            .as_ref()
            .expect("VelloSvgRenderer texture missing");
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        renderer
            .render_to_texture(
                device,
                queue,
                &scene,
                &view,
                &vello::RenderParams {
                    base_color: peniko::Color::TRANSPARENT,
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .expect("VelloSvgRenderer render_to_texture failed");
    }
}

/// Placeholder token used by reactive SVG templates for tint substitution.
pub const SVG_COLOR_PLACEHOLDER: &str = "__WATERUI_SVG_COLOR__";

impl GpuView for VelloSvgRenderer {
    #[allow(clippy::too_many_lines)]
    fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        _env: &mut waterui_core::Environment,
    ) -> impl core::future::Future<Output = ()> {
        self.renderer = Some(
            vello::Renderer::new(
                ctx.device,
                vello::RendererOptions {
                    use_cpu: false,
                    antialiasing_support: vello::AaSupport::area_only(),
                    num_init_threads: std::num::NonZeroUsize::new(1),
                    pipeline_cache: None,
                },
            )
            .expect("failed to create Vello renderer"),
        );

        self.sampler = Some(ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vello_svg_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        }));

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("vello_svg_bind_group_layout"),
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

        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(waterui_graphics::shaders::BLIT.label),
                source: wgpu::ShaderSource::Wgsl(waterui_graphics::shaders::BLIT.source.into()),
            });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("vello_svg_pipeline_layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let blend = if ctx.is_hdr() {
            None
        } else {
            Some(wgpu::BlendState::ALPHA_BLENDING)
        };

        self.blit_pipeline = Some(ctx.device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("vello_svg_blit_pipeline"),
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
                        blend,
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
                cache: ctx.pipeline_cache,
            },
        ));
        core::future::ready(())
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        self.render_to_texture(frame.device, frame.queue, frame.width, frame.height);

        let pipeline = self
            .blit_pipeline
            .as_ref()
            .expect("VelloSvgRenderer blit pipeline missing");
        let bind_group = self
            .bind_group
            .as_ref()
            .expect("VelloSvgRenderer bind group missing");

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vello_svg_blit_encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vello_svg_blit_pass"),
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
}

impl SubView for VelloSvgRenderer {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let svg_size = self.scene_data.svg_tree.size();
        let intrinsic = Size::new(svg_size.width(), svg_size.height());
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(intrinsic.width),
            proposal.height.unwrap_or(intrinsic.height),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}
