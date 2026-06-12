//! Type-erased view dispatch: the handler table, `HydroNativeView`
//! registration, and recursion-depth-guarded dispatch entry points.

use super::*;

pub(crate) type HydroRawHandlerFn =
    Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &mut dyn Any, &Environment)>;
pub(crate) type HydroBoxedHandlerFn =
    Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, AnyView, &Environment)>;

pub(crate) struct HydroHandlerEntry {
    raw: HydroRawHandlerFn,
    boxed: HydroBoxedHandlerFn,
}

#[derive(Clone, Default)]
pub(crate) struct HydroDispatcher {
    handlers: Rc<FxHashMap<core::any::TypeId, HydroHandlerEntry>>,
}

impl core::fmt::Debug for HydroDispatcher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroDispatcher")
            .field("handlers", &self.handlers.len())
            .finish()
    }
}

impl HydroDispatcher {
    pub(super) fn new() -> Self {
        Self {
            handlers: Rc::new(FxHashMap::default()),
        }
    }

    pub(super) fn register<V: View>(
        &mut self,
        handler: impl 'static + Clone + Fn(&mut HydrolysisRenderer, RenderContext, V, &Environment),
    ) {
        let handlers = Rc::get_mut(&mut self.handlers).unwrap_or_else(|| {
            panic!("hydrolysis dispatcher handlers cannot be mutated after cloning")
        });
        let h_raw = handler.clone();
        let h_boxed = handler;
        handlers.insert(
            core::any::TypeId::of::<V>(),
            HydroHandlerEntry {
                raw: Box::new(move |renderer, ctx, slot: &mut dyn Any, env| {
                    let view = slot
                        .downcast_mut::<Option<V>>()
                        .expect("hydrolysis raw dispatch type mismatch")
                        .take()
                        .expect("hydrolysis raw dispatch view already taken");
                    h_raw(renderer, ctx, view, env);
                }),
                boxed: Box::new(move |renderer, ctx, view: AnyView, env| {
                    let view = *view
                        .downcast::<V>()
                        .expect("hydrolysis boxed dispatch type mismatch");
                    h_boxed(renderer, ctx, view, env);
                }),
            },
        );
    }

    pub(super) fn register_renderer<V: View>(
        &mut self,
        handler: impl 'static + Clone + Fn(&mut HydrolysisRenderer, RenderContext, V, &Environment),
    ) {
        self.register(handler);
    }

    pub(super) fn dispatch<V: View>(
        &self,
        renderer: &mut HydrolysisRenderer,
        view: V,
        env: &Environment,
        ctx: RenderContext,
    ) {
        let type_id = core::any::TypeId::of::<V>();

        if type_id == core::any::TypeId::of::<AnyView>() {
            let mut slot = Some(view);
            let any_view = (&mut slot as &mut dyn Any)
                .downcast_mut::<Option<AnyView>>()
                .expect("hydrolysis AnyView downcast should succeed")
                .take()
                .expect("hydrolysis AnyView option should contain a value");
            self.dispatch_boxed(renderer, any_view, env, ctx);
            return;
        }

        if let Some(entry) = self.handlers.get(&type_id) {
            let mut slot = Some(view);
            (entry.raw)(renderer, ctx, &mut slot as &mut dyn Any, env);
            return;
        }

        let body = AnyView::new(view.body(env));
        self.dispatch_boxed(renderer, body, env, ctx);
    }

    pub(super) fn dispatch_boxed(
        &self,
        renderer: &mut HydrolysisRenderer,
        view: AnyView,
        env: &Environment,
        ctx: RenderContext,
    ) {
        let type_id = view.type_id();
        if let Some(entry) = self.handlers.get(&type_id) {
            (entry.boxed)(renderer, ctx, view, env);
            return;
        }

        let body = AnyView::new(view.body(env));
        self.dispatch_boxed(renderer, body, env, ctx);
    }
}

pub(crate) trait HydroNativeView: View + Sized + 'static {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment);
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize;
    fn accessibility_is_render_driven() -> bool {
        false
    }
    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        _proposal: ProposalSize,
    ) -> ViewDimensions {
        ViewDimensions::new(Self::intrinsic(state, view, env))
    }
    fn accessibility(
        _renderer: &mut HydrolysisRenderer,
        _ctx: RenderContext,
        _view: &Self,
        _env: &Environment,
    ) {
    }
}

