//! GPU-based SVG renderer using Vello.
//!
//! This module provides `VelloSvgRenderer`, an implementation of `GpuView`
//! that renders SVG content directly on the GPU using Vello.
//!
//! Note: Vello does NOT support HDR (requires Rgba8Unorm format), but provides
//! potentially better quality and performance for complex vector graphics.

extern crate alloc;

use alloc::{string::String, sync::Arc};
use core::any::Any;
use core::sync::atomic::{AtomicBool, Ordering};
use waterui_core::Signal;
use waterui_graphics::{GpuContext, GpuFrame, GpuView};

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
    svg_tree: vello_svg::usvg::Tree,
    /// Cached converted scene from the source SVG.
    base_scene: vello::Scene,
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
    /// Parses SVG content and builds a cached Vello scene.
    fn parse_tree_and_scene(svg_content: &str) -> (vello_svg::usvg::Tree, vello::Scene) {
        let tree = vello_svg::usvg::Tree::from_str(svg_content, &vello_svg::usvg::Options::default())
            .expect("Failed to parse SVG");
        let svg_size = tree.size();
        assert!(
            svg_size.width().is_finite()
                && svg_size.height().is_finite()
                && svg_size.width() > 0.0
                && svg_size.height() > 0.0,
            "SVG must have positive finite dimensions, got {}x{}",
            svg_size.width(),
            svg_size.height()
        );
        let base_scene = vello_svg::render_tree(&tree);
        (tree, base_scene)
    }

    /// Creates a new Vello SVG renderer from SVG content.
    ///
    /// # Panics
    ///
    /// Panics if the SVG content cannot be parsed.
    #[must_use]
    pub fn new(svg_content: &str) -> Self {
        let (svg_tree, base_scene) = Self::parse_tree_and_scene(svg_content);
        Self {
            svg_tree,
            base_scene,
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
    ///
    /// # Panics
    ///
    /// Panics if the new SVG content cannot be parsed.
    pub fn reparse_svg(&mut self, svg_content: &str) {
        let (svg_tree, base_scene) = Self::parse_tree_and_scene(svg_content);
        self.svg_tree = svg_tree;
        self.base_scene = base_scene;
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
        use vello::kurbo::Affine;
        let svg_size = self.svg_tree.size();
        let svg_width = svg_size.width();
        let svg_height = svg_size.height();

        // Calculate transform to fit SVG in target size
        let scale_x = width / svg_width;
        let scale_y = height / svg_height;
        let scale = scale_x.min(scale_y);

        // Center the SVG
        let offset_x = f64::from((width - svg_width * scale) / 2.0);
        let offset_y = f64::from((height - svg_height * scale) / 2.0);

        let transform = Affine::translate((offset_x, offset_y)) * Affine::scale(f64::from(scale));

        let mut scene = vello::Scene::new();
        scene.append(&self.base_scene, Some(transform));

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
        #[allow(clippy::cast_precision_loss)]
        let scene = self.build_scene(width as f32, height as f32);
        if let (Some(renderer), Some(texture)) = (&mut self.renderer, &self.texture) {
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let render_params = vello::RenderParams {
                base_color: vello::peniko::Color::TRANSPARENT,
                width,
                height,
                antialiasing_method: vello::AaConfig::Area,
            };

            renderer
                .render_to_texture(device, queue, &scene, &view, &render_params)
                .expect("Vello render failed");
        }
    }
}

/// Placeholder token used by reactive SVG templates for tint substitution.
pub(crate) const SVG_COLOR_PLACEHOLDER: &str = "__WATERUI_SVG_COLOR__";

/// Reactive wrapper that updates SVG tint without recreating GPU surface state.
pub(crate) struct ReactiveVelloSvgRenderer<S>
where
    S: Signal<Output = String>,
{
    inner: VelloSvgRenderer,
    svg_template: String,
    color_signal: S,
    pending_update: Arc<AtomicBool>,
    _watcher_guard: Option<Box<dyn Any>>,
    last_color: String,
    template_uses_color: bool,
}

impl<S> ReactiveVelloSvgRenderer<S>
where
    S: Signal<Output = String>,
{
    /// Creates a reactive renderer from an SVG template and color signal.
    #[must_use]
    pub fn new(svg_template: impl Into<String>, color_signal: S) -> Self {
        let svg_template = svg_template.into();
        let template_uses_color = svg_template.contains(SVG_COLOR_PLACEHOLDER);
        let initial_color = color_signal.get();
        let initial_svg = if template_uses_color {
            svg_template.replace(SVG_COLOR_PLACEHOLDER, &initial_color)
        } else {
            svg_template.clone()
        };

        Self {
            inner: VelloSvgRenderer::new(&initial_svg),
            svg_template,
            color_signal,
            pending_update: Arc::new(AtomicBool::new(false)),
            _watcher_guard: None,
            last_color: initial_color,
            template_uses_color,
        }
    }

    fn take_pending_update(&self) -> bool {
        self.pending_update.swap(false, Ordering::AcqRel)
    }

    fn sync_svg_from_color(&mut self) {
        if !self.take_pending_update() {
            return;
        }

        let color = self.color_signal.get();
        if color == self.last_color {
            return;
        }

        if self.template_uses_color {
            let svg_content = self.svg_template.replace(SVG_COLOR_PLACEHOLDER, &color);
            self.inner.reparse_svg(&svg_content);
        }

        self.last_color = color;
    }
}

impl<S> GpuView for ReactiveVelloSvgRenderer<S>
where
    S: Signal<Output = String>,
{
    fn setup(&mut self, ctx: &GpuContext, env: &mut waterui_core::Environment) -> impl core::future::Future<Output = ()> {
        if self.template_uses_color && self._watcher_guard.is_none() {
            let pending_update = Arc::clone(&self.pending_update);
            let redraw_handle = ctx.redraw_handle.clone();
            let guard = self.color_signal.watch(move |_context| {
                pending_update.store(true, Ordering::Release);
                redraw_handle.request_redraw();
            });
            self._watcher_guard = Some(Box::new(guard));
        }
        self.inner.setup(ctx, env)
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        self.sync_svg_from_color();
        self.inner.render(frame);
    }
}

impl GpuView for VelloSvgRenderer {
    fn setup(&mut self, ctx: &GpuContext, _env: &mut waterui_core::Environment) -> impl core::future::Future<Output = ()> {
        // Create Vello renderer
        let renderer = vello::Renderer::new(
            ctx.device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::area_only(),
                num_init_threads: std::num::NonZeroUsize::new(1),
                pipeline_cache: None,
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
                source: wgpu::ShaderSource::Wgsl(waterui_graphics::shaders::BLIT.source.into()),
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

    fn render(&mut self, frame: &mut GpuFrame) {
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
}
