//! The `GpuSurface` realization of a [`SceneView`](crate::scene_view::SceneView).
//!
//! A backend that draws its own scene merges scene content straight into the
//! parent scene and never reaches this module. Everything here is for the other
//! case: a scene that owns a surface of its own, rasterized by whichever engine
//! the device supports.

use alloc::boxed::Box;

use waterui_core::MainThreadBound;

use crate::gpu::shared_context::SceneEngine;
use crate::gpu_surface::{GpuContext, GpuFrame, GpuView};
use crate::scene_view::SceneContent;
use crate::scene2d_hybrid::{HybridScene2D, HybridUpload};
use crate::scene2d_vello::VelloScene2D;

/// The scene this surface builds each frame, in whichever engine's form.
///
/// The engine is a property of the device, so which variant this is settles
/// during setup and never changes afterwards.
/// Both are boxed: they are large and differ in size by an order of magnitude,
/// so every surface would otherwise carry the larger of the two whichever
/// engine it actually uses.
enum SceneBuffer {
    /// A scene for the compute pipeline, rasterized into an intermediate
    /// storage texture and blitted.
    Classic(Box<vello::Scene>),
    /// A scene for the CPU/GPU split pipeline, rasterized straight into the
    /// frame.
    Hybrid(Box<vello_hybrid::Scene>),
}

pub struct SceneSurfaceRenderer {
    // `SceneContent` is `!Send` (it carries an `Rc` invalidator). `measure` never
    // touches it (it returns a fixed size), so confining it keeps the renderer
    // `Send + Sync` for the `SubView` bound while setup/render stay on the main thread.
    content: MainThreadBound<Box<dyn SceneContent>>,
    // Which engine's scene this is settles in setup, once the device says which
    // one it rasterizes with.
    scene: Option<SceneBuffer>,
    // The renderer belongs to the device, not to this view: taken during setup
    // rather than built here, because a renderer's pipelines depend on the
    // device and not on this scene.
    renderer: Option<alloc::sync::Arc<crate::gpu::shared_context::SharedSceneRenderer>>,
    // Classic rasterizes through a compute pass into a storage texture, which
    // then has to be blitted; the hybrid engine draws straight into the frame
    // and leaves all of this unused.
    intermediate_texture: Option<wgpu::Texture>,
    intermediate_view: Option<wgpu::TextureView>,
    blit_pipeline: Option<wgpu::RenderPipeline>,
    blit_bind_group_layout: Option<wgpu::BindGroupLayout>,
    blit_sampler: Option<wgpu::Sampler>,
    intermediate_size: (u32, u32),
}

impl SceneSurfaceRenderer {
    pub fn new(content: Box<dyn SceneContent>) -> Self {
        Self {
            content: MainThreadBound::new(content),
            scene: None,
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
    fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        _env: &mut waterui_core::Environment,
    ) -> impl core::future::Future<Output = ()> {
        self.content
            .set_invalidator(Some(ctx.redraw_handle.invalidator()));

        // The renderer belongs to the device, so this takes a handle to the
        // shared one rather than building another. Building one per view made a
        // dozen icons compile a dozen copies of the same pipelines and reach a
        // first frame a dozen separate times, which is what icons appearing one
        // after another looks like.
        self.renderer = Some(alloc::sync::Arc::clone(ctx.scene_renderer()));

        if ctx.scene_renderer().engine() == SceneEngine::Hybrid {
            // The hybrid engine rasterizes through a render pass, so it draws
            // straight into the frame: no storage texture to rasterize into and
            // nothing to blit out of it. Sized on the first frame.
            self.scene = Some(SceneBuffer::Hybrid(Box::new(vello_hybrid::Scene::new(
                1, 1,
            ))));
            return core::future::ready(());
        }
        self.scene = Some(SceneBuffer::Classic(Box::new(vello::Scene::new())));

        let (vertex_shader, fragment_shader) =
            crate::shaders::BLIT.create_render_stages(ctx.device, "vs_main", "fs_main");

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
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });

        let blend = ctx.alpha_blend_state();

