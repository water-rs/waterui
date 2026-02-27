use core::f64::consts::TAU;
use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use nami::Signal;
use waterui::accessibility::{AccessibilityLabel, AccessibilityRole};
use waterui::background::{Background, MaterialBackground};
use waterui::border::Border;
use waterui::component::focus::Focused;
use waterui::cursor::Cursor;
use waterui::drag_drop::{Draggable, DropDestination};
use waterui::filter::{
    Blur, Brightness, Contrast, Grayscale, HueRotation, Opacity, Saturation,
};
use waterui::gesture::GestureObserver;
use waterui::interaction::Hittable;
use waterui::metadata::context_menu::ContextMenu;
use waterui::metadata::secure::{HighDynamicRange, Secure, StandardDynamicRange};
use waterui::style::{Offset, Rotation, Scale, Shadow};
use waterui::widget::Divider;
use waterui_backend_core::ViewDispatcher;
use waterui_controls::button::ButtonConfig;
use waterui_core::dynamic::Dynamic;
use waterui_core::layout::{
    Layout, ProposalSize, Rect as LayoutRect, Size as LayoutSize, StretchAxis, SubView,
};
use waterui_core::event::{LifeCycleHook, OnEvent};
use waterui_core::metadata::MetadataKey;
use waterui_core::views::Views;
use waterui_core::{AnyView, Environment, IgnorableMetadata, Metadata, Native, Retain, Str, View};
use waterui_graphics::color::{Color, ResolvedColor};
use waterui_graphics::{
    AppliedFilter, FilterContext, FilterInput, FilterOutput, GradientType, ResolvedGradient,
    ResolvedGradientStop,
};
use waterui_layout::container::{FixedContainer, LazyContainer};
use waterui_layout::safe_area::IgnoreSafeArea;
use waterui_layout::scroll::ScrollView;
use waterui_layout::stack::Axis as StackAxis;
use waterui_layout::spacer::Spacer;
use waterui_shape::{ClipShape, PathCommand, ResolvedShape};
use waterui_text::font::FontWeight as TextFontWeight;
use waterui_text::styled::{Style as TextStyle, StyledStr};
use waterui_text::TextConfig;

/// Shared mutable state carried by the hydrolysis dispatcher.
pub struct HydroState {
    pub font_cx: parley::FontContext,
    pub layout_cx: parley::LayoutContext,
    frame_device: *const wgpu::Device,
    frame_queue: *const wgpu::Queue,
}

impl Default for HydroState {
    fn default() -> Self {
        Self {
            font_cx: parley::FontContext::new(),
            layout_cx: parley::LayoutContext::new(),
            frame_device: core::ptr::null(),
            frame_queue: core::ptr::null(),
        }
    }
}

impl HydroState {
    fn set_frame_resources(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.frame_device = device as *const _;
        self.frame_queue = queue as *const _;
    }

    fn clear_frame_resources(&mut self) {
        self.frame_device = core::ptr::null();
        self.frame_queue = core::ptr::null();
    }

    fn frame_resource_ptrs(&self) -> (*const wgpu::Device, *const wgpu::Queue) {
        if self.frame_device.is_null() || self.frame_queue.is_null() {
            panic!("hydrolysis frame resources are unavailable during AppliedFilter dispatch");
        }
        (self.frame_device, self.frame_queue)
    }
}

/// Render context passed to handlers.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext {
    renderer_ptr: *mut HydrolysisRenderer,
    pub transform: vello::kurbo::Affine,
    pub bounds: vello::kurbo::Rect,
}

impl RenderContext {
    pub(crate) fn with_renderer(renderer: &mut HydrolysisRenderer, bounds: vello::kurbo::Rect) -> Self {
        Self {
            renderer_ptr: renderer as *mut HydrolysisRenderer,
            transform: vello::kurbo::Affine::IDENTITY,
            bounds,
        }
    }

    /// # Safety
    /// The caller guarantees the render context belongs to an active render pass.
    pub unsafe fn renderer(&self) -> &mut HydrolysisRenderer {
        unsafe { &mut *self.renderer_ptr }
    }

    /// # Safety
    /// The caller guarantees the render context belongs to an active render pass.
    pub unsafe fn scene(&self) -> &mut vello::Scene {
        unsafe { &mut (*self.renderer_ptr).scene }
    }

    #[must_use]
    pub fn child(&self, transform: vello::kurbo::Affine, bounds: vello::kurbo::Rect) -> Self {
        Self {
            renderer_ptr: self.renderer_ptr,
            transform: self.transform * transform,
            bounds,
        }
    }
}

/// Core hydrolysis renderer state.
pub struct HydrolysisRenderer {
    dispatcher: ViewDispatcher<HydroState, RenderContext, ()>,
    vello_renderer: vello::Renderer,
    scene: vello::Scene,
    active_filter_images: Vec<vello::peniko::ImageData>,
}

