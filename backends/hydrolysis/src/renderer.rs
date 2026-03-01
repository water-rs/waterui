use core::f64::consts::TAU;
use core::time::Duration;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

use nami::Signal;
use waterui::accessibility::{AccessibilityLabel, AccessibilityRole};
use waterui::animation::Animation;
use waterui::background::{Background, MaterialBackground};
use waterui::border::Border;
use waterui::component::focus::Focused;
use waterui::component::list::ListConfig;
use waterui::component::progress::{ProgressConfig, ProgressStyle};
use waterui::component::table::TableConfig;
use waterui::cursor::Cursor;
use waterui::drag_drop::{Draggable, DropDestination};
use waterui::filter::{Blur, Brightness, Contrast, Grayscale, HueRotation, Opacity, Saturation};
use waterui::gesture::{Gesture, GestureObserver, GesturePoint, TapEvent};
use waterui::interaction::Hittable;
use waterui::metadata::context_menu::ContextMenu;
use waterui::metadata::secure::{HighDynamicRange, Secure, StandardDynamicRange};
use waterui::navigation::tab::{TabPosition, Tabs};
use waterui::navigation::{
    CustomNavigationController, NavigationController, NavigationStack, NavigationView,
};
use waterui::style::{Offset, Rotation, Scale, Shadow};
use waterui::widget::Divider;
use waterui_backend_core::ViewDispatcher;
use waterui_controls::button::ButtonConfig;
use waterui_controls::slider::SliderConfig;
use waterui_controls::stepper::StepperConfig;
use waterui_controls::text_field::TextFieldConfig;
use waterui_controls::toggle::ToggleConfig;
use waterui_core::dynamic::Dynamic;
use waterui_core::event::{Event, LifeCycle, LifeCycleHook, OnEvent};
use waterui_core::layout::{
    Layout, ProposalSize, Rect as LayoutRect, Size as LayoutSize, StretchAxis, SubView,
};
use waterui_core::metadata::MetadataKey;
use waterui_core::views::Views;
use waterui_core::{AnyView, Environment, IgnorableMetadata, Metadata, Native, Retain, Str, View};
use waterui_form::picker::PickerConfig;
use waterui_form::secure::{Secure as FormSecure, SecureFieldConfig};
use waterui_graphics::color::{Color, ResolvedColor};
use waterui_graphics::view_effect::{EffectContext, EffectInput, EffectOutput, ViewEffectErased};
use waterui_graphics::{
    AppliedFilter, FilterContext, FilterInput, FilterOutput, GpuSurface, GradientType,
    OffscreenRenderConfig, OffscreenSize, ResolvedGradient, ResolvedGradientStop,
};
use waterui_icon::SystemIcon;
use waterui_layout::container::{FixedContainer, LazyContainer};
use waterui_layout::safe_area::IgnoreSafeArea;
use waterui_layout::scroll::Axis as ScrollAxis;
use waterui_layout::scroll::ScrollView;
use waterui_layout::spacer::Spacer;
use waterui_layout::stack::Axis as StackAxis;
use waterui_shape::{ClipShape, PathCommand, ResolvedShape};
use waterui_text::TextConfig;
use waterui_text::font::FontWeight as TextFontWeight;
use waterui_text::styled::{Style as TextStyle, StyledStr};

use crate::animation::AnimationController;
use crate::platform::{KeyCode, Modifiers, PointerButton, TextInputPurpose, TextInputState};
use crate::scroll::{ScrollController, ScrollMetrics};

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
    pub(crate) fn with_renderer(
        renderer: &mut HydrolysisRenderer,
        bounds: vello::kurbo::Rect,
    ) -> Self {
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
    surface_blit: Option<SurfaceBlitState>,
    active_filter_images: Vec<vello::peniko::ImageData>,
    pointer_targets: Vec<PointerTarget>,
    hover_targets: Vec<HoverTarget>,
    text_input_targets: Vec<TextInputTarget>,
    scroll_targets: Vec<ScrollTarget>,
    focused_text_input: Cell<Option<usize>>,
    ime_preedit: Option<String>,
    lifecycle_disappear_previous: BTreeMap<usize, DeferredLifeCycleHook>,
    lifecycle_disappear_current: BTreeMap<usize, DeferredLifeCycleHook>,
    lifecycle_disappear_slot: usize,
    rebuild_requested: Rc<Cell<bool>>,
    animation_controller: AnimationController,
    scroll_controller: ScrollController,
    current_frame_retain: Vec<Retain>,
    previous_frame_retain: Vec<Retain>,
}

#[derive(Debug, Clone, Copy)]
struct HydroSubview {
    stretch_axis: StretchAxis,
    intrinsic: LayoutSize,
}

struct PointerTarget {
    bounds: vello::kurbo::Rect,
    action: Box<dyn FnMut(vello::kurbo::Point, &Environment) -> bool>,
}

struct HoverTarget {
    bounds: vello::kurbo::Rect,
    hovering: bool,
    on_enter: Option<Box<dyn FnMut(&Environment) -> bool>>,
    on_exit: Option<Box<dyn FnMut(&Environment) -> bool>>,
}

enum TextInputCommand {
    Insert(String),
    Backspace,
}

struct TextInputTarget {
    bounds: vello::kurbo::Rect,
    purpose: TextInputPurpose,
    action: Box<dyn FnMut(TextInputCommand) -> bool>,
}

struct ScrollTarget {
    bounds: vello::kurbo::Rect,
    action: Box<dyn FnMut(f32, f32) -> bool>,
}

struct DeferredLifeCycleHook {
    env: Environment,
    hook: LifeCycleHook,
}

struct SurfaceBlitState {
    target_format: wgpu::TextureFormat,
    size: (u32, u32),
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    blitter: wgpu::util::TextureBlitter,
}

struct HydroNavigationController;

impl DeferredLifeCycleHook {
    fn new(hook: LifeCycleHook, env: Environment) -> Self {
        Self { env, hook }
    }

    fn call(self) {
        self.hook.handle(&self.env);
    }
}

impl CustomNavigationController for HydroNavigationController {
    fn push(&mut self, _content: NavigationView) {
        panic!("hydrolysis NavigationStack push/pop state is not implemented yet");
    }

    fn pop(&mut self) {
        panic!("hydrolysis NavigationStack push/pop state is not implemented yet");
    }
}

impl HydroSubview {
    fn from_view(view: &AnyView, state: &mut HydroState, env: &Environment) -> Self {
        Self {
            stretch_axis: view.stretch_axis(),
            intrinsic: estimate_intrinsic_size(view, state, env),
        }
    }
}

impl SubView for HydroSubview {
    fn size_that_fits(&self, proposal: ProposalSize) -> LayoutSize {
        let width = if self.stretch_axis.stretches_horizontal() {
            proposal.width.unwrap_or(self.intrinsic.width)
        } else {
            proposal.width.map_or(self.intrinsic.width, |value| {
                self.intrinsic.width.min(value)
            })
        };

        let height = if self.stretch_axis.stretches_vertical() {
            proposal.height.unwrap_or(self.intrinsic.height)
        } else {
            proposal.height.map_or(self.intrinsic.height, |value| {
                self.intrinsic.height.min(value)
            })
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
            surface_blit: None,
            active_filter_images: Vec::new(),
            pointer_targets: Vec::new(),
            hover_targets: Vec::new(),
            text_input_targets: Vec::new(),
            scroll_targets: Vec::new(),
            focused_text_input: Cell::new(None),
            ime_preedit: None,
            lifecycle_disappear_previous: BTreeMap::new(),
            lifecycle_disappear_current: BTreeMap::new(),
            lifecycle_disappear_slot: 0,
            rebuild_requested: Rc::new(Cell::new(false)),
            animation_controller: AnimationController::default(),
            scroll_controller: ScrollController::default(),
            current_frame_retain: Vec::new(),
            previous_frame_retain: Vec::new(),
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
        dispatcher.register::<Native<NavigationView>>(Self::render_navigation_view);
        dispatcher.register::<Native<NavigationStack<(), ()>>>(Self::render_navigation_stack);
        dispatcher.register::<Native<Tabs>>(Self::render_tabs);
        dispatcher.register::<Native<ListConfig>>(Self::render_list);
        dispatcher.register::<Native<TableConfig>>(Self::render_table);
        dispatcher.register::<Native<ButtonConfig>>(Self::render_button);
        dispatcher.register::<Native<ToggleConfig>>(Self::render_toggle);
        dispatcher.register::<Native<SliderConfig>>(Self::render_slider);
        dispatcher.register::<Native<StepperConfig>>(Self::render_stepper);
        dispatcher.register::<Native<ProgressConfig>>(Self::render_progress);
        dispatcher.register::<Native<TextFieldConfig>>(Self::render_text_field);
        dispatcher.register::<Native<SecureFieldConfig>>(Self::render_secure_field);
        dispatcher.register::<Native<PickerConfig>>(Self::render_picker);
        dispatcher.register::<Native<Dynamic>>(Self::render_dynamic);
        dispatcher.register::<Native<SystemIcon>>(Self::render_system_icon);
        dispatcher.register::<Native<GpuSurface>>(Self::render_gpu_surface);
        dispatcher.register::<Native<ViewEffectErased>>(Self::render_view_effect);
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
        dispatcher.register::<Metadata<Focused>>(Self::render_focused_metadata);
        dispatcher.register::<Metadata<Hittable>>(Self::render_hittable_metadata);
        dispatcher.register::<Metadata<GestureObserver>>(Self::render_gesture_observer_metadata);
        dispatcher.register::<Metadata<LifeCycleHook>>(Self::render_lifecycle_hook_metadata);
        dispatcher.register::<Metadata<OnEvent>>(Self::render_on_event_metadata);

        Self::register_passthrough_metadata::<Secure>(dispatcher);
        Self::register_passthrough_metadata::<StandardDynamicRange>(dispatcher);
        Self::register_passthrough_metadata::<HighDynamicRange>(dispatcher);
        Self::register_passthrough_metadata::<Cursor>(dispatcher);
        Self::register_passthrough_metadata::<IgnoreSafeArea>(dispatcher);
        Self::register_passthrough_metadata::<ContextMenu>(dispatcher);
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
        dispatcher
            .register::<IgnorableMetadata<T>>(Self::render_passthrough_ignorable_metadata::<T>);
    }

    fn set_focused_text_input(&mut self, focused: Option<usize>) -> bool {
        if self.focused_text_input.get() == focused {
            return false;
        }
        self.focused_text_input.set(focused);
        self.ime_preedit = None;
        true
    }

    fn dispatch_any(ctx: RenderContext, env: &Environment, content: AnyView) {
        let renderer = unsafe { ctx.renderer() };
        renderer.dispatcher.dispatch(content, env, ctx);
    }

    fn dispatch_in_rect(
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
        rect: vello::kurbo::Rect,
    ) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let child_transform = vello::kurbo::Affine::translate((rect.x0, rect.y0));
        let child_bounds = vello::kurbo::Rect::new(0.0, 0.0, rect.width(), rect.height());
        Self::dispatch_any(ctx.child(child_transform, child_bounds), env, content);
    }

