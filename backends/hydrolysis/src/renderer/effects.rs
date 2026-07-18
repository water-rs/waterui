//! GPU-composited effects: `AppliedFilter` runtimes and textures, view
//! effects, scene views, and embedded `GpuSurface` slots/layers.

use super::*;
use std::sync::Arc;
use waterui_graphics::filter_view::EffectFrameClock;

#[derive(Clone)]
pub(crate) struct EffectSetupResources {
    device: wgpu::Device,
    queue: wgpu::Queue,
    shader_cache: ShaderCache,
    host_redraw_handle: Option<RedrawHandle>,
}

impl EffectSetupResources {
    fn connect_redraw(&self, handle: &RedrawHandle) {
        let wake_host: Option<Arc<dyn Fn() + Send + Sync>> =
            self.host_redraw_handle.as_ref().map(|host| {
                let host = host.clone();
                Arc::new(move || host.request_redraw()) as Arc<dyn Fn() + Send + Sync>
            });
        handle.set_waker(wake_host);
    }

    fn wake_host(&self) {
        if let Some(handle) = &self.host_redraw_handle {
            handle.request_redraw();
        }
    }
}

pub(crate) struct AppliedFilterRuntime {
    filter: Option<AppliedFilter>,
    setup_complete: bool,
    input_texture: Option<AppliedFilterInputTexture>,
    output_texture: Option<AppliedFilterOutputTexture>,
    output_image: Option<vello::peniko::ImageData>,
    frame_clock: EffectFrameClock,
}

impl AppliedFilterRuntime {
    pub(super) fn new(filter: AppliedFilter) -> Self {
        Self {
            filter: Some(filter),
            setup_complete: false,
            input_texture: None,
            output_texture: None,
            output_image: None,
            frame_clock: EffectFrameClock::new(),
        }
    }

    pub(super) fn ensure_setup(
        runtime: &Rc<RefCell<Self>>,
        resources: EffectSetupResources,
        signals: FrameSignals,
    ) -> bool {
        {
            let runtime = runtime.borrow();
            if runtime.setup_complete {
                return true;
            }
            if runtime.filter.is_none() {
                return false;
            }
        }

        let runtime = Rc::clone(runtime);
        spawn_local(async move {
            Self::setup(runtime, resources.clone()).await;
            signals.request_refresh();
            resources.wake_host();
        })
        .detach();
        false
    }

    async fn setup(runtime: Rc<RefCell<Self>>, resources: EffectSetupResources) {
        let mut filter = {
            let mut runtime = runtime.borrow_mut();
            if runtime.setup_complete {
                return;
            }
            runtime
                .filter
                .take()
                .expect("hydrolysis AppliedFilter setup started concurrently")
        };
        resources.connect_redraw(&filter.redraw_handle());
        let context = EffectContext {
            device: &resources.device,
            queue: &resources.queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: None,
            shader_cache: &resources.shader_cache,
        };
        filter
            .setup(&context)
            .await
            .unwrap_or_else(|error| panic!("hydrolysis filter setup failed: {error}"));
        let mut runtime = runtime.borrow_mut();
        runtime.filter = Some(filter);
        runtime.setup_complete = true;
    }

    fn filter(&self) -> &AppliedFilter {
        assert!(self.setup_complete, "hydrolysis filter used before setup");
        self.filter
            .as_ref()
            .expect("hydrolysis filter missing after setup")
    }

    fn filter_mut(&mut self) -> &mut AppliedFilter {
        assert!(self.setup_complete, "hydrolysis filter used before setup");
        self.filter
            .as_mut()
            .expect("hydrolysis filter missing after setup")
    }