#[derive(Debug, Clone, Copy)]
struct HydroSubview {
    stretch_axis: StretchAxis,
    intrinsic: LayoutSize,
}

impl HydroSubview {
    fn from_view(view: &AnyView) -> Self {
        Self {
            stretch_axis: view.stretch_axis(),
            intrinsic: estimate_intrinsic_size(view),
        }
    }
}

impl SubView for HydroSubview {
    fn size_that_fits(&self, proposal: ProposalSize) -> LayoutSize {
        let width = if self.stretch_axis.stretches_horizontal() {
            proposal.width.unwrap_or(self.intrinsic.width)
        } else {
            proposal
                .width
                .map_or(self.intrinsic.width, |value| self.intrinsic.width.min(value))
        };

        let height = if self.stretch_axis.stretches_vertical() {
            proposal.height.unwrap_or(self.intrinsic.height)
        } else {
            proposal
                .height
                .map_or(self.intrinsic.height, |value| self.intrinsic.height.min(value))
        };

        LayoutSize::new(width, height)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch_axis
    }

    fn priority(&self) -> i32 {
        0
    }
}

impl core::fmt::Debug for HydroState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroState").finish_non_exhaustive()
    }
}

impl core::fmt::Debug for HydrolysisRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydrolysisRenderer")
            .field("dispatcher", &self.dispatcher)
            .finish_non_exhaustive()
    }
}