        self.blit_pipeline = Some(ctx.device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("SceneView blit pipeline"),
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
                        format: ctx.surface_format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            },
        ));

        self.blit_sampler = Some(ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("SceneView blit sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        }));

        self.blit_bind_group_layout = Some(bind_group_layout);
        core::future::ready(())
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        let renderer = alloc::sync::Arc::clone(
            self.renderer
                .as_ref()
                .expect("SceneView renderer used before setup"),
        );
        let needs_next_frame = match renderer.engine() {
            SceneEngine::Classic => self.render_classic(&renderer, frame),
            SceneEngine::Hybrid => self.render_hybrid(&renderer, frame),
        };
        if needs_next_frame {
            frame.request_redraw();
        }
    }
}

impl SceneSurfaceRenderer {
    /// Rasterizes through the compute pipeline into a storage texture, then
    /// blits that into the frame.
    #[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
    fn render_classic(
        &mut self,
        renderer: &crate::gpu::shared_context::SharedSceneRenderer,
        frame: &mut GpuFrame,
    ) -> bool {
        let SceneBuffer::Classic(scene) = self
            .scene
            .as_mut()
            .expect("SceneView scene used before setup")
        else {
            panic!("SceneView built a hybrid scene for the classic engine");
        };

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

        scene.reset();
        let needs_next_frame = {
            let mut scene2d = VelloScene2D::new(scene);
            self.content
                .build_scene(&mut scene2d, frame.width as f32, frame.height as f32)
        };

        renderer.with_classic(frame.device, |renderer| {
            renderer
                .render_to_texture(
                    frame.device,
                    frame.queue,
                    scene,
                    intermediate_view,
                    &vello::RenderParams {
                        base_color: peniko::Color::TRANSPARENT,
                        width: frame.width,
                        height: frame.height,
                        antialiasing_method: vello::AaConfig::Area,
                    },
                )
                .expect("SceneView vello render failed");
        });

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
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        frame.queue.submit([encoder.finish()]);
        needs_next_frame
    }

    /// Processes paths on the CPU and rasterizes them through a render pass
    /// straight into the frame — no compute, and so nothing that a device
    /// without indirect execution cannot do.
    #[allow(clippy::cast_precision_loss)]
    fn render_hybrid(
        &mut self,
        renderer: &crate::gpu::shared_context::SharedSceneRenderer,
        frame: &mut GpuFrame,
    ) -> bool {
        let SceneBuffer::Hybrid(scene) = self
            .scene
            .as_mut()
            .expect("SceneView scene used before setup")
        else {
            panic!("SceneView built a classic scene for the hybrid engine");
        };

        let width = u16::try_from(frame.width).expect("scene surface is wider than a hybrid scene");
        let height =
            u16::try_from(frame.height).expect("scene surface is taller than a hybrid scene");
        scene.reset_and_resize(width, height);

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("SceneView hybrid encoder"),
            });

        // The engine loads the target rather than clearing it, because it also
        // draws into targets it shares. This surface owns its frame outright,
        // so it starts from transparent.
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SceneView hybrid clear pass"),
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
        }));

        // The content is built inside the renderer's lock because glyphs and
        // images live in atlases the renderer owns: a run of text or an image
        // is recorded against those resources, not against the scene alone.
        // Uploads go into the same encoder the frame is rendered with, so an
        // image reaches the atlas before the pass that samples it.
        let content = &mut self.content;
        let needs_next_frame = renderer.with_hybrid(frame.device, frame.format, |hybrid| {
            let needs_next_frame = {
                let upload = HybridUpload::new(frame.device, frame.queue, &mut encoder);
                let mut scene2d = HybridScene2D::new(scene, hybrid, upload);
                content.build_scene(&mut scene2d, frame.width as f32, frame.height as f32)
            };
            hybrid
                .renderer
                .render(
                    scene,
                    &mut hybrid.resources,
                    frame.device,
                    frame.queue,
                    &mut encoder,
                    &vello_hybrid::RenderSize {
                        width: frame.width,
                        height: frame.height,
                    },
                    &frame.view,
                    &vello_hybrid::TextureBindings::new(),
                )
                .expect("SceneView hybrid render failed");
            needs_next_frame
        });

        frame.queue.submit([encoder.finish()]);
        needs_next_frame
    }
}