    fn render_subtree_scene(
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) -> vello::Scene {
        let renderer = unsafe { ctx.renderer() };
        let mut subtree_scene = vello::Scene::new();
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        renderer.dispatcher.dispatch(content, env, ctx);
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        subtree_scene
    }

    fn watch_signal<S>(&mut self, signal: &S)
    where
        S: Signal + Clone + 'static,
    {
        let rebuild_requested = Rc::clone(&self.rebuild_requested);
        let guard = signal.watch(move |_| rebuild_requested.set(true));
        self.current_frame_retain.push(Retain::new(guard));
    }

    fn read_signal<S>(&mut self, signal: &S) -> S::Output
    where
        S: Signal + Clone + 'static,
    {
        self.watch_signal(signal);
        signal.get()
    }

    fn resolve_animated_scalar<S>(&mut self, signal: &S) -> f32
    where
        S: Signal<Output = f32> + Clone + 'static,
    {
        let now = Instant::now();
        let handle = self.animation_controller.bind_scalar(signal.get());
        let watcher_handle = handle.clone();
        let rebuild_requested = Rc::clone(&self.rebuild_requested);
        let guard = signal.watch(move |update| {
            watcher_handle.apply_update_from_context(update, Instant::now());
            rebuild_requested.set(true);
        });
        self.current_frame_retain.push(Retain::new(guard));
        handle.sample(now)
    }

    fn resolve_toggle_progress<S>(&mut self, signal: &S) -> f32
    where
        S: Signal<Output = bool> + Clone + 'static,
    {
        let now = Instant::now();
        let handle = self
            .animation_controller
            .bind_scalar(if signal.get() { 1.0 } else { 0.0 });
        let watcher_handle = handle.clone();
        let rebuild_requested = Rc::clone(&self.rebuild_requested);
        let default_animation = Animation::ease_in_out(Duration::from_millis(180));
        let guard = signal.watch(move |update| {
            let target = if *update.value() { 1.0 } else { 0.0 };
            let animation = update
                .metadata()
                .try_get::<Animation>()
                .unwrap_or_else(|| default_animation.clone());
            watcher_handle.apply_target(target, Some(animation), Instant::now());
            rebuild_requested.set(true);
        });
        self.current_frame_retain.push(Retain::new(guard));
        handle.sample(now).clamp(0.0, 1.0)
    }