impl HydrolysisRenderer {
    #[must_use]
    pub fn new(device: &wgpu::Device) -> Self {
        Self::new_with_options(
            device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::area_only(),
                num_init_threads: std::num::NonZeroUsize::new(1),
                pipeline_cache: None,
            },
        )
    }

    #[must_use]
    pub fn new_with_options(device: &wgpu::Device, options: vello::RendererOptions) -> Self {
        let mut dispatcher = ViewDispatcher::with_state(HydroState::default());
        Self::register_core_handlers(&mut dispatcher);

        let vello_renderer =
            vello::Renderer::new(device, options).expect("failed to create hydrolysis renderer");
        Self {
            dispatcher,
            vello_renderer,
            scene: vello::Scene::new(),
            active_filter_images: Vec::new(),
        }
    }

    fn register_core_handlers(dispatcher: &mut ViewDispatcher<HydroState, RenderContext, ()>) {
        dispatcher.register::<Native<()>>(|_state, _ctx, _unit, _env| ());
        dispatcher.register::<Native<Spacer>>(|_state, _ctx, _spacer, _env| ());
        dispatcher.register::<Str>(Self::render_str);
        dispatcher.register::<Native<TextConfig>>(Self::render_text_config);

        dispatcher.register::<Native<FixedContainer>>(Self::render_fixed_container);
        dispatcher.register::<Native<LazyContainer>>(Self::render_lazy_container);
        dispatcher.register::<Native<ScrollView>>(Self::render_scroll_view);
        dispatcher.register::<Native<ButtonConfig>>(Self::render_button);
        dispatcher.register::<Native<Dynamic>>(Self::render_dynamic);
        dispatcher.register::<Native<ResolvedColor>>(Self::render_resolved_color);
        dispatcher.register::<Native<ResolvedGradient>>(Self::render_resolved_gradient);
        dispatcher.register::<Native<ResolvedShape>>(Self::render_resolved_shape);
        dispatcher.register::<Divider>(Self::render_divider);

        dispatcher.register::<Metadata<Environment>>(Self::render_environment_metadata);
        dispatcher.register::<Metadata<Retain>>(Self::render_retain_metadata);
        dispatcher.register::<Metadata<Opacity>>(Self::render_opacity_metadata);
        dispatcher.register::<Metadata<AppliedFilter>>(Self::render_applied_filter_metadata);
        dispatcher.register::<Metadata<Scale>>(Self::render_scale_metadata);
        dispatcher.register::<Metadata<Rotation>>(Self::render_rotation_metadata);
        dispatcher.register::<Metadata<Offset>>(Self::render_offset_metadata);
        dispatcher.register::<Metadata<ClipShape>>(Self::render_clip_shape_metadata);
        dispatcher.register::<Metadata<Border>>(Self::render_border_metadata);
        dispatcher.register::<Metadata<Shadow>>(Self::render_shadow_metadata);

        Self::register_passthrough_metadata::<Secure>(dispatcher);
        Self::register_passthrough_metadata::<StandardDynamicRange>(dispatcher);
        Self::register_passthrough_metadata::<HighDynamicRange>(dispatcher);
        Self::register_passthrough_metadata::<GestureObserver>(dispatcher);
        Self::register_passthrough_metadata::<LifeCycleHook>(dispatcher);
        Self::register_passthrough_metadata::<OnEvent>(dispatcher);
        Self::register_passthrough_metadata::<Cursor>(dispatcher);
        Self::register_passthrough_metadata::<Focused>(dispatcher);
        Self::register_passthrough_metadata::<IgnoreSafeArea>(dispatcher);
        Self::register_passthrough_metadata::<ContextMenu>(dispatcher);
        Self::register_passthrough_metadata::<Hittable>(dispatcher);
        Self::register_passthrough_metadata::<Draggable>(dispatcher);
        Self::register_passthrough_metadata::<DropDestination>(dispatcher);
        Self::register_passthrough_metadata::<Blur>(dispatcher);
        Self::register_passthrough_metadata::<Brightness>(dispatcher);
        Self::register_passthrough_metadata::<Contrast>(dispatcher);
        Self::register_passthrough_metadata::<Saturation>(dispatcher);
        Self::register_passthrough_metadata::<Grayscale>(dispatcher);
        Self::register_passthrough_metadata::<HueRotation>(dispatcher);
        Self::register_passthrough_metadata::<Background>(dispatcher);

        Self::register_passthrough_ignorable_metadata::<MaterialBackground>(dispatcher);
        Self::register_passthrough_ignorable_metadata::<AccessibilityLabel>(dispatcher);
        Self::register_passthrough_ignorable_metadata::<AccessibilityRole>(dispatcher);
    }

    fn register_passthrough_metadata<T: MetadataKey>(
        dispatcher: &mut ViewDispatcher<HydroState, RenderContext, ()>,
    ) {
        dispatcher.register::<Metadata<T>>(Self::render_passthrough_metadata::<T>);
    }

    fn register_passthrough_ignorable_metadata<T: MetadataKey>(
        dispatcher: &mut ViewDispatcher<HydroState, RenderContext, ()>,
    ) {
        dispatcher.register::<IgnorableMetadata<T>>(Self::render_passthrough_ignorable_metadata::<T>);
    }

    fn dispatch_any(ctx: RenderContext, env: &Environment, content: AnyView) {
        let renderer = unsafe { ctx.renderer() };
        renderer.dispatcher.dispatch(content, env, ctx);
    }

    fn render_layout_container(
        ctx: RenderContext,
        layout: Box<dyn Layout>,
        children: Vec<AnyView>,
        env: &Environment,
    ) {
        let subviews: Vec<HydroSubview> = children.iter().map(HydroSubview::from_view).collect();
        let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();

        let proposal =
            ProposalSize::new(Some(ctx.bounds.width() as f32), Some(ctx.bounds.height() as f32));
        let _ = layout.size_that_fits(proposal, &refs);
        let bounds = LayoutRect::from_size(LayoutSize::new(
            ctx.bounds.width() as f32,
            ctx.bounds.height() as f32,
        ));
        let child_rects = layout.place(bounds, &refs);

        for (child, rect) in children.into_iter().zip(child_rects) {
            let child_transform =
                vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y())));
            let child_bounds =
                vello::kurbo::Rect::new(0.0, 0.0, f64::from(rect.width()), f64::from(rect.height()));
            Self::dispatch_any(ctx.child(child_transform, child_bounds), env, child);
        }
    }

    fn render_fixed_container(
        _state: &mut HydroState,
        ctx: RenderContext,
        container: Native<FixedContainer>,
        env: &Environment,
    ) {
        let (layout, children) = container.into_inner().into_inner();
        Self::render_layout_container(ctx, layout, children, env);
    }

    fn render_lazy_container(
        _state: &mut HydroState,
        ctx: RenderContext,
        container: Native<LazyContainer>,
        env: &Environment,
    ) {
        let (layout, children) = container.into_inner().into_inner();
        let count = children.len().get();
        let mut materialized = Vec::with_capacity(count);
        for index in 0..count {
            let view = children.get_view(index).unwrap_or_else(|| {
                panic!("LazyContainer failed to materialize child at index {index}")
            });
            materialized.push(view);
        }
        Self::render_layout_container(ctx, layout, materialized, env);
    }

    fn render_scroll_view(
        _state: &mut HydroState,
        ctx: RenderContext,
        scroll: Native<ScrollView>,
        env: &Environment,
    ) {
        let (_axis, content) = scroll.into_inner().into_inner();
        let scene = unsafe { ctx.scene() };
        scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            1.0,
            ctx.transform,
            &ctx.bounds,
        );
        Self::dispatch_any(ctx, env, content);
        scene.pop_layer();
    }

    fn render_divider(_state: &mut HydroState, ctx: RenderContext, _divider: Divider, env: &Environment) {
        let vertical = matches!(env.get::<StackAxis>(), Some(StackAxis::Horizontal));
        let rect = if vertical {
            vello::kurbo::Rect::new(ctx.bounds.x0, ctx.bounds.y0, ctx.bounds.x0 + 1.0, ctx.bounds.y1)
        } else {
            vello::kurbo::Rect::new(ctx.bounds.x0, ctx.bounds.y0, ctx.bounds.x1, ctx.bounds.y0 + 1.0)
        };

        let scene = unsafe { ctx.scene() };
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            vello::peniko::Color::new([0.75, 0.75, 0.75, 1.0]),
            None,
            &rect,
        );
    }

    fn render_str(state: &mut HydroState, ctx: RenderContext, text: Str, env: &Environment) {
        Self::render_styled_text(state, ctx, StyledStr::plain(text), env);
    }

    fn render_text_config(
        state: &mut HydroState,
        ctx: RenderContext,
        text: Native<TextConfig>,
        env: &Environment,
    ) {
        let styled = text.into_inner().content.get();
        Self::render_styled_text(state, ctx, styled, env);
    }

    fn render_styled_text(
        state: &mut HydroState,
        ctx: RenderContext,
        styled: StyledStr,
        env: &Environment,
    ) {
        let mut plain = String::new();
        let mut spans = Vec::with_capacity(styled.chunks().len());
        for (chunk, style) in styled.chunks() {
            let start = plain.len();
            plain.push_str(chunk.as_str());
            let end = plain.len();
            spans.push((start..end, style.clone()));
        }
        if plain.is_empty() {
            return;
        }

        let mut family_storage = Vec::new();
        let default_font = waterui_text::font::Font::default().resolve(env).get();
        let default_brush = resolved_color_to_rgba8(Color::srgb(0, 0, 0).resolve(env).get());
        let mut builder = state
            .layout_cx
            .ranged_builder(&mut state.font_cx, &plain, 1.0, true);
        builder.push_default(parley::StyleProperty::Brush(default_brush));
        builder.push_default(parley::StyleProperty::FontSize(default_font.size));
        builder.push_default(parley::StyleProperty::FontWeight(parley_font_weight(
            default_font.weight,
        )));
        if let Some(family) = default_font.family {
            family_storage.push(family.to_string());
            let family_name = family_storage
                .last()
                .expect("default font family storage must contain the pushed value");
            builder.push_default(parley::StyleProperty::FontStack(parley::FontStack::Single(
                parley::FontFamily::Named(Cow::Borrowed(family_name.as_str())),
            )));
        }

        for (range, style) in spans {
            Self::push_text_style(&mut builder, &mut family_storage, style, range, env);
        }

        let mut layout = builder.build(&plain);
        let max_width = Some(ctx.bounds.width() as f32);
        layout.break_all_lines(max_width);
        layout.align(
            max_width,
            parley::Alignment::Start,
            parley::AlignmentOptions::default(),
        );

        let text_transform =
            ctx.transform * vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0));
        let scene = unsafe { ctx.scene() };
        for line in layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let style = glyph_run.style();
                    let brush = rgba8_to_peniko(style.brush);
                    let normalized_coords: Vec<vello::NormalizedCoord> =
                        run.normalized_coords().to_vec();

                    let mut run_x = glyph_run.offset();
                    let run_y = glyph_run.baseline();
                    let glyphs = glyph_run.glyphs().map(move |glyph| {
                        let x = run_x + glyph.x;
                        let y = run_y - glyph.y;
                        run_x += glyph.advance;
                        vello::Glyph {
                            id: glyph.id,
                            x,
                            y,
                        }
                    });

                    scene
                        .draw_glyphs(run.font())
                        .brush(brush)
                        .transform(text_transform)
                        .font_size(run.font_size())
                        .normalized_coords(&normalized_coords)
                        .draw(vello::peniko::Fill::NonZero, glyphs);
                }
            }
        }
    }

    fn push_text_style(
        builder: &mut parley::RangedBuilder<'_, [u8; 4]>,
        family_storage: &mut Vec<String>,
        style: TextStyle,
        range: std::ops::Range<usize>,
        env: &Environment,
    ) {
        let resolved_font = style.font.resolve(env).get();
        builder.push(parley::StyleProperty::FontSize(resolved_font.size), range.clone());
        builder.push(
            parley::StyleProperty::FontWeight(parley_font_weight(resolved_font.weight)),
            range.clone(),
        );
        if let Some(family) = resolved_font.family {
            family_storage.push(family.to_string());
            let family_name = family_storage
                .last()
                .expect("font family storage must contain the pushed value");
            builder.push(
                parley::StyleProperty::FontStack(parley::FontStack::Single(
                    parley::FontFamily::Named(Cow::Borrowed(family_name.as_str())),
                )),
                range.clone(),
            );
        }
        builder.push(
            parley::StyleProperty::FontStyle(if style.italic {
                parley::FontStyle::Italic
            } else {
                parley::FontStyle::Normal
            }),
            range.clone(),
        );
        builder.push(parley::StyleProperty::Underline(style.underline), range.clone());
        builder.push(
            parley::StyleProperty::Strikethrough(style.strikethrough),
            range.clone(),
        );
        if let Some(color) = style.foreground {
            builder.push(
                parley::StyleProperty::Brush(resolved_color_to_rgba8(color.resolve(env).get())),
                range,
            );
        }
    }

    fn render_button(
        _state: &mut HydroState,
        ctx: RenderContext,
        button: Native<ButtonConfig>,
        env: &Environment,
    ) {
        let button = button.into_inner();
        Self::dispatch_any(ctx, env, button.label);
    }

    fn render_dynamic(
        _state: &mut HydroState,
        ctx: RenderContext,
        dynamic: Native<Dynamic>,
        env: &Environment,
    ) {
        let current = Rc::new(RefCell::new(None::<AnyView>));
        dynamic.into_inner().connect({
            let current = Rc::clone(&current);
            move |update| {
                let next = update.into_value();
                let mut slot = current.borrow_mut();
                if slot.is_some() {
                    panic!("hydrolysis Dynamic update after initial dispatch is not implemented");
                }
                *slot = Some(next);
            }
        });
        let content = current
            .borrow_mut()
            .take()
            .expect("hydrolysis Dynamic must provide an initial view before dispatch");
        Self::dispatch_any(ctx, env, content);
    }

    fn render_resolved_color(
        _state: &mut HydroState,
        ctx: RenderContext,
        color: Native<ResolvedColor>,
        _env: &Environment,
    ) {
        let scene = unsafe { ctx.scene() };
        let brush = resolved_color_to_peniko(color.into_inner());
        scene.fill(vello::peniko::Fill::NonZero, ctx.transform, brush, None, &ctx.bounds);
    }

    fn render_resolved_gradient(
        _state: &mut HydroState,
        ctx: RenderContext,
        gradient: Native<ResolvedGradient>,
        _env: &Environment,
    ) {
        let scene = unsafe { ctx.scene() };
        let brush = resolved_gradient_to_brush(&gradient.into_inner(), ctx.bounds);
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            &brush,
            None,
            &ctx.bounds,
        );
    }

    fn render_resolved_shape(
        _state: &mut HydroState,
        ctx: RenderContext,
        shape: Native<ResolvedShape>,
        _env: &Environment,
    ) {
        let resolved = shape.into_inner();
        let path = resolved_shape_to_path(&resolved, ctx.bounds);
        let fill = resolved_color_to_peniko(resolved.fill);
        let scene = unsafe { ctx.scene() };
        scene.fill(vello::peniko::Fill::NonZero, ctx.transform, fill, None, &path);
    }

    fn render_environment_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Environment>,
        _env: &Environment,
    ) {
        let renderer = unsafe { ctx.renderer() };
        renderer
            .dispatcher
            .dispatch(metadata.content, &metadata.value, ctx);
    }

    fn render_retain_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Retain>,
        env: &Environment,
    ) {
        let retain = metadata.value;
        let renderer = unsafe { ctx.renderer() };
        renderer.dispatcher.dispatch(metadata.content, env, ctx);
        drop(retain);
    }

    fn render_opacity_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Opacity>,
        env: &Environment,
    ) {
        let alpha = metadata.value.value.get();
        let scene = unsafe { ctx.scene() };
        scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            alpha,
            ctx.transform,
            &ctx.bounds,
        );

        let renderer = unsafe { ctx.renderer() };
        renderer.dispatcher.dispatch(metadata.content, env, ctx);
        scene.pop_layer();
    }

    fn render_applied_filter_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<AppliedFilter>,
        env: &Environment,
    ) {
        let Metadata {
            content,
            value: mut filter,
        } = metadata;
        let renderer = unsafe { ctx.renderer() };
        let (device_ptr, queue_ptr) = renderer.state().frame_resource_ptrs();
        let device = unsafe { &*device_ptr };
        let queue = unsafe { &*queue_ptr };

        let width = (ctx.bounds.width().max(1.0).round()) as u32;
        let height = (ctx.bounds.height().max(1.0).round()) as u32;
        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let mut subtree_scene = vello::Scene::new();
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        renderer.dispatcher.dispatch(content, env, ctx);
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);

        let input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_applied_filter_input"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());
        renderer
            .vello_renderer
            .render_to_texture(
                device,
                queue,
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

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_applied_filter_output"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let filter_context = FilterContext {
            device,
            queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: None,
        };
        pollster::block_on(filter.setup(&filter_context));
        filter.sync_targets();

        let input = FilterInput {
            device,
            queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width,
            height,
        };
        let output = FilterOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width,
            height,
        };
        let _ = filter.render(&input, &output);

        let image = renderer.vello_renderer.register_texture(output_texture);
        renderer.active_filter_images.push(image.clone());
        let image_transform = vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))
            * vello::kurbo::Affine::scale_non_uniform(
                ctx.bounds.width() / f64::from(width),
                ctx.bounds.height() / f64::from(height),
            );
        let scene = unsafe { ctx.scene() };
        scene.draw_image(
            &vello::peniko::ImageBrush::new(image),
            ctx.transform * image_transform,
        );
    }

    fn render_scale_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Scale>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let center = anchor_point(ctx.bounds, value.anchor);
        let transform = vello::kurbo::Affine::translate((center.x, center.y))
            * vello::kurbo::Affine::scale_non_uniform(
                f64::from(value.x.get()),
                f64::from(value.y.get()),
            )
            * vello::kurbo::Affine::translate((-center.x, -center.y));
        Self::dispatch_any(ctx.child(transform, ctx.bounds), env, content);
    }

    fn render_rotation_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Rotation>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let center = anchor_point(ctx.bounds, value.anchor);
        let radians = f64::from(value.angle.get()).to_radians();
        let transform = vello::kurbo::Affine::translate((center.x, center.y))
            * vello::kurbo::Affine::rotate(radians)
            * vello::kurbo::Affine::translate((-center.x, -center.y));
        Self::dispatch_any(ctx.child(transform, ctx.bounds), env, content);
    }

    fn render_offset_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Offset>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let transform =
            vello::kurbo::Affine::translate((f64::from(value.x.get()), f64::from(value.y.get())));
        Self::dispatch_any(ctx.child(transform, ctx.bounds), env, content);
    }

    fn render_clip_shape_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<ClipShape>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let clip_path = path_commands_to_path(value.commands(), ctx.bounds);
        let scene = unsafe { ctx.scene() };
        scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            1.0,
            ctx.transform,
            &clip_path,
        );
        Self::dispatch_any(ctx, env, content);
        scene.pop_layer();
    }

    fn render_border_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Border>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let border = value;
        Self::dispatch_any(ctx, env, content);

        if border.width <= 0.0 {
            return;
        }

        let scene = unsafe { ctx.scene() };
        let brush = resolved_color_to_peniko(border.color.resolve(env).get());
        let width = f64::from(border.width);

        if border.edges.all() && border.corner_radius > 0.0 {
            let rounded = vello::kurbo::RoundedRect::from_rect(
                ctx.bounds,
                f64::from(border.corner_radius),
            );
            let stroke = vello::kurbo::Stroke::new(width);
            scene.stroke(&stroke, ctx.transform, brush, None, &rounded);
            return;
        }

        if border.edges.top {
            let top = vello::kurbo::Rect::new(ctx.bounds.x0, ctx.bounds.y0, ctx.bounds.x1, ctx.bounds.y0 + width);
            scene.fill(vello::peniko::Fill::NonZero, ctx.transform, brush, None, &top);
        }
        if border.edges.bottom {
            let bottom =
                vello::kurbo::Rect::new(ctx.bounds.x0, ctx.bounds.y1 - width, ctx.bounds.x1, ctx.bounds.y1);
            scene.fill(vello::peniko::Fill::NonZero, ctx.transform, brush, None, &bottom);
        }
        if border.edges.leading {
            let leading =
                vello::kurbo::Rect::new(ctx.bounds.x0, ctx.bounds.y0, ctx.bounds.x0 + width, ctx.bounds.y1);
            scene.fill(vello::peniko::Fill::NonZero, ctx.transform, brush, None, &leading);
        }
        if border.edges.trailing {
            let trailing =
                vello::kurbo::Rect::new(ctx.bounds.x1 - width, ctx.bounds.y0, ctx.bounds.x1, ctx.bounds.y1);
            scene.fill(vello::peniko::Fill::NonZero, ctx.transform, brush, None, &trailing);
        }
    }

    fn render_shadow_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Shadow>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let shadow = value;
        let spread = f64::from(shadow.radius.max(0.0));
        let offset_x = f64::from(shadow.offset.x);
        let offset_y = f64::from(shadow.offset.y);
        let shadow_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0 + offset_x - spread,
            ctx.bounds.y0 + offset_y - spread,
            ctx.bounds.x1 + offset_x + spread,
            ctx.bounds.y1 + offset_y + spread,
        );
        let shadow_color = resolved_color_to_peniko(shadow.color.resolve(env).get());

        let scene = unsafe { ctx.scene() };
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            shadow_color,
            None,
            &shadow_rect,
        );
        Self::dispatch_any(ctx, env, content);
    }

    fn render_passthrough_metadata<T: MetadataKey>(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<T>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let _ = value;
        Self::dispatch_any(ctx, env, content);
    }

    fn render_passthrough_ignorable_metadata<T: MetadataKey>(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: IgnorableMetadata<T>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let _ = value;
        Self::dispatch_any(ctx, env, content);
    }

    #[must_use]
    pub fn state(&self) -> &HydroState {
        self.dispatcher.state()
    }

    pub fn state_mut(&mut self) -> &mut HydroState {
        self.dispatcher.state_mut()
    }

    #[must_use]
    pub fn scene(&self) -> &vello::Scene {
        &self.scene
    }

    pub fn reset_scene(&mut self) {
        for image in self.active_filter_images.drain(..) {
            self.vello_renderer.unregister_texture(image);
        }
        self.scene.reset();
    }

    pub fn scene_mut(&mut self) -> &mut vello::Scene {
        &mut self.scene
    }

    pub fn vello_renderer(&mut self) -> &mut vello::Renderer {
        &mut self.vello_renderer
    }

    pub fn dispatcher_mut(&mut self) -> &mut ViewDispatcher<HydroState, RenderContext, ()> {
        &mut self.dispatcher
    }

    pub fn set_frame_resources(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.dispatcher.state_mut().set_frame_resources(device, queue);
    }

    pub fn clear_frame_resources(&mut self) {
        self.dispatcher.state_mut().clear_frame_resources();
    }

    pub fn dispatch<V: View>(&mut self, view: V, env: &Environment, bounds: vello::kurbo::Rect) {
        let ctx = RenderContext::with_renderer(self, bounds);
        self.dispatcher.dispatch(view, env, ctx);
    }

    pub fn render_scene_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let params = vello::RenderParams {
            base_color: vello::peniko::Color::TRANSPARENT,
            width,
            height,
            antialiasing_method: vello::AaConfig::Area,
        };
        self.vello_renderer
            .render_to_texture(device, queue, &self.scene, target, &params)
            .expect("hydrolysis renderer: failed to render scene");
    }
}

