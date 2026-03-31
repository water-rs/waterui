use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::TypeId;
use core::fmt;

use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, Native, NativeView, View};

use crate::gpu_surface::{GpuContext, GpuFrame, GpuSurface, GpuView};
use crate::scene2d::Scene2D;
use crate::scene2d_vello::VelloScene2D;

/// Environment marker: render `SceneView` directly in the backend scene.
#[derive(Debug, Clone, Copy, Default)]
pub struct SceneViewMergeToParent;

pub type SceneInvalidator = Rc<dyn Fn()>;

/// Object-safe scene producer for `SceneView`.
pub trait SceneContent: 'static {
    /// Build commands into the provided scene.
    ///
    /// Returns true when the content requires another frame to be rendered.
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) -> bool;

    /// Installs an invalidation callback that content can trigger from signal watchers.
    fn set_invalidator(&mut self, _invalidator: Option<SceneInvalidator>) {}

    /// Returns the concrete runtime scene content type id behind this trait object.
    #[must_use]
    fn concrete_type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
}

/// A view that renders scene content either directly (backend) or via `GpuSurface`.
pub struct SceneView {
    content: Box<dyn SceneContent>,
}

impl fmt::Debug for SceneView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneView").finish_non_exhaustive()
    }
}

impl SceneView {
    #[must_use]
    pub fn new<C: SceneContent>(content: C) -> Self {
        Self {
            content: Box::new(content),
        }
    }

    /// Returns mutable access to the inner scene content.
    #[must_use]
    pub fn content_mut(&mut self) -> &mut dyn SceneContent {
        &mut *self.content
    }

    /// Returns the concrete runtime type id of the wrapped scene content.
    #[must_use]
    pub fn content_type_id(&self) -> TypeId {
        self.content.concrete_type_id()
    }

    /// Takes ownership of the wrapped scene content.
    #[must_use]
    pub fn into_content(self) -> Box<dyn SceneContent> {
        self.content
    }
}

impl NativeView for SceneView {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

impl View for SceneView {
    fn body(self, env: &Environment) -> impl View {
        if env.get::<SceneViewMergeToParent>().is_some() {
            AnyView::new(Native::new(self))
        } else {
            AnyView::new(GpuSurface::new(SceneSurfaceRenderer::new(self.content)))
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

struct SceneSurfaceRenderer {
    content: Box<dyn SceneContent>,
    scene: vello::Scene,
    renderer: Option<vello::Renderer>,
    intermediate_texture: Option<wgpu::Texture>,
    intermediate_view: Option<wgpu::TextureView>,
    blit_pipeline: Option<wgpu::RenderPipeline>,
    blit_bind_group_layout: Option<wgpu::BindGroupLayout>,
    blit_sampler: Option<wgpu::Sampler>,
    intermediate_size: (u32, u32),
}

impl SceneSurfaceRenderer {
    fn new(content: Box<dyn SceneContent>) -> Self {
        Self {
            content,
            scene: vello::Scene::new(),
            renderer: None,
            intermediate_texture: None,
            intermediate_view: None,
            blit_pipeline: None,
            blit_bind_group_layout: None,
            blit_sampler: None,
            intermediate_size: (0, 0),
        }
    }
}

impl GpuView for SceneSurfaceRenderer {
    async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        let redraw_handle = ctx.redraw_handle.clone();
        self.content
            .set_invalidator(Some(Rc::new(move || redraw_handle.request_redraw())));

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
            .expect("failed to create SceneView vello renderer"),
        );

        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(crate::shaders::BLIT.label),
                source: wgpu::ShaderSource::Wgsl(crate::shaders::BLIT.source.into()),
            });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("SceneView blit bind group layout"),
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

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("SceneView blit pipeline layout"),
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
                label: Some("SceneView blit pipeline"),
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
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: ctx.pipeline_cache,
            },
        ));

        self.blit_sampler = Some(ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("SceneView blit sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        }));

        self.blit_bind_group_layout = Some(bind_group_layout);
    }

    #[allow(clippy::cast_precision_loss)]
    fn render(&mut self, frame: &mut GpuFrame) {
        let renderer = self
            .renderer
            .as_mut()
            .expect("SceneView renderer used before setup");

        if self.intermediate_size != (frame.width, frame.height) {
            let texture = frame.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("SceneView intermediate texture"),
                size: wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.intermediate_view =
                Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
            self.intermediate_texture = Some(texture);
            self.intermediate_size = (frame.width, frame.height);
        }

        let intermediate_view = self
            .intermediate_view
            .as_ref()
            .expect("SceneView intermediate view missing");

        self.scene.reset();
        let needs_next_frame = {
            let mut scene2d = VelloScene2D::new(&mut self.scene);
            self.content
                .build_scene(&mut scene2d, frame.width as f32, frame.height as f32)
        };

        renderer
            .render_to_texture(
                frame.device,
                frame.queue,
                &self.scene,
                intermediate_view,
                &vello::RenderParams {
                    base_color: peniko::Color::TRANSPARENT,
                    width: frame.width,
                    height: frame.height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .expect("SceneView vello render failed");

        let bind_group_layout = self
            .blit_bind_group_layout
            .as_ref()
            .expect("SceneView blit bind group layout missing");
        let sampler = self
            .blit_sampler
            .as_ref()
            .expect("SceneView blit sampler missing");
        let pipeline = self
            .blit_pipeline
            .as_ref()
            .expect("SceneView blit pipeline missing");

        let bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SceneView blit bind group"),
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(intermediate_view),
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
                label: Some("SceneView blit encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SceneView blit pass"),
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
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        frame.queue.submit([encoder.finish()]);
        if needs_next_frame {
            frame.request_redraw();
        }
    }
}

crate::impl_gpu_subview!(SceneSurfaceRenderer);