    fn render_layout_container(
        state: &mut HydroState,
        ctx: RenderContext,
        layout: Box<dyn Layout>,
        children: Vec<AnyView>,
        env: &Environment,
    ) {
        let mut subviews = Vec::with_capacity(children.len());
        for child in &children {
            subviews.push(HydroSubview::from_view(child, state, env));
        }
        let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();

        let proposal = ProposalSize::new(
            Some(ctx.bounds.width() as f32),
            Some(ctx.bounds.height() as f32),
        );
        let _ = layout.size_that_fits(proposal, &refs);
        let bounds = LayoutRect::from_size(LayoutSize::new(
            ctx.bounds.width() as f32,
            ctx.bounds.height() as f32,
        ));
        let child_rects = layout.place(bounds, &refs);

        for (child, rect) in children.into_iter().zip(child_rects) {
            let child_transform =
                vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y())));
            let child_bounds = vello::kurbo::Rect::new(
                0.0,
                0.0,
                f64::from(rect.width()),
                f64::from(rect.height()),
            );
            Self::dispatch_any(ctx.child(child_transform, child_bounds), env, child);
        }
    }

    fn render_fixed_container(
        state: &mut HydroState,
        ctx: RenderContext,
        container: Native<FixedContainer>,
        env: &Environment,
    ) {
        let (layout, children) = container.into_inner().into_inner();
        Self::render_layout_container(state, ctx, layout, children, env);
    }

    fn render_lazy_container(
        state: &mut HydroState,
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
        Self::render_layout_container(state, ctx, layout, materialized, env);
    }

    fn render_scroll_view(
        state: &mut HydroState,
        ctx: RenderContext,
        scroll: Native<ScrollView>,
        env: &Environment,
    ) {
        let (axis, content) = scroll.into_inner().into_inner();
        let viewport = ctx.bounds;
        let intrinsic = estimate_intrinsic_size(&content, state, env);
        let (content_width, content_height) = match axis {
            ScrollAxis::Horizontal => (
                f64::from(intrinsic.width).max(viewport.width()),
                viewport.height(),
            ),
            ScrollAxis::Vertical => (
                viewport.width(),
                f64::from(intrinsic.height).max(viewport.height()),
            ),
            ScrollAxis::All => (
                f64::from(intrinsic.width).max(viewport.width()),
                f64::from(intrinsic.height).max(viewport.height()),
            ),
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        };

        let handle = {
            let renderer = unsafe { ctx.renderer() };
            renderer.scroll_controller.bind(
                axis,
                viewport.width(),
                viewport.height(),
                content_width,
                content_height,
            )
        };
        let metrics = handle.metrics();

        let content_transform =
            vello::kurbo::Affine::translate((-metrics.offset_x, -metrics.offset_y));
        let content_bounds = vello::kurbo::Rect::new(0.0, 0.0, content_width, content_height);
        let scene = unsafe { ctx.scene() };
        scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            1.0,
            ctx.transform,
            &viewport,
        );
        Self::dispatch_any(ctx.child(content_transform, content_bounds), env, content);
        scene.pop_layer();

        {
            let renderer = unsafe { ctx.renderer() };
            let target_handle = handle.clone();
            renderer.register_scroll_target(
                transformed_rect(ctx.transform, viewport),
                move |dx, dy| target_handle.apply_scroll_delta(dx, dy),
            );
        }

        Self::draw_scroll_indicators(scene, ctx.transform, viewport, metrics, axis);
    }

    fn render_navigation_view(
        _state: &mut HydroState,
        ctx: RenderContext,
        navigation: Native<NavigationView>,
        env: &Environment,
    ) {
        let navigation = navigation.into_inner();
        let NavigationView { bar, content } = navigation;
        let bar_hidden = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&bar.hidden)
        };
        let bar_height = if bar_hidden {
            0.0
        } else {
            match bar.display_mode {
                waterui::navigation::NavigationTitleDisplayMode::Automatic => 52.0,
                waterui::navigation::NavigationTitleDisplayMode::Inline => 44.0,
                waterui::navigation::NavigationTitleDisplayMode::Large => 64.0,
            }
        };

        if bar_height > 0.0 {
            let bar_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
            );
            let bar_color = {
                let renderer = unsafe { ctx.renderer() };
                let color = renderer.read_signal(&bar.color);
                resolved_color_to_peniko(color.resolve(env).get())
            };
            {
                let scene = unsafe { ctx.scene() };
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    bar_color,
                    None,
                    &bar_rect,
                );
                let separator = vello::kurbo::Rect::new(
                    bar_rect.x0,
                    (bar_rect.y1 - 1.0).max(bar_rect.y0),
                    bar_rect.x1,
                    bar_rect.y1,
                );
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.8, 0.8, 0.82, 1.0]),
                    None,
                    &separator,
                );
            }

            let title_height = if matches!(
                bar.display_mode,
                waterui::navigation::NavigationTitleDisplayMode::Large
            ) {
                32.0
            } else {
                24.0
            };
            let title_rect = vello::kurbo::Rect::new(
                bar_rect.x0 + 12.0,
                bar_rect.y1 - title_height - 8.0,
                bar_rect.x1 - 12.0,
                bar_rect.y1 - 8.0,
            );
            if title_rect.width() > 0.0 && title_rect.height() > 0.0 {
                Self::dispatch_in_rect(ctx, env, bar.title, title_rect);
            }
        }

        let content_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
            ctx.bounds.x1,
            ctx.bounds.y1,
        );
        if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
            Self::dispatch_in_rect(ctx, env, content, content_rect);
        }
    }

    fn render_navigation_stack(
        _state: &mut HydroState,
        ctx: RenderContext,
        stack: Native<NavigationStack<(), ()>>,
        env: &Environment,
    ) {
        let root = stack.into_inner().into_inner();
        let mut local_env = env.clone();
        local_env.insert(NavigationController::new(HydroNavigationController));
        Self::dispatch_any(ctx, &local_env, root);
    }

    fn render_tabs(
        _state: &mut HydroState,
        ctx: RenderContext,
        tabs: Native<Tabs>,
        env: &Environment,
    ) {
        let tabs = tabs.into_inner();
        if tabs.tabs.is_empty() {
            panic!("hydrolysis Tabs requires at least one tab");
        }

        let tab_count = tabs.tabs.len();
        let position = tabs.position;
        let selection = tabs.selection;
        let selected_id = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&selection)
        };
        let selected_index = tabs
            .tabs
            .iter()
            .position(|tab| tab.label.tag == selected_id)
            .unwrap_or(0);

        if tabs.tabs[selected_index].label.tag != selected_id {
            selection.set(tabs.tabs[selected_index].label.tag);
        }

        let bar_height = (ctx.bounds.height() * 0.12).clamp(44.0, 64.0);
        let (bar_rect, content_rect) = match position {
            TabPosition::Top => (
                vello::kurbo::Rect::new(
                    ctx.bounds.x0,
                    ctx.bounds.y0,
                    ctx.bounds.x1,
                    (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
                ),
                vello::kurbo::Rect::new(
                    ctx.bounds.x0,
                    (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
                    ctx.bounds.x1,
                    ctx.bounds.y1,
                ),
            ),
            TabPosition::Bottom => (
                vello::kurbo::Rect::new(
                    ctx.bounds.x0,
                    (ctx.bounds.y1 - bar_height).max(ctx.bounds.y0),
                    ctx.bounds.x1,
                    ctx.bounds.y1,
                ),
                vello::kurbo::Rect::new(
                    ctx.bounds.x0,
                    ctx.bounds.y0,
                    ctx.bounds.x1,
                    (ctx.bounds.y1 - bar_height).max(ctx.bounds.y0),
                ),
            ),
        };

        {
            let scene = unsafe { ctx.scene() };
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                vello::peniko::Color::new([0.95, 0.95, 0.97, 1.0]),
                None,
                &bar_rect,
            );
            let separator = if matches!(position, TabPosition::Top) {
                vello::kurbo::Rect::new(bar_rect.x0, bar_rect.y1 - 1.0, bar_rect.x1, bar_rect.y1)
            } else {
                vello::kurbo::Rect::new(bar_rect.x0, bar_rect.y0, bar_rect.x1, bar_rect.y0 + 1.0)
            };
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                vello::peniko::Color::new([0.82, 0.82, 0.85, 1.0]),
                None,
                &separator,
            );
        }

        let mut selected_content = None;
        for (index, tab) in tabs.tabs.into_iter().enumerate() {
            if index == selected_index {
                selected_content = Some(AnyView::new(tab.content.build()));
            }

            let button_width = bar_rect.width() / tab_count as f64;
            let x0 = bar_rect.x0 + button_width * index as f64;
            let button_rect =
                vello::kurbo::Rect::new(x0, bar_rect.y0, x0 + button_width, bar_rect.y1);
            {
                let scene = unsafe { ctx.scene() };
                if index == selected_index {
                    let highlight = inset_rect(button_rect, 4.0, 6.0);
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        ctx.transform,
                        vello::peniko::Color::new([0.84, 0.9, 1.0, 1.0]),
                        None,
                        &vello::kurbo::RoundedRect::from_rect(highlight, 8.0),
                    );
                }
            }
            let label_rect = inset_rect(button_rect, 8.0, 8.0);
            let tab_id = tab.label.tag;
            if label_rect.width() > 0.0 && label_rect.height() > 0.0 {
                Self::dispatch_in_rect(ctx, env, tab.label.content, label_rect);
            }

            let selection_binding = selection.clone();
            let renderer = unsafe { ctx.renderer() };
            renderer.register_pointer_target(
                transformed_rect(ctx.transform, button_rect),
                move |_point, _env| {
                    if selection_binding.get() != tab_id {
                        selection_binding.set(tab_id);
                    }
                    true
                },
            );
        }

        if let Some(content) = selected_content {
            if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
                Self::dispatch_in_rect(ctx, env, content, content_rect);
            }
        }
    }

    fn render_list(
        state: &mut HydroState,
        ctx: RenderContext,
        list: Native<ListConfig>,
        env: &Environment,
    ) {
        let list = list.into_inner();
        let editing = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&list.editing)
        };
        let row_count_signal = list.contents.len();
        let row_count = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&row_count_signal)
        };
        let delete_action = list.on_delete.map(Rc::new);
        let move_action = list.on_move.map(Rc::new);

        let mut rows = Vec::with_capacity(row_count);
        for index in 0..row_count {
            let item = list.contents.get_view(index).unwrap_or_else(|| {
                panic!("ListConfig failed to materialize item at index {index}")
            });
            let intrinsic = estimate_intrinsic_size(&item.content, state, env);
            let row_height = f64::from(intrinsic.height.max(28.0)) + 16.0;
            rows.push((index, item, row_height));
        }

        let viewport = ctx.bounds;
        let content_height = rows
            .iter()
            .fold(0.0, |acc, (_index, _item, height)| acc + *height)
            .max(viewport.height());
        let handle = {
            let renderer = unsafe { ctx.renderer() };
            renderer.scroll_controller.bind(
                ScrollAxis::Vertical,
                viewport.width(),
                viewport.height(),
                viewport.width(),
                content_height,
            )
        };
        let metrics = handle.metrics();
        {
            let scene = unsafe { ctx.scene() };
            scene.push_layer(
                vello::peniko::Fill::NonZero,
                vello::peniko::BlendMode::default(),
                1.0,
                ctx.transform,
                &viewport,
            );
        }

        let total_rows = rows.len();
        let mut y = viewport.y0 - metrics.offset_y;
        for (index, item, row_height) in rows {
            let row_rect = vello::kurbo::Rect::new(viewport.x0, y, viewport.x1, y + row_height);
            y += row_height;
            if row_rect.y1 <= viewport.y0 || row_rect.y0 >= viewport.y1 {
                continue;
            }

            {
                let scene = unsafe { ctx.scene() };
                let row_color = if index % 2 == 0 {
                    vello::peniko::Color::new([1.0, 1.0, 1.0, 1.0])
                } else {
                    vello::peniko::Color::new([0.985, 0.985, 0.99, 1.0])
                };
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    row_color,
                    None,
                    &row_rect,
                );
            }

            let deletable = {
                let renderer = unsafe { ctx.renderer() };
                renderer.read_signal(&item.deletable)
            };
            let mut content_rect = inset_rect(row_rect, 12.0, 8.0);
            let mut trailing_x = row_rect.x1 - 8.0;

            if editing && move_action.is_some() {
                let control_width = 20.0;
                let control_height = (row_height - 12.0).max(12.0);
                let control_rect = vello::kurbo::Rect::new(
                    trailing_x - control_width,
                    row_rect.y0 + 6.0,
                    trailing_x,
                    row_rect.y0 + 6.0 + control_height,
                );
                trailing_x -= control_width + 6.0;
                {
                    let scene = unsafe { ctx.scene() };
                    draw_stepper_button(scene, ctx.transform, control_rect);
                    let split = control_rect.y0 + control_rect.height() / 2.0;
                    let separator = vello::kurbo::Line::new(
                        (control_rect.x0 + 3.0, split),
                        (control_rect.x1 - 3.0, split),
                    );
                    scene.stroke(
                        &vello::kurbo::Stroke::new(1.0),
                        ctx.transform,
                        vello::peniko::Color::new([0.65, 0.65, 0.68, 1.0]),
                        None,
                        &separator,
                    );
                }

                if index > 0 {
                    let up_rect = vello::kurbo::Rect::new(
                        control_rect.x0,
                        control_rect.y0,
                        control_rect.x1,
                        control_rect.y0 + control_rect.height() / 2.0,
                    );
                    let action = Rc::clone(move_action.as_ref().expect("move action missing"));
                    let renderer = unsafe { ctx.renderer() };
                    renderer.register_pointer_target(
                        transformed_rect(ctx.transform, up_rect),
                        move |_point, env| {
                            (action.as_ref())(env, index, index - 1);
                            true
                        },
                    );
                }
                if index + 1 < total_rows {
                    let down_rect = vello::kurbo::Rect::new(
                        control_rect.x0,
                        control_rect.y0 + control_rect.height() / 2.0,
                        control_rect.x1,
                        control_rect.y1,
                    );
                    let action = Rc::clone(move_action.as_ref().expect("move action missing"));
                    let renderer = unsafe { ctx.renderer() };
                    renderer.register_pointer_target(
                        transformed_rect(ctx.transform, down_rect),
                        move |_point, env| {
                            (action.as_ref())(env, index, index + 1);
                            true
                        },
                    );
                }
            }

            if editing && deletable && delete_action.is_some() {
                let delete_rect = vello::kurbo::Rect::new(
                    trailing_x - 26.0,
                    row_rect.y0 + 6.0,
                    trailing_x,
                    row_rect.y1 - 6.0,
                );
                trailing_x = delete_rect.x0 - 6.0;
                {
                    let scene = unsafe { ctx.scene() };
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        ctx.transform,
                        vello::peniko::Color::new([0.91, 0.25, 0.2, 1.0]),
                        None,
                        &vello::kurbo::RoundedRect::from_rect(delete_rect, 5.0),
                    );
                }
                let action = Rc::clone(delete_action.as_ref().expect("delete action missing"));
                let renderer = unsafe { ctx.renderer() };
                renderer.register_pointer_target(
                    transformed_rect(ctx.transform, delete_rect),
                    move |_point, env| {
                        (action.as_ref())(env, index);
                        true
                    },
                );
            }

            content_rect.x1 = content_rect.x1.min(trailing_x);
            if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
                Self::dispatch_in_rect(ctx, env, item.content, content_rect);
            }

            {
                let scene = unsafe { ctx.scene() };
                let separator = vello::kurbo::Rect::new(
                    row_rect.x0 + 8.0,
                    row_rect.y1 - 1.0,
                    row_rect.x1 - 8.0,
                    row_rect.y1,
                );
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.9, 0.9, 0.92, 1.0]),
                    None,
                    &separator,
                );
            }
        }

        {
            let scene = unsafe { ctx.scene() };
            scene.pop_layer();
        }

        let renderer = unsafe { ctx.renderer() };
        let handle_for_input = handle.clone();
        renderer
            .register_scroll_target(transformed_rect(ctx.transform, viewport), move |dx, dy| {
                handle_for_input.apply_scroll_delta(dx, dy)
            });
        let scene = unsafe { ctx.scene() };
        Self::draw_scroll_indicators(
            scene,
            ctx.transform,
            viewport,
            metrics,
            ScrollAxis::Vertical,
        );
    }

    fn render_table(
        state: &mut HydroState,
        ctx: RenderContext,
        table: Native<TableConfig>,
        env: &Environment,
    ) {
        let table = table.into_inner();
        let columns = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&table.columns)
        };
        if columns.is_empty() {
            return;
        }

        let mut column_widths = Vec::with_capacity(columns.len());
        let mut max_rows = 0usize;
        for column in &columns {
            let mut width: f64 = 72.0;
            let label_size = estimate_intrinsic_size(&AnyView::new(column.label()), state, env);
            width = width.max(f64::from(label_size.width) + 18.0);

            let rows = column.rows();
            let row_count_signal = rows.len();
            let row_count = {
                let renderer = unsafe { ctx.renderer() };
                renderer.read_signal(&row_count_signal)
            };
            max_rows = max_rows.max(row_count);
            for index in 0..row_count {
                if let Some(cell) = rows.get_view(index) {
                    let size = estimate_intrinsic_size(&AnyView::new(cell), state, env);
                    width = width.max(f64::from(size.width) + 18.0);
                }
            }
            column_widths.push(width);
        }

        let header_height = 32.0;
        let row_height = 30.0;
        let table_width: f64 = column_widths.iter().sum::<f64>();
        let table_height = header_height + row_height * max_rows as f64;
        let viewport = ctx.bounds;
        let handle = {
            let renderer = unsafe { ctx.renderer() };
            renderer.scroll_controller.bind(
                ScrollAxis::All,
                viewport.width(),
                viewport.height(),
                table_width.max(viewport.width()),
                table_height.max(viewport.height()),
            )
        };
        let metrics = handle.metrics();

        {
            let scene = unsafe { ctx.scene() };
            scene.push_layer(
                vello::peniko::Fill::NonZero,
                vello::peniko::BlendMode::default(),
                1.0,
                ctx.transform,
                &viewport,
            );
        }

        let origin_x = viewport.x0 - metrics.offset_x;
        let origin_y = viewport.y0 - metrics.offset_y;
        {
            let scene = unsafe { ctx.scene() };
            let header_rect = vello::kurbo::Rect::new(
                origin_x,
                origin_y,
                origin_x + table_width,
                origin_y + header_height,
            );
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                vello::peniko::Color::new([0.95, 0.95, 0.96, 1.0]),
                None,
                &header_rect,
            );
        }

        let mut x = origin_x;
        for (column_index, column) in columns.into_iter().enumerate() {
            let width = column_widths[column_index];
            let header_cell =
                vello::kurbo::Rect::new(x, origin_y, x + width, origin_y + header_height);
            Self::dispatch_in_rect(
                ctx,
                env,
                AnyView::new(column.label()),
                inset_rect(header_cell, 8.0, 6.0),
            );

            let rows = column.rows();
            for row_index in 0..max_rows {
                let cell_rect = vello::kurbo::Rect::new(
                    x,
                    origin_y + header_height + row_height * row_index as f64,
                    x + width,
                    origin_y + header_height + row_height * (row_index + 1) as f64,
                );
                if let Some(cell) = rows.get_view(row_index) {
                    Self::dispatch_in_rect(
                        ctx,
                        env,
                        AnyView::new(cell),
                        inset_rect(cell_rect, 8.0, 6.0),
                    );
                }
                let scene = unsafe { ctx.scene() };
                scene.stroke(
                    &vello::kurbo::Stroke::new(1.0),
                    ctx.transform,
                    vello::peniko::Color::new([0.85, 0.85, 0.87, 1.0]),
                    None,
                    &cell_rect,
                );
            }

            let separator = vello::kurbo::Line::new(
                (x + width, origin_y),
                (x + width, origin_y + table_height),
            );
            let scene = unsafe { ctx.scene() };
            scene.stroke(
                &vello::kurbo::Stroke::new(1.0),
                ctx.transform,
                vello::peniko::Color::new([0.8, 0.8, 0.83, 1.0]),
                None,
                &separator,
            );
            x += width;
        }

        {
            let scene = unsafe { ctx.scene() };
            scene.pop_layer();
        }

        let renderer = unsafe { ctx.renderer() };
        let handle_for_input = handle.clone();
        renderer
            .register_scroll_target(transformed_rect(ctx.transform, viewport), move |dx, dy| {
                handle_for_input.apply_scroll_delta(dx, dy)
            });
        let scene = unsafe { ctx.scene() };
        Self::draw_scroll_indicators(scene, ctx.transform, viewport, metrics, ScrollAxis::All);
    }

    fn render_divider(
        _state: &mut HydroState,
        ctx: RenderContext,
        _divider: Divider,
        env: &Environment,
    ) {
        let vertical = matches!(env.get::<StackAxis>(), Some(StackAxis::Horizontal));
        let rect = if vertical {
            vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x0 + 1.0,
                ctx.bounds.y1,
            )
        } else {
            vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                ctx.bounds.y0 + 1.0,
            )
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
        let styled = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&text.into_inner().content)
        };
        Self::render_styled_text(state, ctx, styled, env);
    }

    fn render_styled_text(
        state: &mut HydroState,
        ctx: RenderContext,
        styled: StyledStr,
        env: &Environment,
    ) {
        let layout = Self::build_text_layout(state, styled, env, Some(ctx.bounds.width() as f32));
        if layout.is_empty() {
            return;
        }

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
                        vello::Glyph { id: glyph.id, x, y }
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

    fn build_text_layout(
        state: &mut HydroState,
        styled: StyledStr,
        env: &Environment,
        max_width: Option<f32>,
    ) -> parley::Layout<[u8; 4]> {
        let mut plain = String::new();
        let mut spans = Vec::with_capacity(styled.chunks().len());
        for (chunk, style) in styled.chunks() {
            let start = plain.len();
            plain.push_str(chunk.as_str());
            let end = plain.len();
            spans.push((start..end, style.clone()));
        }

        if plain.is_empty() {
            return parley::Layout::new();
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
        layout.break_all_lines(max_width);
        layout.align(
            max_width,
            parley::Alignment::Start,
            parley::AlignmentOptions::default(),
        );
        layout
    }

    fn measure_text_intrinsic_size(
        state: &mut HydroState,
        styled: StyledStr,
        env: &Environment,
    ) -> LayoutSize {
        let layout = Self::build_text_layout(state, styled, env, None);
        LayoutSize::new(layout.full_width(), layout.height())
    }

    fn push_text_style(
        builder: &mut parley::RangedBuilder<'_, [u8; 4]>,
        family_storage: &mut Vec<String>,
        style: TextStyle,
        range: std::ops::Range<usize>,
        env: &Environment,
    ) {
        let resolved_font = style.font.resolve(env).get();
        builder.push(
            parley::StyleProperty::FontSize(resolved_font.size),
            range.clone(),
        );
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
        builder.push(
            parley::StyleProperty::Underline(style.underline),
            range.clone(),
        );
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
        let hit_bounds = transformed_rect(ctx.transform, ctx.bounds);
        {
            let renderer = unsafe { ctx.renderer() };
            let mut action = button.action;
            renderer.register_pointer_target(hit_bounds, move |_point, env| {
                action(env);
                true
            });
        }
        Self::dispatch_any(ctx, env, button.label);
    }

    fn render_toggle(
        _state: &mut HydroState,
        ctx: RenderContext,
        toggle: Native<ToggleConfig>,
        env: &Environment,
    ) {
        let toggle = toggle.into_inner();
        let switch_width = 51.0;
        let switch_height = 31.0;
        let spacing = 8.0;
        let switch_x0 = (ctx.bounds.x1 - switch_width).max(ctx.bounds.x0);
        let switch_y0 = ctx.bounds.y0 + ((ctx.bounds.height() - switch_height) / 2.0).max(0.0);
        let switch_bounds = vello::kurbo::Rect::new(
            switch_x0,
            switch_y0,
            switch_x0 + switch_width,
            switch_y0 + switch_height,
        );
        let label_bounds = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            (switch_x0 - spacing).max(ctx.bounds.x0),
            ctx.bounds.y1,
        );
        if label_bounds.width() > 0.0 {
            Self::dispatch_in_rect(ctx, env, toggle.label, label_bounds);
        }

        let thumb_progress = {
            let renderer = unsafe { ctx.renderer() };
            renderer.resolve_toggle_progress(&toggle.toggle)
        };
        let track_color = lerp_color(
            [0.7058824, 0.7058824, 0.7254902, 1.0],
            [0.20392157, 0.78039217, 0.34901962, 1.0],
            thumb_progress,
        );
        let thumb_center_x = lerp_f64(
            switch_bounds.x0 + 15.0,
            switch_bounds.x1 - 15.0,
            thumb_progress,
        );
        let thumb_center =
            vello::kurbo::Point::new(thumb_center_x, switch_bounds.y0 + switch_height / 2.0);
        let track = vello::kurbo::RoundedRect::from_rect(switch_bounds, 15.5);
        let thumb = vello::kurbo::Circle::new(thumb_center, 13.0);

        let scene = unsafe { ctx.scene() };
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            track_color,
            None,
            &track,
        );
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            vello::peniko::Color::WHITE,
            None,
            &thumb,
        );

        let hit_bounds = transformed_rect(ctx.transform, switch_bounds);
        let toggle_binding = toggle.toggle;
        let renderer = unsafe { ctx.renderer() };
        renderer.register_pointer_target(hit_bounds, move |_point, _env| {
            let next = !toggle_binding.get();
            toggle_binding.set(next);
            true
        });
    }

    fn render_slider(
        _state: &mut HydroState,
        ctx: RenderContext,
        slider: Native<SliderConfig>,
        env: &Environment,
    ) {
        let slider = slider.into_inner();
        let label_height = if ctx.bounds.height() >= 36.0 {
            20.0
        } else {
            0.0
        };
        if label_height > 0.0 {
            let label_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
            );
            Self::dispatch_in_rect(ctx, env, slider.label, label_rect);
        }

        let range_start = *slider.range.start();
        let range_end = *slider.range.end();
        let span = range_end - range_start;
        if span <= 0.0 {
            panic!("hydrolysis slider requires range start < end");
        }

        let track_left = ctx.bounds.x0 + 12.0;
        let track_right = ctx.bounds.x1 - 12.0;
        let track_center_y = ctx.bounds.y1 - ((ctx.bounds.height() - label_height) / 2.0).max(10.0);
        let track_rect = vello::kurbo::Rect::new(
            track_left,
            track_center_y - 2.0,
            track_right,
            track_center_y + 2.0,
        );

        let clamped = {
            let renderer = unsafe { ctx.renderer() };
            renderer
                .read_signal(&slider.value)
                .clamp(range_start, range_end)
        };
        let progress = (clamped - range_start) / span;
        let fill_right = track_left + (track_right - track_left) * progress;
        let fill_rect = vello::kurbo::Rect::new(
            track_left,
            track_center_y - 2.0,
            fill_right,
            track_center_y + 2.0,
        );
        let thumb =
            vello::kurbo::Circle::new(vello::kurbo::Point::new(fill_right, track_center_y), 7.0);

        let scene = unsafe { ctx.scene() };
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            vello::peniko::Color::new([0.75, 0.75, 0.78, 1.0]),
            None,
            &track_rect,
        );
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            vello::peniko::Color::new([0.20392157, 0.53333336, 0.94509804, 1.0]),
            None,
            &fill_rect,
        );
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            vello::peniko::Color::WHITE,
            None,
            &thumb,
        );

        let hit_bounds = transformed_rect(
            ctx.transform,
            vello::kurbo::Rect::new(
                track_left,
                track_center_y - 14.0,
                track_right,
                track_center_y + 14.0,
            ),
        );
        let value_binding = slider.value;
        let usable_track = track_right - track_left;
        let renderer = unsafe { ctx.renderer() };
        renderer.register_pointer_target(hit_bounds, move |point, _env| {
            let x = point.x.clamp(track_left, track_right);
            let t = (x - track_left) / usable_track;
            value_binding.set(range_start + span * t);
            true
        });
    }

    fn render_stepper(
        _state: &mut HydroState,
        ctx: RenderContext,
        stepper: Native<StepperConfig>,
        env: &Environment,
    ) {
        let stepper = stepper.into_inner();
        let button_size = ctx.bounds.height().clamp(24.0, 32.0);
        let spacing = 4.0;
        let controls_width = button_size * 2.0 + spacing;
        let controls_x0 = (ctx.bounds.x1 - controls_width).max(ctx.bounds.x0);

        let label_bounds =
            vello::kurbo::Rect::new(ctx.bounds.x0, ctx.bounds.y0, controls_x0, ctx.bounds.y1);
        if label_bounds.width() > 0.0 {
            Self::dispatch_in_rect(ctx, env, stepper.label, label_bounds);
        }

        let button_y0 = ctx.bounds.y0 + ((ctx.bounds.height() - button_size) / 2.0).max(0.0);
        let minus_bounds = vello::kurbo::Rect::new(
            controls_x0,
            button_y0,
            controls_x0 + button_size,
            button_y0 + button_size,
        );
        let plus_bounds = vello::kurbo::Rect::new(
            controls_x0 + button_size + spacing,
            button_y0,
            controls_x0 + controls_width,
            button_y0 + button_size,
        );
        let scene = unsafe { ctx.scene() };
        draw_stepper_button(scene, ctx.transform, minus_bounds);
        draw_stepper_button(scene, ctx.transform, plus_bounds);

        let line_color = vello::peniko::Color::new([0.2, 0.2, 0.22, 1.0]);
        let minus_line = vello::kurbo::Line::new(
            (minus_bounds.x0 + 6.0, minus_bounds.y0 + button_size / 2.0),
            (minus_bounds.x1 - 6.0, minus_bounds.y0 + button_size / 2.0),
        );
        let plus_horizontal = vello::kurbo::Line::new(
            (plus_bounds.x0 + 6.0, plus_bounds.y0 + button_size / 2.0),
            (plus_bounds.x1 - 6.0, plus_bounds.y0 + button_size / 2.0),
        );
        let plus_vertical = vello::kurbo::Line::new(
            (plus_bounds.x0 + button_size / 2.0, plus_bounds.y0 + 6.0),
            (plus_bounds.x0 + button_size / 2.0, plus_bounds.y1 - 6.0),
        );
        let stroke = vello::kurbo::Stroke::new(2.0);
        scene.stroke(&stroke, ctx.transform, line_color, None, &minus_line);
        scene.stroke(&stroke, ctx.transform, line_color, None, &plus_horizontal);
        scene.stroke(&stroke, ctx.transform, line_color, None, &plus_vertical);

        let range_start = *stepper.range.start();
        let range_end = *stepper.range.end();
        if range_start > range_end {
            panic!("hydrolysis stepper requires an ordered range");
        }

        let value_binding_minus = stepper.value.clone();
        let value_binding_plus = stepper.value;
        let step_signal_minus = stepper.step.clone();
        let step_signal_plus = stepper.step;

        let renderer = unsafe { ctx.renderer() };
        renderer.register_pointer_target(
            transformed_rect(ctx.transform, minus_bounds),
            move |_point, _env| {
                let step = step_signal_minus.get();
                if step <= 0 {
                    panic!("hydrolysis stepper requires positive step");
                }
                let current = value_binding_minus.get();
                let next = current.saturating_sub(step).clamp(range_start, range_end);
                value_binding_minus.set(next);
                true
            },
        );
        renderer.register_pointer_target(
            transformed_rect(ctx.transform, plus_bounds),
            move |_point, _env| {
                let step = step_signal_plus.get();
                if step <= 0 {
                    panic!("hydrolysis stepper requires positive step");
                }
                let current = value_binding_plus.get();
                let next = current.saturating_add(step).clamp(range_start, range_end);
                value_binding_plus.set(next);
                true
            },
        );
    }

    fn render_progress(
        _state: &mut HydroState,
        ctx: RenderContext,
        progress: Native<ProgressConfig>,
        env: &Environment,
    ) {
        let progress = progress.into_inner();
        let clamped = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&progress.value).clamp(0.0, 1.0) as f64
        };

        match progress.style {
            ProgressStyle::Linear => {
                let label_height = if ctx.bounds.height() >= 40.0 {
                    18.0
                } else {
                    0.0
                };
                if label_height > 0.0 {
                    let label_rect = vello::kurbo::Rect::new(
                        ctx.bounds.x0,
                        ctx.bounds.y0,
                        ctx.bounds.x1,
                        (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
                    );
                    Self::dispatch_in_rect(ctx, env, progress.label, label_rect);
                }

                let bar_y = ctx.bounds.y0 + label_height + 10.0;
                let bar = vello::kurbo::RoundedRect::from_rect(
                    vello::kurbo::Rect::new(
                        ctx.bounds.x0 + 8.0,
                        bar_y,
                        ctx.bounds.x1 - 8.0,
                        bar_y + 8.0,
                    ),
                    4.0,
                );
                let width = bar.rect().width() * clamped;
                let fill = vello::kurbo::RoundedRect::from_rect(
                    vello::kurbo::Rect::new(
                        bar.rect().x0,
                        bar.rect().y0,
                        bar.rect().x0 + width,
                        bar.rect().y1,
                    ),
                    4.0,
                );
                let scene = unsafe { ctx.scene() };
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.84, 0.84, 0.87, 1.0]),
                    None,
                    &bar,
                );
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.20392157, 0.53333336, 0.94509804, 1.0]),
                    None,
                    &fill,
                );

                let value_label_rect = vello::kurbo::Rect::new(
                    ctx.bounds.x0,
                    bar.rect().y1 + 6.0,
                    ctx.bounds.x1,
                    ctx.bounds.y1,
                );
                if value_label_rect.height() > 0.0 {
                    Self::dispatch_in_rect(ctx, env, progress.value_label, value_label_rect);
                }
            }
            ProgressStyle::Circular => {
                let center = vello::kurbo::Point::new(
                    ctx.bounds.x0 + ctx.bounds.width() / 2.0,
                    ctx.bounds.y0 + ctx.bounds.height() / 2.0,
                );
                let radius = (ctx.bounds.width().min(ctx.bounds.height()) / 2.0 - 6.0).max(2.0);
                let track = vello::kurbo::Circle::new(center, radius);
                let arc =
                    circle_arc_path(center, radius, -core::f64::consts::FRAC_PI_2, TAU * clamped);
                let scene = unsafe { ctx.scene() };
                let stroke = vello::kurbo::Stroke::new(5.0);
                scene.stroke(
                    &stroke,
                    ctx.transform,
                    vello::peniko::Color::new([0.84, 0.84, 0.87, 1.0]),
                    None,
                    &track,
                );
                scene.stroke(
                    &stroke,
                    ctx.transform,
                    vello::peniko::Color::new([0.20392157, 0.53333336, 0.94509804, 1.0]),
                    None,
                    &arc,
                );
                let label_rect = vello::kurbo::Rect::new(
                    ctx.bounds.x0,
                    ctx.bounds.y1 + 4.0,
                    ctx.bounds.x1,
                    ctx.bounds.y1,
                );
                let _ = label_rect;
            }
            _ => {
                panic!("hydrolysis ProgressStyle variant is not implemented");
            }
        }
    }

    fn render_text_field(
        state: &mut HydroState,
        ctx: RenderContext,
        text_field: Native<TextFieldConfig>,
        env: &Environment,
    ) {
        let text_field = text_field.into_inner();
        let label_height = if ctx.bounds.height() >= 36.0 {
            18.0
        } else {
            0.0
        };
        if label_height > 0.0 {
            let label_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
            );
            Self::dispatch_in_rect(ctx, env, text_field.label, label_rect);
        }

        let field_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0 + label_height,
            ctx.bounds.x1,
            ctx.bounds.y1,
        );
        let scene = unsafe { ctx.scene() };
        draw_input_field(scene, ctx.transform, field_rect);

        let prompt_signal = text_field.prompt.content();
        let (prompt, value, preedit) = {
            let renderer = unsafe { ctx.renderer() };
            let preedit =
                if renderer.focused_text_input.get() == Some(renderer.text_input_targets.len()) {
                    renderer.ime_preedit.clone().unwrap_or_default()
                } else {
                    String::new()
                };
            (
                renderer.read_signal(&prompt_signal).to_plain().to_string(),
                renderer
                    .read_signal(&text_field.value)
                    .to_plain()
                    .to_string(),
                preedit,
            )
        };
        let display = if value.is_empty() && preedit.is_empty() {
            prompt
        } else {
            format!("{value}{preedit}")
        };
        let text_bounds = inset_rect(field_rect, 8.0, 6.0);
        Self::render_styled_text(
            state,
            ctx.child(
                vello::kurbo::Affine::translate((text_bounds.x0, text_bounds.y0)),
                vello::kurbo::Rect::new(0.0, 0.0, text_bounds.width(), text_bounds.height()),
            ),
            StyledStr::plain(display),
            env,
        );

        let value_binding = text_field.value;
        let renderer = unsafe { ctx.renderer() };
        renderer.register_text_input_target(
            transformed_rect(ctx.transform, field_rect),
            TextInputPurpose::Normal,
            move |command| {
                let mut plain = value_binding.get().to_plain().to_string();
                match command {
                    TextInputCommand::Insert(text) => {
                        if text.is_empty() {
                            return false;
                        }
                        plain.push_str(text.as_str());
                        value_binding.set(StyledStr::plain(plain));
                        true
                    }
                    TextInputCommand::Backspace => {
                        if plain.pop().is_none() {
                            return false;
                        }
                        value_binding.set(StyledStr::plain(plain));
                        true
                    }
                }
            },
        );
    }

    fn render_secure_field(
        state: &mut HydroState,
        ctx: RenderContext,
        secure_field: Native<SecureFieldConfig>,
        env: &Environment,
    ) {
        let secure_field = secure_field.into_inner();
        let label_height = if ctx.bounds.height() >= 36.0 {
            18.0
        } else {
            0.0
        };
        if label_height > 0.0 {
            let label_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
            );
            Self::dispatch_in_rect(ctx, env, secure_field.label, label_rect);
        }

        let field_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0 + label_height,
            ctx.bounds.x1,
            ctx.bounds.y1,
        );
        let scene = unsafe { ctx.scene() };
        draw_input_field(scene, ctx.transform, field_rect);

        let masked = {
            let renderer = unsafe { ctx.renderer() };
            let preedit_count =
                if renderer.focused_text_input.get() == Some(renderer.text_input_targets.len()) {
                    renderer
                        .ime_preedit
                        .as_ref()
                        .map_or(0, |value| value.chars().count())
                } else {
                    0
                };
            let count = renderer
                .read_signal(&secure_field.value)
                .expose()
                .chars()
                .count()
                + preedit_count;
            "*".repeat(count)
        };
        let text_bounds = inset_rect(field_rect, 8.0, 6.0);
        Self::render_styled_text(
            state,
            ctx.child(
                vello::kurbo::Affine::translate((text_bounds.x0, text_bounds.y0)),
                vello::kurbo::Rect::new(0.0, 0.0, text_bounds.width(), text_bounds.height()),
            ),
            StyledStr::plain(masked),
            env,
        );

        let value_binding = secure_field.value;
        let renderer = unsafe { ctx.renderer() };
        renderer.register_text_input_target(
            transformed_rect(ctx.transform, field_rect),
            TextInputPurpose::Password,
            move |command| {
                let mut plain = value_binding.get().expose().to_owned();
                match command {
                    TextInputCommand::Insert(text) => {
                        if text.is_empty() {
                            return false;
                        }
                        plain.push_str(text.as_str());
                    }
                    TextInputCommand::Backspace => {
                        if plain.pop().is_none() {
                            return false;
                        }
                    }
                }
                let mut next = FormSecure::default();
                next.set(plain);
                value_binding.set(next);
                true
            },
        );
    }

    fn render_picker(
        state: &mut HydroState,
        ctx: RenderContext,
        picker: Native<PickerConfig>,
        env: &Environment,
    ) {
        let picker = picker.into_inner();
        let items = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&picker.items)
        };
        if items.is_empty() {
            panic!("hydrolysis picker requires at least one item");
        }

        let selected = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&picker.selection)
        };
        let selected_index = items
            .iter()
            .position(|item| item.tag == selected)
            .unwrap_or(0);
        let selected_text = {
            let selected_signal = items[selected_index].content.content();
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&selected_signal).to_plain()
        };
        let ids: Vec<_> = items.iter().map(|item| item.tag).collect();

        let scene = unsafe { ctx.scene() };
        draw_input_field(scene, ctx.transform, ctx.bounds);

        let text_bounds = inset_rect(ctx.bounds, 8.0, 6.0);
        Self::render_styled_text(
            state,
            ctx.child(
                vello::kurbo::Affine::translate((text_bounds.x0, text_bounds.y0)),
                vello::kurbo::Rect::new(0.0, 0.0, text_bounds.width(), text_bounds.height()),
            ),
            StyledStr::plain(selected_text),
            env,
        );

        let selection_binding = picker.selection;
        let renderer = unsafe { ctx.renderer() };
        renderer.register_pointer_target(
            transformed_rect(ctx.transform, ctx.bounds),
            move |_point, _env| {
                let current = selection_binding.get();
                let index = ids.iter().position(|id| *id == current).unwrap_or(0);
                let next = ids[(index + 1) % ids.len()];
                selection_binding.set(next);
                true
            },
        );
    }

    fn render_dynamic(
        _state: &mut HydroState,
        ctx: RenderContext,
        dynamic: Native<Dynamic>,
        env: &Environment,
    ) {
        let current = Rc::new(RefCell::new(None::<AnyView>));
        let is_initial = Rc::new(Cell::new(true));
        let rebuild_requested = {
            let renderer = unsafe { ctx.renderer() };
            Rc::clone(&renderer.rebuild_requested)
        };
        dynamic.into_inner().connect({
            let current = Rc::clone(&current);
            let is_initial = Rc::clone(&is_initial);
            let rebuild_requested = Rc::clone(&rebuild_requested);
            move |update| {
                *current.borrow_mut() = Some(update.into_value());
                if !is_initial.replace(false) {
                    rebuild_requested.set(true);
                }
            }
        });
        let content = current
            .borrow_mut()
            .take()
            .expect("hydrolysis Dynamic must provide an initial view before dispatch");
        Self::dispatch_any(ctx, env, content);
    }

    fn render_system_icon(
        state: &mut HydroState,
        ctx: RenderContext,
        icon: Native<SystemIcon>,
        env: &Environment,
    ) {
        let styled = StyledStr::plain(icon.into_inner().name);
        Self::render_styled_text(state, ctx, styled, env);
    }

    fn render_gpu_surface(
        _state: &mut HydroState,
        ctx: RenderContext,
        surface: Native<GpuSurface>,
        env: &Environment,
    ) {
        let width = (ctx.bounds.width().max(1.0).round()) as u32;
        let height = (ctx.bounds.height().max(1.0).round()) as u32;
        let size = OffscreenSize::try_from_pixels(width, height)
            .expect("hydrolysis GpuSurface requires non-zero offscreen size");
        let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8Unorm);
        let mut local_env = env.clone();
        let output = surface
            .into_inner()
            .render_offscreen(config, &mut local_env)
            .expect("hydrolysis failed to render GpuSurface offscreen");

        let image = vello::peniko::ImageData {
            data: vello::peniko::Blob::from(output.rgba8),
            format: vello::peniko::ImageFormat::Rgba8,
            alpha_type: vello::peniko::ImageAlphaType::Alpha,
            width: output.width,
            height: output.height,
        };
        let image_transform = vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))
            * vello::kurbo::Affine::scale_non_uniform(
                ctx.bounds.width() / f64::from(output.width),
                ctx.bounds.height() / f64::from(output.height),
            );
        let scene = unsafe { ctx.scene() };
        scene.draw_image(
            &vello::peniko::ImageBrush::new(image),
            ctx.transform * image_transform,
        );
    }

    fn render_view_effect(
        _state: &mut HydroState,
        ctx: RenderContext,
        effect: Native<ViewEffectErased>,
        env: &Environment,
    ) {
        let mut effect = effect.into_inner();
        let renderer = unsafe { ctx.renderer() };
        let (device_ptr, queue_ptr) = renderer.state().frame_resource_ptrs();
        let device = unsafe { &*device_ptr };
        let queue = unsafe { &*queue_ptr };

        let input_width = (ctx.bounds.width().max(1.0).round()) as u32;
        let input_height = (ctx.bounds.height().max(1.0).round()) as u32;
        let output_size = effect.output_size();
        let (output_width, output_height) = output_size.compute(input_width, input_height);
        if output_width == 0 || output_height == 0 {
            panic!("hydrolysis ViewEffect requires non-zero output dimensions");
        }

        let subtree = Self::render_subtree_scene(ctx, env, effect.take_content());

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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());
        renderer
            .vello_renderer
            .render_to_texture(
                device,
                queue,
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

        let setup_context = EffectContext {
            device,
            queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: None,
        };
        pollster::block_on(effect.setup(&setup_context));

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

        let input = EffectInput {
            device,
            queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: input_width,
            height: input_height,
        };
        let output = EffectOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: output_width,
            height: output_height,
        };
        effect.render(&input, &output);
        if effect.needs_redraw() {
            renderer.request_rebuild();
        }

        let image = renderer.vello_renderer.register_texture(output_texture);
        renderer.active_filter_images.push(image.clone());
        let image_transform = vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))
            * vello::kurbo::Affine::scale_non_uniform(
                ctx.bounds.width() / f64::from(output_width),
                ctx.bounds.height() / f64::from(output_height),
            );
        let scene = unsafe { ctx.scene() };
        scene.draw_image(
            &vello::peniko::ImageBrush::new(image),
            ctx.transform * image_transform,
        );
    }

    fn render_resolved_color(
        _state: &mut HydroState,
        ctx: RenderContext,
        color: Native<ResolvedColor>,
        _env: &Environment,
    ) {
        let scene = unsafe { ctx.scene() };
        let brush = resolved_color_to_peniko(color.into_inner());
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            brush,
            None,
            &ctx.bounds,
        );
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
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            fill,
            None,
            &path,
        );
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
        let Metadata { content, value } = metadata;
        let renderer = unsafe { ctx.renderer() };
        renderer.current_frame_retain.push(value);
        renderer.dispatcher.dispatch(content, env, ctx);
    }

    fn render_opacity_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Opacity>,
        env: &Environment,
    ) {
        let alpha = {
            let renderer = unsafe { ctx.renderer() };
            renderer.resolve_animated_scalar(&metadata.value.value)
        };
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
        let needs_redraw = filter.render(&input, &output) || filter.redraw_hint();
        if needs_redraw {
            renderer.request_rebuild();
        }

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
        let (scale_x, scale_y) = {
            let renderer = unsafe { ctx.renderer() };
            (
                renderer.resolve_animated_scalar(&value.x),
                renderer.resolve_animated_scalar(&value.y),
            )
        };
        let transform = vello::kurbo::Affine::translate((center.x, center.y))
            * vello::kurbo::Affine::scale_non_uniform(f64::from(scale_x), f64::from(scale_y))
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
        let radians = {
            let renderer = unsafe { ctx.renderer() };
            f64::from(renderer.resolve_animated_scalar(&value.angle)).to_radians()
        };
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
        let (offset_x, offset_y) = {
            let renderer = unsafe { ctx.renderer() };
            (
                renderer.resolve_animated_scalar(&value.x),
                renderer.resolve_animated_scalar(&value.y),
            )
        };
        let transform = vello::kurbo::Affine::translate((f64::from(offset_x), f64::from(offset_y)));
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
            let rounded =
                vello::kurbo::RoundedRect::from_rect(ctx.bounds, f64::from(border.corner_radius));
            let stroke = vello::kurbo::Stroke::new(width);
            scene.stroke(&stroke, ctx.transform, brush, None, &rounded);
            return;
        }

        if border.edges.top {
            let top = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                ctx.bounds.y0 + width,
            );
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &top,
            );
        }
        if border.edges.bottom {
            let bottom = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y1 - width,
                ctx.bounds.x1,
                ctx.bounds.y1,
            );
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &bottom,
            );
        }
        if border.edges.leading {
            let leading = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x0 + width,
                ctx.bounds.y1,
            );
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &leading,
            );
        }
        if border.edges.trailing {
            let trailing = vello::kurbo::Rect::new(
                ctx.bounds.x1 - width,
                ctx.bounds.y0,
                ctx.bounds.x1,
                ctx.bounds.y1,
            );
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &trailing,
            );
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

    fn render_focused_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Focused>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let should_focus = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&value.0)
        };
        let renderer = unsafe { ctx.renderer() };
        let start = renderer.text_input_targets.len();
        Self::dispatch_any(ctx, env, content);
        let end = renderer.text_input_targets.len();

        if should_focus {
            if start < end {
                renderer.set_focused_text_input(Some(start));
            }
            return;
        }

        if matches!(
            renderer.focused_text_input.get(),
            Some(index) if index >= start && index < end
        ) {
            renderer.set_focused_text_input(None);
        }
    }

    fn render_hittable_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Hittable>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let enabled = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&value.enabled)
        };
        let renderer = unsafe { ctx.renderer() };
        let pointer_start = renderer.pointer_targets.len();
        let hover_start = renderer.hover_targets.len();
        let scroll_start = renderer.scroll_targets.len();
        let text_start = renderer.text_input_targets.len();

        Self::dispatch_any(ctx, env, content);

        if enabled {
            return;
        }

        renderer.pointer_targets.truncate(pointer_start);
        renderer.hover_targets.truncate(hover_start);
        renderer.scroll_targets.truncate(scroll_start);
        renderer.text_input_targets.truncate(text_start);

        if matches!(
            renderer.focused_text_input.get(),
            Some(index) if index >= text_start
        ) {
            renderer.set_focused_text_input(None);
        }
    }

    fn render_gesture_observer_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<GestureObserver>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let GestureObserver {
            gesture,
            mut action,
            ..
        } = value;
        let bounds = transformed_rect(ctx.transform, ctx.bounds);
        let renderer = unsafe { ctx.renderer() };

        match gesture {
            Gesture::Tap(tap) => {
                if tap.count != 1 {
                    panic!("hydrolysis tap gesture currently supports count == 1");
                }
                renderer.register_pointer_target(bounds, move |point, env| {
                    let mut local_env = env.clone();
                    local_env.insert(TapEvent {
                        location: GesturePoint::new(point.x as f32, point.y as f32),
                        count: 1,
                    });
                    action(&local_env);
                    true
                });
            }
            _ => panic!("hydrolysis gesture variant is not implemented"),
        }

        Self::dispatch_any(ctx, env, content);
    }

    fn render_lifecycle_hook_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<LifeCycleHook>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        match value.lifecycle() {
            LifeCycle::Appear => value.handle(env),
            LifeCycle::Disappear => {
                let renderer = unsafe { ctx.renderer() };
                let slot = renderer.lifecycle_disappear_slot;
                renderer.lifecycle_disappear_slot += 1;
                renderer
                    .lifecycle_disappear_current
                    .insert(slot, DeferredLifeCycleHook::new(value, env.clone()));
            }
            _ => panic!("hydrolysis lifecycle variant is not supported"),
        }
        Self::dispatch_any(ctx, env, content);
    }

    fn render_on_event_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<OnEvent>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let event = value.event();
        let bounds = transformed_rect(ctx.transform, ctx.bounds);
        let renderer = unsafe { ctx.renderer() };
        match event {
            Event::HoverEnter => {
                let mut handler = value;
                renderer.register_hover_enter_target(bounds, move |env| {
                    handler.handle(env);
                    true
                });
            }
            Event::HoverExit => {
                let mut handler = value;
                renderer.register_hover_exit_target(bounds, move |env| {
                    handler.handle(env);
                    true
                });
            }
            _ => panic!("hydrolysis event variant is not supported"),
        }
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

    fn draw_scroll_indicators(
        scene: &mut vello::Scene,
        transform: vello::kurbo::Affine,
        viewport: vello::kurbo::Rect,
        metrics: ScrollMetrics,
        axis: ScrollAxis,
    ) {
        let indicator_color = vello::peniko::Color::new([0.4, 0.4, 0.4, 0.55]);
        match axis {
            ScrollAxis::Vertical | ScrollAxis::All => {
                if metrics.max_y > 0.0 {
                    let track_height = viewport.height();
                    let thumb_height = (track_height
                        * (metrics.viewport_height / metrics.content_height))
                        .clamp(12.0, track_height);
                    let travel = track_height - thumb_height;
                    let progress = if metrics.max_y > 0.0 {
                        metrics.offset_y / metrics.max_y
                    } else {
                        0.0
                    };
                    let thumb_y = viewport.y0 + travel * progress;
                    let thumb = vello::kurbo::RoundedRect::from_rect(
                        vello::kurbo::Rect::new(
                            viewport.x1 - 4.0,
                            thumb_y,
                            viewport.x1 - 1.5,
                            thumb_y + thumb_height,
                        ),
                        1.25,
                    );
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        transform,
                        indicator_color,
                        None,
                        &thumb,
                    );
                }
            }
            _ => {}
        }

        match axis {
            ScrollAxis::Horizontal | ScrollAxis::All => {
                if metrics.max_x > 0.0 {
                    let track_width = viewport.width();
                    let thumb_width = (track_width
                        * (metrics.viewport_width / metrics.content_width))
                        .clamp(12.0, track_width);
                    let travel = track_width - thumb_width;
                    let progress = if metrics.max_x > 0.0 {
                        metrics.offset_x / metrics.max_x
                    } else {
                        0.0
                    };
                    let thumb_x = viewport.x0 + travel * progress;
                    let thumb = vello::kurbo::RoundedRect::from_rect(
                        vello::kurbo::Rect::new(
                            thumb_x,
                            viewport.y1 - 4.0,
                            thumb_x + thumb_width,
                            viewport.y1 - 1.5,
                        ),
                        1.25,
                    );
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        transform,
                        indicator_color,
                        None,
                        &thumb,
                    );
                }
            }
            _ => {}
        }
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
        self.pointer_targets.clear();
        self.hover_targets.clear();
        self.text_input_targets.clear();
        self.scroll_targets.clear();
        self.scene.reset();
    }

    pub fn begin_rebuild_frame(&mut self) {
        self.current_frame_retain.clear();
        self.lifecycle_disappear_current.clear();
        self.lifecycle_disappear_slot = 0;
        self.animation_controller.begin_rebuild_frame();
        self.scroll_controller.begin_rebuild_frame();
    }

    pub fn finish_rebuild_frame(&mut self) {
        self.previous_frame_retain = core::mem::take(&mut self.current_frame_retain);

        let previous_hooks = core::mem::take(&mut self.lifecycle_disappear_previous);
        for (slot, hook) in previous_hooks {
            if !self.lifecycle_disappear_current.contains_key(&slot) {
                hook.call();
            }
        }
        self.lifecycle_disappear_previous = core::mem::take(&mut self.lifecycle_disappear_current);

        if matches!(
            self.focused_text_input.get(),
            Some(index) if index >= self.text_input_targets.len()
        ) {
            self.set_focused_text_input(None);
        }

        self.animation_controller.finish_rebuild_frame();
        self.scroll_controller.finish_rebuild_frame();
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
        self.dispatcher
            .state_mut()
            .set_frame_resources(device, queue);
    }

    pub fn clear_frame_resources(&mut self) {
        self.dispatcher.state_mut().clear_frame_resources();
    }

    pub fn request_rebuild(&self) {
        self.rebuild_requested.set(true);
    }

    pub fn take_rebuild_request(&self) -> bool {
        self.rebuild_requested.replace(false)
    }

    #[must_use]
    pub fn focused_text_input_state(&self) -> Option<TextInputState> {
        let index = self.focused_text_input.get()?;
        let target = self.text_input_targets.get(index)?;
        Some(TextInputState {
            x: target.bounds.x0,
            y: target.bounds.y0,
            width: target.bounds.width().max(1.0),
            height: target.bounds.height().max(1.0),
            purpose: target.purpose,
        })
    }

    pub fn advance_animations(&mut self) -> bool {
        self.animation_controller.tick(Instant::now())
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

    fn ensure_surface_blit_state(
        &mut self,
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        let size = (width, height);
        let needs_recreate = self
            .surface_blit
            .as_ref()
            .is_none_or(|state| state.target_format != target_format || state.size != size);

        if !needs_recreate {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_surface_blit_input"),
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
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let blitter = wgpu::util::TextureBlitter::new(device, target_format);

        self.surface_blit = Some(SurfaceBlitState {
            target_format,
            size,
            _texture: texture,
            view,
            blitter,
        });
    }

    pub fn render_scene_to_surface(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        target_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        if target_format == wgpu::TextureFormat::Rgba8Unorm {
            self.render_scene_to_texture(device, queue, target, width, height);
            return;
        }

        if !matches!(
            target_format.remove_srgb_suffix(),
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
        ) {
            panic!(
                "hydrolysis renderer: unsupported surface format for Vello path: {target_format:?}"
            );
        }

        self.ensure_surface_blit_state(device, target_format, width, height);
        let source_view = {
            let state = self
                .surface_blit
                .as_ref()
                .expect("hydrolysis renderer: missing surface blit state");
            state.view.clone()
        };

        self.render_scene_to_texture(device, queue, &source_view, width, height);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hydrolysis_surface_blit_encoder"),
        });
        self.surface_blit
            .as_ref()
            .expect("hydrolysis renderer: missing surface blit state")
            .blitter
            .copy(device, &mut encoder, &source_view, target);
        queue.submit(std::iter::once(encoder.finish()));
    }

    pub fn handle_pointer_down(
        &mut self,
        x: f32,
        y: f32,
        _button: PointerButton,
        env: &Environment,
    ) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let mut rebuild_requested = false;

        let focused = self
            .text_input_targets
            .iter()
            .enumerate()
            .rev()
            .find(|(_, target)| target.bounds.contains(point))
            .map(|(index, _)| index);
        if self.set_focused_text_input(focused) {
            rebuild_requested = true;
        }

        for target in self.pointer_targets.iter_mut().rev() {
            if target.bounds.contains(point) {
                return (target.action)(point, env) || rebuild_requested;
            }
        }
        rebuild_requested
    }

    pub fn handle_pointer_up(
        &mut self,
        x: f32,
        y: f32,
        _button: PointerButton,
        env: &Environment,
    ) -> bool {
        self.handle_pointer_move(x, y, env)
    }

    pub fn handle_pointer_move(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let mut rebuild_requested = false;
        for target in &mut self.hover_targets {
            let contains = target.bounds.contains(point);
            if contains && !target.hovering {
                target.hovering = true;
                if let Some(on_enter) = target.on_enter.as_mut() {
                    rebuild_requested |= on_enter(env);
                }
            } else if !contains && target.hovering {
                target.hovering = false;
                if let Some(on_exit) = target.on_exit.as_mut() {
                    rebuild_requested |= on_exit(env);
                }
            }
        }
        rebuild_requested
    }

    fn dispatch_text_input_command(&mut self, command: TextInputCommand) -> bool {
        let Some(index) = self.focused_text_input.get() else {
            return false;
        };
        if index >= self.text_input_targets.len() {
            self.set_focused_text_input(None);
            return false;
        }

        let target = &mut self.text_input_targets[index];
        (target.action)(command)
    }

    pub fn handle_text_input(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        self.ime_preedit = None;
        self.dispatch_text_input_command(TextInputCommand::Insert(text.to_owned()))
    }

    pub fn handle_ime_preedit(&mut self, text: &str) -> bool {
        if self.focused_text_input.get().is_none() {
            return false;
        }
        let next = if text.is_empty() {
            None
        } else {
            Some(text.to_owned())
        };
        if self.ime_preedit == next {
            return false;
        }
        self.ime_preedit = next;
        true
    }

    pub fn handle_ime_commit(&mut self, text: &str) -> bool {
        self.ime_preedit = None;
        self.handle_text_input(text)
    }

    pub fn handle_ime_disabled(&mut self) -> bool {
        self.ime_preedit.take().is_some()
    }

    pub fn handle_key(&mut self, key: &KeyCode, modifiers: Modifiers) -> bool {
        if modifiers.control || modifiers.alt || modifiers.super_key {
            return false;
        }

        match key {
            KeyCode::Named(value) if value == "Backspace" => {
                self.ime_preedit = None;
                self.dispatch_text_input_command(TextInputCommand::Backspace)
            }
            KeyCode::Named(_) | KeyCode::Character(_) | KeyCode::Unidentified => false,
        }
    }

    pub fn handle_scroll(&mut self, x: f32, y: f32, dx: f32, dy: f32) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        for target in self.scroll_targets.iter_mut().rev() {
            if target.bounds.contains(point) {
                return (target.action)(dx, dy);
            }
        }
        false
    }

    fn register_pointer_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(vello::kurbo::Point, &Environment) -> bool,
    {
        self.pointer_targets.push(PointerTarget {
            bounds,
            action: Box::new(action),
        });
    }

    fn register_hover_enter_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(&Environment) -> bool,
    {
        self.hover_targets.push(HoverTarget {
            bounds,
            hovering: false,
            on_enter: Some(Box::new(action)),
            on_exit: None,
        });
    }

    fn register_hover_exit_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(&Environment) -> bool,
    {
        self.hover_targets.push(HoverTarget {
            bounds,
            hovering: false,
            on_enter: None,
            on_exit: Some(Box::new(action)),
        });
    }

    fn register_text_input_target<F>(
        &mut self,
        bounds: vello::kurbo::Rect,
        purpose: TextInputPurpose,
        action: F,
    ) where
        F: 'static + FnMut(TextInputCommand) -> bool,
    {
        self.text_input_targets.push(TextInputTarget {
            bounds,
            purpose,
            action: Box::new(action),
        });
    }

    fn register_scroll_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(f32, f32) -> bool,
    {
        self.scroll_targets.push(ScrollTarget {
            bounds,
            action: Box::new(action),
        });
    }
}