#[cfg(feature = "accessibility")]
pub(crate) fn register_native_view<V: HydroNativeView>(dispatcher: &mut HydroDispatcher) {
    dispatcher.register::<V>(|renderer, ctx, view, env| {
        let accessibility_is_render_driven = V::accessibility_is_render_driven();
        let hidden_from_accessibility = env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden);
        if hidden_from_accessibility {
            renderer.push_accessibility_suppression();
            if !accessibility_is_render_driven {
                V::accessibility(renderer, ctx, &view, env);
            }
            let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
            V::render(&mut widget_ctx, view, env);
            renderer.pop_accessibility_suppression();
            return;
        }
        if !accessibility_is_render_driven {
            V::accessibility(renderer, ctx, &view, env);
        }
        let suppress_descendants = env
            .get::<AccessibilityChildren>()
            .is_some_and(AccessibilityChildren::excludes_descendants);
        if suppress_descendants && !accessibility_is_render_driven {
            renderer.push_accessibility_suppression();
            let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
            V::render(&mut widget_ctx, view, env);
            renderer.pop_accessibility_suppression();
            return;
        }
        let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
        V::render(&mut widget_ctx, view, env);
    });
}

#[cfg(not(feature = "accessibility"))]
pub(crate) fn register_native_view<V: HydroNativeView>(dispatcher: &mut HydroDispatcher) {
    dispatcher.register::<V>(|renderer, ctx, view, env| {
        V::accessibility(renderer, ctx, &view, env);
        let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
        V::render(&mut widget_ctx, view, env);
    });
}