    pub(super) fn input_texture(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (&wgpu::Texture, &wgpu::TextureView) {
        if self
            .input_texture
            .as_ref()
            .is_none_or(|texture| texture.width != width || texture.height != height)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hydrolysis_applied_filter_input"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.input_texture = Some(AppliedFilterInputTexture {
                width,
                height,
                texture,
                view,
            });
        }

        let Some(texture) = self.input_texture.as_ref() else {
            panic!("hydrolysis AppliedFilter input texture cache missing after allocation");
        };
        (&texture.texture, &texture.view)
    }

    pub(super) fn has_input_texture(&self, width: u32, height: u32) -> bool {
        self.input_texture
            .as_ref()
            .is_some_and(|texture| texture.width == width && texture.height == height)
    }

    pub(super) fn output_texture(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (&wgpu::Texture, &wgpu::TextureView) {
        if self
            .output_texture
            .as_ref()
            .is_none_or(|texture| texture.width != width || texture.height != height)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hydrolysis_applied_filter_output"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.output_texture = Some(AppliedFilterOutputTexture {
                width,
                height,
                texture,
                view,
            });
        }

        let Some(texture) = self.output_texture.as_ref() else {
            panic!("hydrolysis AppliedFilter output texture cache missing after allocation");
        };
        (&texture.texture, &texture.view)
    }

    pub(super) fn needs_redraw_refresh(&mut self) -> bool {
        if !self.setup_complete {
            return false;
        }
        self.filter().redraw_hint()
    }

    /// The input dimensions the runtime last allocated its capture texture at.
    /// `None` before the first render. Used by the node-owned refresh path (the
    /// redraw-only frame does not re-flush the tree, so the node cannot supply
    /// the dimensions; the runtime remembers them from its last flush).
    pub(super) fn input_dimensions(&self) -> Option<(u32, u32)> {
        self.input_texture
            .as_ref()
            .map(|texture| (texture.width, texture.height))
    }

    pub(super) fn render_output(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vello_renderer: &mut vello::Renderer,
        width: u32,
        height: u32,
    ) -> (vello::peniko::ImageData, bool) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hydrolysis applied filter encoder"),
        });
        let output = self.encode_output(device, queue, vello_renderer, width, height, &mut encoder);
        queue.submit([encoder.finish()]);
        output
    }

    pub(super) fn encode_output(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vello_renderer: &mut vello::Renderer,
        width: u32,
        height: u32,
        encoder: &mut wgpu::CommandEncoder,
    ) -> (vello::peniko::ImageData, bool) {
        let (output_width, output_height) = self.filter().output_size(width, height);
        let (input_texture, input_view) = {
            let Some(input_texture) = self.input_texture.as_ref() else {
                panic!("hydrolysis AppliedFilter input texture missing before render");
            };
            (input_texture.texture.clone(), input_texture.view.clone())
        };
        let (output_texture, output_view) = {
            let (texture, view) = self.output_texture(device, output_width, output_height);
            (texture.clone(), view.clone())
        };
        let timing = self.frame_clock.tick();
        let input = EffectInput {
            device,
            queue,
            texture: &input_texture,
            view: input_view,
            format: wgpu::TextureFormat::Rgba8Unorm,
            width,
            height,
            timing,
        };
        let output = EffectOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_view,
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: output_width,
            height: output_height,
        };
        let needs_redraw = match self.filter_mut().encode_render(&input, &output, encoder) {
            Ok(needs_redraw) => needs_redraw || self.filter().redraw_hint(),
            Err(err) => {
                panic!("hydrolysis filter render failed: {err}");
            }
        };

        let image = if let Some(image) = self
            .output_image
            .as_ref()
            .filter(|image| image.width == output_width && image.height == output_height)
        {
            let texture_base = wgpu::TexelCopyTextureInfoBase {
                texture: output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            };
            let _ = vello_renderer.override_image(image, Some(texture_base));
            image.clone()
        } else {
            let image = vello_renderer.register_texture(output_texture);
            self.output_image = Some(image.clone());
            image
        };
        (image, needs_redraw)
    }
}

