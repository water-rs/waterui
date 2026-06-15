//! GPU-composited effects: `AppliedFilter` runtimes and textures, view
//! effects, scene views, and embedded `GpuSurface` slots/layers.

use super::*;

/// Renderer-owned slots for effect runtimes that hold persistent GPU
/// resources (textures, prepared pipelines) across structural rebuilds.
///
/// Slots are bound in dispatch order: the cursor resets at the start of every
/// rebuild and unbound slots are dropped when the rebuild finishes — the same
/// retention contract as `gpu_surface_slots` and the press controller. A
/// content/type change at a reused slot is handled by the caller via
/// `replace_*`, so a slot never serves a stale runtime kind.
#[derive(Default)]
pub(crate) struct EffectRuntimeSlots {
    scene_views: RuntimeSlots<RefCell<SceneViewRuntime>>,
    view_effects: RuntimeSlots<RefCell<ViewEffectRuntime>>,
    applied_filters: RuntimeSlots<RefCell<AppliedFilterRuntime>>,
}

impl EffectRuntimeSlots {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.scene_views.begin_rebuild_frame();
        self.view_effects.begin_rebuild_frame();
        self.applied_filters.begin_rebuild_frame();
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.scene_views.finish_rebuild_frame();
        self.view_effects.finish_rebuild_frame();
        self.applied_filters.finish_rebuild_frame();
    }
}

pub(crate) struct RuntimeSlots<T> {
    slots: Vec<Rc<T>>,
    cursor: usize,
}

impl<T> Default for RuntimeSlots<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            cursor: 0,
        }
    }
}

impl<T> RuntimeSlots<T> {
    fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
    }

    fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
    }

    fn bind(&mut self, init: impl FnOnce() -> T) -> Rc<T> {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("hydrolysis effect runtime slot cursor overflow");
        if index == self.slots.len() {
            self.slots.push(Rc::new(init()));
        }
        Rc::clone(&self.slots[index])
    }
}

pub(crate) struct AppliedFilterRuntime {
    filter: AppliedFilter,
    setup_complete: bool,
    input_texture: Option<AppliedFilterInputTexture>,
    output_texture: Option<AppliedFilterOutputTexture>,
    output_image: Option<vello::peniko::ImageData>,
}

impl AppliedFilterRuntime {
    pub(super) fn new(filter: AppliedFilter) -> Self {
        Self {
            filter,
            setup_complete: false,
            input_texture: None,
            output_texture: None,
            output_image: None,
        }
    }