pub(crate) fn dimensions_for_native<V: HydroNativeView>(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> Option<ViewDimensions> {
    view.downcast_ref::<V>()
        .map(|native| V::dimensions(state, native, env, proposal))
}

macro_rules! hydro_native_view_types {
    ($macro:ident) => {
        $macro!(Native<()>);
        $macro!(Native<Spacer>);
        $macro!(Native<TextConfig>);
        $macro!(Native<FixedContainer>);
        $macro!(Native<LazyContainer>);
        $macro!(Native<ScrollView>);
        $macro!(Native<NavigationView>);
        $macro!(Native<NavigationSplitLayout>);
        $macro!(Native<NavigationStack<(), ()>>);
        $macro!(Native<Tabs>);
        $macro!(Native<BadgeConfig>);
        $macro!(Native<ListConfig>);
        $macro!(Native<TableConfig>);
        $macro!(Native<ButtonConfig>);
        $macro!(Native<ResolvedMenu>);
        $macro!(Native<ToggleConfig>);
        $macro!(Native<SliderConfig>);
        $macro!(Native<StepperConfig>);
        $macro!(Native<ProgressConfig>);
        $macro!(Native<ColorPickerConfig>);
        $macro!(Native<DatePickerConfig>);
        $macro!(Native<ResolvedTextFieldConfig>);
        $macro!(Native<SecureFieldConfig>);
        $macro!(Native<PickerConfig>);
        $macro!(Native<Dynamic>);
        $macro!(Native<SystemIcon>);
        $macro!(Native<GpuSurface>);
        $macro!(Native<SceneView>);
        $macro!(Native<ViewEffectErased>);
        $macro!(Native<ResolvedColor>);
        $macro!(Native<ResolvedGradient>);
        $macro!(Native<ResolvedShape>);
        $macro!(Native<ResolvedMorphShape>);
        $macro!(Native<MapConfig>);
        $macro!(WebView);
    };
}

pub(crate) fn is_hydro_native_view(view: &AnyView) -> bool {
    macro_rules! check_native_view {
        ($ty:ty) => {
            if view.downcast_ref::<$ty>().is_some() {
                return true;
            }
        };
    }
    hydro_native_view_types!(check_native_view);
    false
}

pub(crate) fn dimensions_for_known_native_views(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> Option<ViewDimensions> {
    macro_rules! try_native_dimensions {
        ($ty:ty) => {
            if let Some(dimensions) = dimensions_for_native::<$ty>(view, proposal, state, env) {
                return Some(dimensions);
            }
        };
    }
    hydro_native_view_types!(try_native_dimensions);
    None
}

impl HydrolysisRenderer {
    pub(super) fn register_core_handlers(dispatcher: &mut HydroDispatcher) {
        dispatcher.register_renderer::<Str>(Self::render_str);
        dispatcher
            .register_renderer::<Divider>(crate::widgets::divider::render_divider_with_renderer);
        macro_rules! register_native {
            ($ty:ty) => {
                register_native_view::<$ty>(dispatcher);
            };
        }
        hydro_native_view_types!(register_native);

        dispatcher.register_renderer::<Metadata<Environment>>(Self::render_environment_metadata);
        dispatcher.register_renderer::<Metadata<Retain>>(Self::render_retain_metadata);
        dispatcher.register_renderer::<Metadata<Opacity>>(Self::render_opacity_metadata);
        dispatcher
            .register_renderer::<Metadata<AppliedFilter>>(Self::render_applied_filter_metadata);
        dispatcher.register_renderer::<Metadata<Scale>>(Self::render_scale_metadata);
        dispatcher.register_renderer::<Metadata<Rotation>>(Self::render_rotation_metadata);
        dispatcher.register_renderer::<Metadata<Offset>>(Self::render_offset_metadata);
        dispatcher.register_renderer::<Metadata<ClipShape>>(Self::render_clip_shape_metadata);
        dispatcher.register_renderer::<Metadata<Border>>(Self::render_border_metadata);
        dispatcher.register_renderer::<Metadata<Shadow>>(Self::render_shadow_metadata);
        dispatcher.register_renderer::<Metadata<Focused>>(Self::render_focused_metadata);
        dispatcher.register_renderer::<Metadata<Hittable>>(Self::render_hittable_metadata);
        dispatcher.register_renderer::<Metadata<Cursor>>(Self::render_cursor_metadata);
        dispatcher
            .register_renderer::<Metadata<GestureObserver>>(Self::render_gesture_observer_metadata);
        dispatcher
            .register_renderer::<Metadata<LifeCycleHook>>(Self::render_lifecycle_hook_metadata);
        dispatcher.register_renderer::<Metadata<OnEvent>>(Self::render_on_event_metadata);

        Self::register_passthrough_metadata::<Secure>(dispatcher);
        Self::register_passthrough_metadata::<StandardDynamicRange>(dispatcher);
        Self::register_passthrough_metadata::<HighDynamicRange>(dispatcher);
        Self::register_passthrough_metadata::<IgnoreSafeArea>(dispatcher);
        Self::register_passthrough_metadata::<ContextMenu>(dispatcher);
        dispatcher
            .register_renderer::<Metadata<ResolvedContextMenu>>(Self::render_context_menu_metadata);
        dispatcher.register_renderer::<Metadata<Draggable>>(Self::render_draggable_metadata);
        dispatcher
            .register_renderer::<Metadata<DropDestination>>(Self::render_drop_destination_metadata);
        Self::register_passthrough_metadata::<Background>(dispatcher);

        Self::register_passthrough_ignorable_metadata::<MaterialBackground>(dispatcher);
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityLabel>>(
            Self::render_accessibility_label_metadata,
        );
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityRole>>(
            Self::render_accessibility_role_metadata,
        );
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityHidden>>(
            Self::render_accessibility_hidden_metadata,
        );
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityChildren>>(
            Self::render_accessibility_children_metadata,
        );
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityState>>(
            Self::render_accessibility_state_metadata,
        );
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityStateSignal>>(
            Self::render_accessibility_state_signal_metadata,
        );
    }

    pub(super) fn register_passthrough_metadata<T: MetadataKey>(dispatcher: &mut HydroDispatcher) {
        dispatcher.register_renderer::<Metadata<T>>(Self::render_passthrough_metadata::<T>);
    }

    pub(super) fn register_passthrough_ignorable_metadata<T: MetadataKey>(
        dispatcher: &mut HydroDispatcher,
    ) {
        dispatcher.register_renderer::<IgnorableMetadata<T>>(
            Self::render_passthrough_ignorable_metadata::<T>,
        );
    }

    pub(super) fn push_render_depth(&mut self) {
        self.render_depth = self
            .render_depth
            .checked_add(1)
            .expect("hydrolysis render depth overflow");
    }

    pub(super) fn next_hit_test_order(&mut self) -> usize {
        self.hit_test.next_hit_test_order()
    }

    pub(super) fn pop_render_depth(&mut self) {
        self.render_depth = self
            .render_depth
            .checked_sub(1)
            .expect("hydrolysis render depth underflow");
    }

    pub(super) fn dispatch_with_render_depth<V: View>(
        &mut self,
        view: V,
        env: &Environment,
        ctx: RenderContext,
    ) {
        assert!(
            self.render_depth < 256,
            "hydrolysis render dispatch exceeded recursion budget for {}",
            core::any::type_name::<V>()
        );
        self.push_render_depth();
        let dispatcher = self.dispatcher.clone();
        dispatcher.dispatch(self, view, env, ctx);
        self.pop_render_depth();
    }

    pub(super) fn dispatch_boxed_with_render_depth(
        &mut self,
        view: AnyView,
        env: &Environment,
        ctx: RenderContext,
    ) {
        assert!(
            self.render_depth < 256,
            "hydrolysis render dispatch exceeded recursion budget for {}",
            view.name()
        );
        tracing::trace!(
            target: "waterui::hydrolysis::dispatch",
            depth = self.render_depth,
            view = view.name(),
            bounds = ?ctx.bounds,
            "dispatch"
        );
        self.push_render_depth();
        let dispatcher = self.dispatcher.clone();
        dispatcher.dispatch_boxed(self, view, env, ctx);
        self.pop_render_depth();
    }

    pub(super) fn replay_target_depth(
        &self,
        subtree_depth_base: usize,
        target_depth: usize,
    ) -> usize {
        let relative_depth = target_depth
            .checked_sub(subtree_depth_base)
            .expect("hydrolysis dynamic subtree target depth underflow");
        self.render_depth
            .checked_add(relative_depth)
            .expect("hydrolysis dynamic subtree target depth overflow")
    }

    pub(crate) fn dispatch_any(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) {
        renderer.dispatch_boxed_with_render_depth(content, env, ctx);
    }

    pub(crate) fn dispatch_any_without_accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) {
        #[cfg(feature = "accessibility")]
        {
            renderer.push_accessibility_suppression();
            renderer.dispatch_boxed_with_render_depth(content, env, ctx);
            renderer.pop_accessibility_suppression();
        }
        #[cfg(not(feature = "accessibility"))]
        Self::dispatch_any(renderer, ctx, env, content);
    }

    pub(crate) fn dispatch_in_rect(
        renderer: &mut HydrolysisRenderer,
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
        Self::dispatch_any(
            renderer,
            ctx.child(child_transform, child_bounds),
            env,
            content,
        );
    }

    pub(crate) fn dispatch_in_rect_without_accessibility(
        renderer: &mut HydrolysisRenderer,
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
        Self::dispatch_any_without_accessibility(
            renderer,
            ctx.child(child_transform, child_bounds),
            env,
            content,
        );
    }

    pub(crate) fn render_subtree_scene(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) -> vello::Scene {
        let mut subtree_scene = vello::Scene::new();
        let local_ctx = ctx.with_identity_transforms(vello::kurbo::Rect::new(
            0.0,
            0.0,
            ctx.bounds.width(),
            ctx.bounds.height(),
        ));
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        renderer.dispatch_boxed_with_render_depth(content, env, local_ctx);
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        subtree_scene
    }

    pub fn dispatch<V: View>(&mut self, view: V, env: &Environment, bounds: vello::kurbo::Rect) {
        self.dispatch_with_transform(
            view,
            env,
            bounds,
            vello::kurbo::Affine::IDENTITY,
            vello::kurbo::Affine::IDENTITY,
        );
    }

    pub fn dispatch_with_transform<V: View>(
        &mut self,
        view: V,
        env: &Environment,
        bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        hit_transform: vello::kurbo::Affine,
    ) {
        #[cfg(feature = "accessibility")]
        {
            self.accessibility.root_bounds = transformed_rect(hit_transform, bounds);
        }
        let ctx = RenderContext::with_transforms(bounds, transform, hit_transform);
        self.render_depth = 0;
        self.dispatch_with_render_depth(view, env, ctx);
    }
}