fn estimate_intrinsic_size(view: &AnyView) -> LayoutSize {
    if let Some(text) = view.downcast_ref::<Str>() {
        return LayoutSize::new(text.len() as f32 * 8.0, 20.0);
    }

    if view.is::<Native<TextConfig>>() {
        return LayoutSize::new(120.0, 20.0);
    }

    if view.stretch_axis().stretches_any() {
        return LayoutSize::zero();
    }

    LayoutSize::new(44.0, 44.0)
}

fn resolved_color_to_peniko(color: ResolvedColor) -> vello::peniko::Color {
    let srgb = color.to_srgb_with_headroom();
    vello::peniko::Color::new([srgb.red, srgb.green, srgb.blue, color.opacity])
}

fn resolved_gradient_to_brush(
    gradient: &ResolvedGradient,
    bounds: vello::kurbo::Rect,
) -> vello::peniko::Brush {
    let mut stops: Vec<vello::peniko::ColorStop> = gradient
        .stops
        .iter()
        .map(to_peniko_stop)
        .collect();

    let brush = match gradient.gradient_type {
        GradientType::Linear => {
            let start = resolved_point_to_kurbo(gradient.start_point, bounds);
            let end = resolved_point_to_kurbo(gradient.end_point, bounds);
            vello::peniko::Gradient::new_linear(start, end).with_stops(&*stops)
        }
        GradientType::Radial => {
            let center = resolved_point_to_kurbo(gradient.start_point, bounds);
            let radius_scale = bounds.width().min(bounds.height()) as f32;
            let start_radius = gradient.start_value * radius_scale;
            let end_radius = gradient.end_value * radius_scale;
            vello::peniko::Gradient::new_two_point_radial(
                center,
                start_radius,
                center,
                end_radius,
            )
            .with_stops(&*stops)
        }
        GradientType::Angular => {
            let sweep = gradient.end_value - gradient.start_value;
            let sweep_fraction = f64::from(sweep) / TAU;
            if sweep_fraction < 1.0 {
                let last_color = stops
                    .last()
                    .expect("resolved gradient must contain at least one stop")
                    .color;
                for stop in &mut stops {
                    stop.offset = (f64::from(stop.offset) * sweep_fraction) as f32;
                }
                stops.push(vello::peniko::ColorStop {
                    offset: sweep_fraction as f32,
                    color: last_color,
                });
                stops.push(vello::peniko::ColorStop {
                    offset: 1.0,
                    color: last_color,
                });
            }
            let center = resolved_point_to_kurbo(gradient.start_point, bounds);
            vello::peniko::Gradient::new_sweep(center, gradient.start_value, 0.0)
                .with_stops(&*stops)
        }
        GradientType::Mesh => {
            panic!("resolved mesh gradient must not be dispatched through ResolvedGradient")
        }
    };

    vello::peniko::Brush::Gradient(brush)
}