    pub(super) fn replace_filter(&mut self, filter: AppliedFilter) {
        self.filter = filter;
        self.setup_complete = false;
        self.input_texture = None;
        self.output_texture = None;
        self.output_image = None;
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
        self.filter.sync_targets();
        self.filter.redraw_hint()
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
        let filter_context = EffectContext {
            device,
            queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: None,
        };
        if !self.setup_complete {
            match pollster::block_on(self.filter.setup(&filter_context)) {
                Ok(()) => {}
                Err(err) => {
                    panic!("hydrolysis filter setup failed: {err}");
                }
            }
            self.setup_complete = true;
        }
        self.filter.sync_targets();
        let (output_width, output_height) = self.filter.output_size(width, height);
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
        let input = EffectInput {
            device,
            queue,
            texture: &input_texture,
            view: input_view,
            format: wgpu::TextureFormat::Rgba8Unorm,
            width,
            height,
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
        let needs_redraw = match self.filter.encode_render(&input, &output, encoder) {
            Ok(needs_redraw) => needs_redraw || self.filter.redraw_hint(),
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

#[derive(Clone)]
pub(crate) struct ActiveAppliedFilter {
    runtime: Rc<RefCell<AppliedFilterRuntime>>,
    width: u32,
    height: u32,
}

pub(crate) struct ViewEffectRuntime {
    effect: ViewEffectErased,
    setup_complete: bool,
}

impl ViewEffectRuntime {
    pub(super) fn new(effect: ViewEffectErased) -> Self {
        Self {
            effect,
            setup_complete: false,
        }
    }

    pub(super) fn replace_effect(&mut self, effect: ViewEffectErased) {
        self.effect = effect;
        self.setup_complete = false;
    }
}

pub(crate) struct SceneViewRuntime {
    content: Box<dyn waterui_graphics::SceneContent>,
}

impl SceneViewRuntime {
    pub(super) fn new(content: Box<dyn waterui_graphics::SceneContent>) -> Self {
        Self { content }
    }

    pub(super) fn replace_content(&mut self, content: Box<dyn waterui_graphics::SceneContent>) {
        self.content = content;
    }
}

impl HydrolysisRenderer {
    pub(crate) fn render_gpu_surface(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        surface: Native<GpuSurface>,
        env: &Environment,
    ) {
        let slot_index = renderer.bind_gpu_surface_slot(surface.into_inner(), env);
        renderer.push_gpu_surface_layer(slot_index, ctx.transform, ctx.bounds);
    }

    pub(crate) fn render_scene_view(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        scene_view: Native<SceneView>,
        env: &Environment,
    ) {
        let _ = env;
        let scene_view = scene_view.into_inner();
        let incoming_content = Rc::new(RefCell::new(Some(scene_view.into_content())));
        let init_content = Rc::clone(&incoming_content);
        let runtime = renderer.effect_runtime_slots.scene_views.bind(move || {
            RefCell::new(SceneViewRuntime::new(
                init_content
                    .borrow_mut()
                    .take()
                    .expect("hydrolysis SceneView slot initializer must run exactly once"),
            ))
        });
        // Always adopt the incoming content. Slots are bound by dispatch-order
        // cursor, so a reused slot can correspond to a *different* logical
        // `SceneView` after the view tree reorders (a reactive collection whose
        // membership changed): keying the refresh on the concrete type would keep
        // the previous occupant's content whenever the two share a type (every SVG
        // icon is `SvgSceneContent`), rendering a stale icon. `build_scene` runs
        // each dispatch and `SceneViewRuntime` holds no GPU state to preserve, so
        // refreshing the content unconditionally is correct and cheap.
        if let Some(content) = incoming_content.borrow_mut().take() {
            runtime.borrow_mut().replace_content(content);
        }
        let rebuild_signals = renderer.signals.clone();
        let mut runtime = runtime.borrow_mut();
        runtime
            .content
            .set_invalidator(Some(Rc::new(move || rebuild_signals.request_rebuild())));

        let mut scene = vello::Scene::new();
        let mut scene2d = VelloScene2D::new(&mut scene);
        #[allow(clippy::cast_precision_loss)]
        let needs_next_frame = runtime.content.build_scene(
            &mut scene2d,
            ctx.bounds.width() as f32,
            ctx.bounds.height() as f32,
        );
        renderer.scene.append(
            &scene,
            Some(ctx.transform * vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))),
        );
        if needs_next_frame {
            renderer.request_next_frame_rebuild();
        }
    }

    pub(crate) fn render_view_effect(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        effect: Native<ViewEffectErased>,
        env: &Environment,
    ) {
        let _ = env;
        let incoming_effect = Rc::new(RefCell::new(Some(effect.into_inner())));
        let init_effect = Rc::clone(&incoming_effect);
        let runtime = renderer.effect_runtime_slots.view_effects.bind(move || {
            RefCell::new(ViewEffectRuntime::new(
                init_effect
                    .borrow_mut()
                    .take()
                    .expect("hydrolysis ViewEffect slot initializer must run exactly once"),
            ))
        });
        let mut runtime = runtime.borrow_mut();
        if let Some(mut effect) = incoming_effect.borrow_mut().take() {
            let incoming_type = effect.concrete_type_id();
            if runtime.effect.concrete_type_id() != incoming_type {
                runtime.replace_effect(effect);
            } else {
                runtime.effect.replace_content(effect.take_content());
                runtime.effect.set_output_size(effect.output_size());
            }
        }
        let (device, queue) = {
            let (device, queue) = renderer.state().frame_resources();
            (device.clone(), queue.clone())
        };

        let input_width = (ctx.bounds.width().max(1.0).round()) as u32;
        let input_height = (ctx.bounds.height().max(1.0).round()) as u32;
        let output_size = runtime.effect.output_size();
        let (output_width, output_height) = output_size.compute(input_width, input_height);
        assert!(
            !(output_width == 0 || output_height == 0),
            "hydrolysis ViewEffect requires non-zero output dimensions"
        );

        let subtree = Self::render_subtree_scene(renderer, ctx, env, runtime.effect.take_content());

        let input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_view_effect_input"),
            size: wgpu::Extent3d {
                width: input_width,
                height: input_height,
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
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());
        renderer
            .vello_renderer
            .render_to_texture(
                &device,
                &queue,
                &subtree,
                &input_view,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::TRANSPARENT,
                    width: input_width,
                    height: input_height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .expect("hydrolysis ViewEffect failed to capture child scene");

        let setup_context = ViewEffectContext {
            device: &device,
            queue: &queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: None,
        };
        if !runtime.setup_complete {
            pollster::block_on(runtime.effect.setup(&setup_context));
            runtime.setup_complete = true;
        }

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_view_effect_output"),
            size: wgpu::Extent3d {
                width: output_width,
                height: output_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let input = ViewEffectInput {
            device: &device,
            queue: &queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: input_width,
            height: input_height,
        };
        let output = ViewEffectOutput {
            device: &device,
            queue: &queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: output_width,
            height: output_height,
        };
        runtime.effect.render(&input, &output);
        let needs_redraw = runtime.effect.needs_redraw();
        drop(runtime);
        if needs_redraw {
            renderer.request_next_frame_rebuild();
        }

        let image = renderer.vello_renderer.register_texture(output_texture);
        renderer.compositor.active_filter_images.push(image.clone());
        let image_transform = vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))
            * vello::kurbo::Affine::scale_non_uniform(
                ctx.bounds.width() / f64::from(output_width),
                ctx.bounds.height() / f64::from(output_height),
            );
        renderer.scene.draw_image(
            &vello::peniko::ImageBrush::new(image),
            ctx.transform * image_transform,
        );
    }

    pub(super) fn render_applied_filter_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<AppliedFilter>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let incoming_filter = Rc::new(RefCell::new(Some(value)));
        let init_filter = Rc::clone(&incoming_filter);
        let runtime = renderer.effect_runtime_slots.applied_filters.bind(move || {
            RefCell::new(AppliedFilterRuntime::new(
                init_filter
                    .borrow_mut()
                    .take()
                    .expect("hydrolysis AppliedFilter slot initializer must run exactly once"),
            ))
        });
        if let Some(filter) = incoming_filter.borrow_mut().take() {
            let incoming_type = filter.concrete_type_id();
            let mut runtime = runtime.borrow_mut();
            if runtime.filter.concrete_type_id() != incoming_type {
                runtime.replace_filter(filter);
            }
        }
        let (device, queue) = {
            let (device, queue) = renderer.state().frame_resources();
            (device.clone(), queue.clone())
        };

        let width = (ctx.bounds.width().max(1.0).round()) as u32;
        let height = (ctx.bounds.height().max(1.0).round()) as u32;
        let should_capture_input = {
            let runtime = runtime.borrow();
            !renderer.reuse_applied_filter_inputs || !runtime.has_input_texture(width, height)
        };
        let input_view = {
            let mut runtime = runtime.borrow_mut();
            let (_, view) = runtime.input_texture(&device, width, height);
            view.clone()
        };
        if should_capture_input {
            let capture_started_at = Instant::now();
            let subtree_scene = Self::render_subtree_scene(renderer, ctx, env, content);
            renderer
                .vello_renderer
                .render_to_texture(
                    &device,
                    &queue,
                    &subtree_scene,
                    &input_view,
                    &vello::RenderParams {
                        base_color: vello::peniko::Color::TRANSPARENT,
                        width,
                        height,
                        antialiasing_method: vello::AaConfig::Area,
                    },
                )
                .expect("hydrolysis AppliedFilter: failed to render subtree");
            renderer.frame_applied_filter_capture += capture_started_at.elapsed();
        }

        let effect_started_at = Instant::now();
        let (image, needs_redraw) = runtime.borrow_mut().render_output(
            &device,
            &queue,
            &mut renderer.vello_renderer,
            width,
            height,
        );
        renderer.frame_applied_filter_effect += effect_started_at.elapsed();
        renderer.frame_applied_filter_count = renderer
            .frame_applied_filter_count
            .checked_add(1)
            .expect("hydrolysis applied filter counter overflow");
        renderer.remember_active_applied_filter(Rc::clone(&runtime), width, height);
        if needs_redraw {
            renderer.request_redraw();
        }

        let image_transform = vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))
            * vello::kurbo::Affine::scale_non_uniform(
                ctx.bounds.width() / f64::from(image.width),
                ctx.bounds.height() / f64::from(image.height),
            );
        let scene = renderer.scene_mut();
        scene.draw_image(
            &vello::peniko::ImageBrush::new(image),
            ctx.transform * image_transform,
        );
    }

    pub(super) fn remember_active_applied_filter(
        &mut self,
        runtime: Rc<RefCell<AppliedFilterRuntime>>,
        width: u32,
        height: u32,
    ) {
        self.remember_active_applied_filter_entry(ActiveAppliedFilter {
            runtime,
            width,
            height,
        });
    }

    pub(super) fn remember_active_applied_filter_entry(&mut self, active: ActiveAppliedFilter) {
        let index = self.active_applied_filter_cursor;
        self.active_applied_filter_cursor = self
            .active_applied_filter_cursor
            .checked_add(1)
            .expect("hydrolysis active AppliedFilter cursor overflow");
        if index == self.active_applied_filters.len() {
            self.active_applied_filters.push(active);
        } else {
            self.active_applied_filters[index] = active;
        }
    }

    pub(crate) fn refresh_active_applied_filters(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let active_filters = self
            .active_applied_filters
            .iter()
            .map(|filter| (Rc::clone(&filter.runtime), filter.width, filter.height))
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

    pub(super) fn bind_gpu_surface_slot(
        &mut self,
        surface: GpuSurface,
        env: &Environment,
    ) -> usize {
        let index = self.compositor.gpu_surface_cursor;
        self.compositor.gpu_surface_cursor = self
            .compositor
            .gpu_surface_cursor
            .checked_add(1)
            .expect("hydrolysis gpu surface slot cursor overflow");

        if index == self.compositor.gpu_surface_slots.len() {
            self.compositor
                .gpu_surface_slots
                .push(EmbeddedGpuSurfaceRuntime::new(surface, env));
        } else {
            self.compositor.gpu_surface_slots[index].replace_surface(surface, env);
        }

        index
    }

    pub(super) fn push_gpu_surface_layer(
        &mut self,
        slot_index: usize,
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
                slot_index,
                transform,
                bounds,
                active_layers: self.compositor.active_scene_layers.clone(),
                direct_to_target,
            }));
    }

    pub fn poll_gpu_surface_redraw_handles(&mut self) -> bool {
        let mut requested = false;
        for runtime in &self.compositor.gpu_surface_slots {
            if runtime.take_external_redraw_request() {
                requested = true;
            }
        }
        if requested {
            self.signals.request_redraw();
        }
        requested
    }

    pub(crate) fn applied_filter_stats(&self) -> (u32, u64, u64) {
        (
            self.frame_applied_filter_count,
            duration_micros_u64(self.frame_applied_filter_capture),
            duration_micros_u64(self.frame_applied_filter_effect),
        )
    }
}