pub(crate) struct AppliedFilterInputTexture {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

pub(crate) struct AppliedFilterOutputTexture {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

pub(crate) struct ViewEffectRuntime {
    effect: Option<ViewEffectErased>,
    setup_complete: bool,
}

impl ViewEffectRuntime {
    pub(super) fn new(effect: ViewEffectErased) -> Self {
        Self {
            effect: Some(effect),
            setup_complete: false,
        }
    }

    pub(super) fn ensure_setup(
        runtime: &Rc<RefCell<Self>>,
        resources: EffectSetupResources,
        signals: FrameSignals,
    ) -> bool {
        {
            let runtime = runtime.borrow();
            if runtime.setup_complete {
                return true;
            }
            if runtime.effect.is_none() {
                return false;
            }
        }

        let runtime = Rc::clone(runtime);
        spawn_local(async move {
            Self::setup(runtime, resources.clone()).await;
            signals.request_refresh();
            resources.wake_host();
        })
        .detach();
        false
    }

    async fn setup(runtime: Rc<RefCell<Self>>, resources: EffectSetupResources) {
        let mut effect = {
            let mut runtime = runtime.borrow_mut();
            if runtime.setup_complete {
                return;
            }
            runtime
                .effect
                .take()
                .expect("hydrolysis ViewEffect setup started concurrently")
        };
        resources.connect_redraw(&effect.redraw_handle());
        let context = ViewEffectContext {
            device: &resources.device,
            queue: &resources.queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: None,
            shader_cache: &resources.shader_cache,
        };
        effect.setup(&context).await;
        let mut runtime = runtime.borrow_mut();
        runtime.effect = Some(effect);
        runtime.setup_complete = true;
    }

    pub(super) fn effect(&self) -> &ViewEffectErased {
        assert!(
            self.setup_complete,
            "hydrolysis ViewEffect used before setup"
        );
        self.effect
            .as_ref()
            .expect("hydrolysis ViewEffect missing after setup")
    }

    pub(super) fn effect_mut(&mut self) -> &mut ViewEffectErased {
        assert!(
            self.setup_complete,
            "hydrolysis ViewEffect used before setup"
        );
        self.effect
            .as_mut()
            .expect("hydrolysis ViewEffect missing after setup")
    }
}

impl HydrolysisRenderer {
    pub(crate) fn effect_setup_resources(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> EffectSetupResources {
        EffectSetupResources {
            device: device.clone(),
            queue: queue.clone(),
            shader_cache: self.shader_cache.clone(),
            host_redraw_handle: self.host_redraw_handle.clone(),
        }
    }

    /// Await setup for all effects reachable from a statically prepared
    /// embedded Hydrolysis tree.
    pub(crate) async fn setup_embedded_effects(&self, context: &GpuContext<'_>) {
        let resources = self.effect_setup_resources(context.device, context.queue);
        for runtime in self.node_view_effects.iter().filter_map(Weak::upgrade) {
            ViewEffectRuntime::setup(runtime, resources.clone()).await;
        }
        for runtime in self.node_applied_filters.clone() {
            AppliedFilterRuntime::setup(runtime, resources.clone()).await;
        }
    }

    pub(crate) fn refresh_active_applied_filters(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        // Prune retained render-tree filters whose node was dropped (only this
        // registry still holds the `Rc`).
        self.node_applied_filters
            .retain(|runtime| Rc::strong_count(runtime) > 1);
        // Node-owned filters supply their dimensions from the runtime's last
        // flush (the redraw-only frame does not re-flush the tree). A filter that
        // has never rendered has no input texture yet and is skipped.
        let active_filters = self
            .node_applied_filters
            .iter()
            .filter_map(|runtime| {
                runtime
                    .borrow()
                    .input_dimensions()
                    .map(|(width, height)| (Rc::clone(runtime), width, height))
            })
            .collect::<Vec<_>>();
        if active_filters.is_empty() {
            return;
        }
        let mut encoder = None;
        for (runtime, width, height) in active_filters {
            if !runtime.borrow_mut().needs_redraw_refresh() {
                continue;
            }
            let encoder = encoder.get_or_insert_with(|| {
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("hydrolysis active applied filters encoder"),
                })
            });
            let effect_started_at = Instant::now();
            let needs_redraw = runtime
                .borrow_mut()
                .encode_output(
                    device,
                    queue,
                    &mut self.vello_renderer,
                    width,
                    height,
                    encoder,
                )
                .1;
            self.frame_applied_filter_effect += effect_started_at.elapsed();
            self.frame_applied_filter_count = self
                .frame_applied_filter_count
                .checked_add(1)
                .expect("hydrolysis applied filter counter overflow");
            if needs_redraw {
                self.request_redraw();
            }
        }
        if let Some(encoder) = encoder {
            queue.submit([encoder.finish()]);
        }
    }