fn resolved_point_to_kurbo(point: [f32; 2], bounds: vello::kurbo::Rect) -> vello::kurbo::Point {
    vello::kurbo::Point::new(
        f64::from(point[0]) * bounds.width(),
        f64::from(point[1]) * bounds.height(),
    )
}

fn to_peniko_stop(stop: &ResolvedGradientStop) -> vello::peniko::ColorStop {
    vello::peniko::ColorStop {
        offset: stop.position,
        color: resolved_color_to_peniko(stop.color).into(),
    }
}

fn resolved_shape_to_path(shape: &ResolvedShape, bounds: vello::kurbo::Rect) -> vello::kurbo::BezPath {
    path_commands_to_path(&shape.commands, bounds)
}

fn path_commands_to_path(commands: &[PathCommand], bounds: vello::kurbo::Rect) -> vello::kurbo::BezPath {
    let width = bounds.width();
    let height = bounds.height();
    let mut path = vello::kurbo::BezPath::new();
    let mut has_current = false;

    for command in commands {
        match command {
            PathCommand::MoveTo { x, y } => {
                path.move_to(vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height));
                has_current = true;
            }
            PathCommand::LineTo { x, y } => {
                if !has_current {
                    panic!("PathCommand::LineTo requires an active current point");
                }
                path.line_to(vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height));
            }
            PathCommand::QuadTo { cx, cy, x, y } => {
                if !has_current {
                    panic!("PathCommand::QuadTo requires an active current point");
                }
                path.quad_to(
                    vello::kurbo::Point::new(f64::from(*cx) * width, f64::from(*cy) * height),
                    vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height),
                );
            }
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                if !has_current {
                    panic!("PathCommand::CubicTo requires an active current point");
                }
                path.curve_to(
                    vello::kurbo::Point::new(f64::from(*c1x) * width, f64::from(*c1y) * height),
                    vello::kurbo::Point::new(f64::from(*c2x) * width, f64::from(*c2y) * height),
                    vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height),
                );
            }
            PathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => {
                let center_x = f64::from(*cx) * width;
                let center_y = f64::from(*cy) * height;
                let radius_x = f64::from(*rx) * width;
                let radius_y = f64::from(*ry) * height;
                let start = f64::from(*start);
                let step = f64::from(*sweep) / 32.0;

                let start_point = vello::kurbo::Point::new(
                    center_x + radius_x * start.cos(),
                    center_y + radius_y * start.sin(),
                );
                if has_current {
                    path.line_to(start_point);
                } else {
                    path.move_to(start_point);
                    has_current = true;
                }

                let mut angle = start;
                for _ in 0..32 {
                    angle += step;
                    path.line_to(vello::kurbo::Point::new(
                        center_x + radius_x * angle.cos(),
                        center_y + radius_y * angle.sin(),
                    ));
                }
            }
            PathCommand::Close => {
                path.close_path();
                has_current = false;
            }
        }
    }

    path
}