impl Drop for HydrolysisRenderer {
    fn drop(&mut self) {
        for (_, hook) in core::mem::take(&mut self.lifecycle_disappear_previous) {
            hook.call();
        }
        for (_, hook) in core::mem::take(&mut self.lifecycle_disappear_current) {
            hook.call();
        }
    }
}

fn estimate_intrinsic_size(
    view: &AnyView,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    if let Some(text) = view.downcast_ref::<Str>() {
        return HydrolysisRenderer::measure_text_intrinsic_size(
            state,
            StyledStr::plain(text.clone()),
            env,
        );
    }

    if let Some(text) = view.downcast_ref::<Native<TextConfig>>() {
        return HydrolysisRenderer::measure_text_intrinsic_size(
            state,
            text.as_inner().content.get(),
            env,
        );
    }

    if let Some(icon) = view.downcast_ref::<Native<SystemIcon>>() {
        return HydrolysisRenderer::measure_text_intrinsic_size(
            state,
            StyledStr::plain(icon.as_inner().name.clone()),
            env,
        );
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
    let mut stops: Vec<vello::peniko::ColorStop> =
        gradient.stops.iter().map(to_peniko_stop).collect();

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
            vello::peniko::Gradient::new_two_point_radial(center, start_radius, center, end_radius)
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

fn resolved_shape_to_path(
    shape: &ResolvedShape,
    bounds: vello::kurbo::Rect,
) -> vello::kurbo::BezPath {
    path_commands_to_path(&shape.commands, bounds)
}

fn path_commands_to_path(
    commands: &[PathCommand],
    bounds: vello::kurbo::Rect,
) -> vello::kurbo::BezPath {
    let width = bounds.width();
    let height = bounds.height();
    let mut path = vello::kurbo::BezPath::new();
    let mut has_current = false;

    for command in commands {
        match command {
            PathCommand::MoveTo { x, y } => {
                path.move_to(vello::kurbo::Point::new(
                    f64::from(*x) * width,
                    f64::from(*y) * height,
                ));
                has_current = true;
            }
            PathCommand::LineTo { x, y } => {
                if !has_current {
                    panic!("PathCommand::LineTo requires an active current point");
                }
                path.line_to(vello::kurbo::Point::new(
                    f64::from(*x) * width,
                    f64::from(*y) * height,
                ));
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

fn transformed_rect(
    transform: vello::kurbo::Affine,
    rect: vello::kurbo::Rect,
) -> vello::kurbo::Rect {
    let points = [
        transform * vello::kurbo::Point::new(rect.x0, rect.y0),
        transform * vello::kurbo::Point::new(rect.x1, rect.y0),
        transform * vello::kurbo::Point::new(rect.x0, rect.y1),
        transform * vello::kurbo::Point::new(rect.x1, rect.y1),
    ];
    let min_x = points
        .iter()
        .fold(f64::INFINITY, |acc, point| acc.min(point.x));
    let min_y = points
        .iter()
        .fold(f64::INFINITY, |acc, point| acc.min(point.y));
    let max_x = points
        .iter()
        .fold(f64::NEG_INFINITY, |acc, point| acc.max(point.x));
    let max_y = points
        .iter()
        .fold(f64::NEG_INFINITY, |acc, point| acc.max(point.y));
    vello::kurbo::Rect::new(min_x, min_y, max_x, max_y)
}

fn lerp_f32(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

fn lerp_f64(from: f64, to: f64, t: f32) -> f64 {
    from + (to - from) * f64::from(t)
}

fn lerp_color(from: [f32; 4], to: [f32; 4], t: f32) -> vello::peniko::Color {
    vello::peniko::Color::new([
        lerp_f32(from[0], to[0], t),
        lerp_f32(from[1], to[1], t),
        lerp_f32(from[2], to[2], t),
        lerp_f32(from[3], to[3], t),
    ])
}

fn inset_rect(rect: vello::kurbo::Rect, dx: f64, dy: f64) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(
        rect.x0 + dx,
        rect.y0 + dy,
        (rect.x1 - dx).max(rect.x0 + dx),
        (rect.y1 - dy).max(rect.y0 + dy),
    )
}

fn circle_arc_path(
    center: vello::kurbo::Point,
    radius: f64,
    start_angle: f64,
    sweep: f64,
) -> vello::kurbo::BezPath {
    let mut path = vello::kurbo::BezPath::new();
    if sweep == 0.0 {
        return path;
    }
    let segments = 64usize;
    let step = sweep / segments as f64;
    let mut angle = start_angle;
    path.move_to(vello::kurbo::Point::new(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
    ));
    for _ in 0..segments {
        angle += step;
        path.line_to(vello::kurbo::Point::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        ));
    }
    path
}

fn draw_input_field(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
) {
    let rounded = vello::kurbo::RoundedRect::from_rect(bounds, 6.0);
    scene.fill(
        vello::peniko::Fill::NonZero,
        transform,
        vello::peniko::Color::new([1.0, 1.0, 1.0, 1.0]),
        None,
        &rounded,
    );
    scene.stroke(
        &vello::kurbo::Stroke::new(1.0),
        transform,
        vello::peniko::Color::new([0.75, 0.75, 0.78, 1.0]),
        None,
        &rounded,
    );
}

fn draw_stepper_button(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
) {
    scene.fill(
        vello::peniko::Fill::NonZero,
        transform,
        vello::peniko::Color::new([0.93, 0.93, 0.95, 1.0]),
        None,
        &vello::kurbo::RoundedRect::from_rect(bounds, 6.0),
    );
}