    pub(crate) fn push_gpu_surface_layer(
        &mut self,
        source: GpuSurfaceSource,
        transform: vello::kurbo::Affine,
        bounds: vello::kurbo::Rect,
    ) {
        if self
            .compositor
            .active_scene_layers
            .iter()
            .any(|layer| layer.alpha <= HIT_TEST_ALPHA_THRESHOLD)
        {
            return;
        }

        self.flush_vello_scene_layer();
        let direct_to_target = self.compositor.render_layers.is_empty()
            && self.compositor.active_scene_layers.is_empty()
            && affine_near(transform, vello::kurbo::Affine::IDENTITY)
            && rect_near(bounds, self.window_bounds);
        self.compositor
            .render_layers
            .push(RenderLayer::GpuSurface(GpuSurfaceLayer {
                source,
                transform,
                bounds,
                active_layers: self.compositor.active_scene_layers.clone(),
                direct_to_target,
            }));
    }

    pub fn poll_gpu_surface_redraw_handles(&mut self) -> bool {
        let mut requested = false;
        // Retained render-tree GPU surfaces own their runtime; prune any whose
        // node has been dropped (only this registry still holds the `Rc`), then
        // poll the live ones for off-thread redraw requests.
        self.node_gpu_surfaces
            .retain(|runtime| Rc::strong_count(runtime) > 1);
        for runtime in &self.node_gpu_surfaces {
            if runtime.borrow().take_external_redraw_request() {
                requested = true;
            }
        }
        if requested {
            self.signals.request_redraw();
        }
        requested
    }

    /// Register a retained render-tree GPU surface runtime (owned by a
    /// `GpuSurfaceNode`) so its off-thread redraw handle is polled. Called once
    /// at node build time; the node keeps the only other `Rc`, so a dropped node
    /// is pruned by [`Self::poll_gpu_surface_redraw_handles`].
    pub(crate) fn register_node_gpu_surface(
        &mut self,
        runtime: Rc<RefCell<EmbeddedGpuSurfaceRuntime>>,
    ) {
        self.node_gpu_surfaces.push(runtime);
    }

    pub(crate) fn register_node_view_effect(&mut self, runtime: Rc<RefCell<ViewEffectRuntime>>) {
        self.node_view_effects.push(Rc::downgrade(&runtime));
    }

    /// Register a retained render-tree applied-filter runtime (owned by an
    /// `AppliedFilterNode`) so animated filters are refreshed on redraw-only
    /// frames. Called once at node build time; pruned by strong count when the
    /// owning node is dropped.
    pub(crate) fn register_node_applied_filter(
        &mut self,
        runtime: Rc<RefCell<AppliedFilterRuntime>>,
    ) {
        self.node_applied_filters.push(runtime);
    }

    pub(crate) fn applied_filter_stats(&self) -> (u32, u64, u64) {
        (
            self.frame_applied_filter_count,
            duration_micros_u64(self.frame_applied_filter_capture),
            duration_micros_u64(self.frame_applied_filter_effect),
        )
    }
}