fn anchor_point(bounds: vello::kurbo::Rect, anchor: waterui::style::Anchor) -> vello::kurbo::Point {
    vello::kurbo::Point::new(
        bounds.x0 + bounds.width() * f64::from(anchor.x),
        bounds.y0 + bounds.height() * f64::from(anchor.y),
    )
}

fn resolved_color_to_rgba8(color: ResolvedColor) -> [u8; 4] {
    let srgb = color.to_srgb_with_headroom();
    [
        (srgb.red.clamp(0.0, 1.0) * 255.0).round() as u8,
        (srgb.green.clamp(0.0, 1.0) * 255.0).round() as u8,
        (srgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

fn rgba8_to_peniko(color: [u8; 4]) -> vello::peniko::Color {
    vello::peniko::Color::new([
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
        f32::from(color[3]) / 255.0,
    ])
}

fn parley_font_weight(weight: TextFontWeight) -> parley::FontWeight {
    let value = match weight {
        TextFontWeight::Thin => 100.0,
        TextFontWeight::UltraLight => 200.0,
        TextFontWeight::Light => 300.0,
        TextFontWeight::Normal => 400.0,
        TextFontWeight::Medium => 500.0,
        TextFontWeight::SemiBold => 600.0,
        TextFontWeight::Bold => 700.0,
        TextFontWeight::UltraBold => 800.0,
        TextFontWeight::Black => 900.0,
    };
    parley::FontWeight::new(value)
}
