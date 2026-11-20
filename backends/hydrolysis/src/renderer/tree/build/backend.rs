//! Hydrolysis's [`NativeBackend`]: each payload's retained node, built by
//! the method `View::dispatch` reaches.

use core::marker::PhantomData;

use waterui_backend_core::{
    ChildView, NativeBackend, NativeBackendFamily, Realize, report_unprovided,
};
#[cfg(any(feature = "menu", hydrolysis_video_plane))]
use waterui_core::backend::ChildSlot;
use waterui_core::backend::{ErasedPayload, cast};
use waterui_core::metadata::MetadataKey;
use waterui_core::{NativeView, View};

#[allow(clippy::wildcard_imports)]
use super::*;

/// Why a GPU leaf is not drawn in a build without the GPU painter.
#[cfg(all(waterui_feature = "gpu", not(hydrolysis_gpu)))]
const NO_GPU_PAINTER: &str =
    "this Hydrolysis build has no GPU painter: it was built without the `gpu` feature";

/// Builds retained nodes into one renderer.
pub struct HydroBackend<'r, E: crate::text::TextEngine> {
    renderer: &'r mut SemanticCore<E>,
}

impl<'r, E: crate::text::TextEngine> HydroBackend<'r, E> {
    pub(in crate::renderer::tree) const fn new(renderer: &'r mut SemanticCore<E>) -> Self {
        Self { renderer }
    }

    /// Builds `view` into a node.
    pub(in crate::renderer::tree) fn build(
        &mut self,
        view: impl View,
        env: &Environment,
    ) -> RenderNode<E> {
        self.renderer.state.counters.semantic_builds += 1;
        Realize::child(self, view, env, ())
    }

    /// Builds a payload's child view into a node, counted like [`build`](Self::build).
    pub(in crate::renderer::tree) fn build_child<C: ChildView<Self>>(
        &mut self,
        child: C,
        env: &Environment,
    ) -> RenderNode<E> {
        self.renderer.state.counters.semantic_builds += 1;
        child.realize(self, env, ())
    }

    /// An accessibility wrapper: its content builds under `scoped`, the
    /// environment the wrapper's value installs, held by an `Env` node so
    /// emission reads it every flush and the content stays live.
    fn accessibility_scope(&mut self, content: HeldChild<E>, scoped: Environment) -> RenderNode<E> {
        self.env_scope(content, scoped)
    }

    pub(in crate::renderer::tree) fn env_scope(
        &mut self,
        content: HeldChild<E>,
        scoped: Environment,
    ) -> RenderNode<E> {
        let child = RenderNode::build_slot(content, &scoped, self.renderer);
        RenderNode::Env(Box::new(EnvNode {
            core: self.renderer.new_core(),
            env: scoped,
            child,
        }))
    }

    #[cfg(any(waterui_feature = "calendar", waterui_feature = "multi-date-picker"))]
    fn build_vertical_pair<C: ChildView<Self>>(
        &mut self,
        label: &waterui_controls::label::ControlLabel<C>,
        body: impl View,
        env: &Environment,
    ) -> RenderNode<E> {
        let mut child_env = env.clone();
        child_env.insert(waterui::layout::stack::Axis::Vertical);
        let container = waterui::layout::container::FixedContainer::new(
            waterui::layout::stack::VStackLayout::default(),
            vec![label.build().into_child(), HeldChild::from_view(body)],
        );
        let node = self.fixed_container(container, &child_env, ());
        RenderNode::Env(Box::new(EnvNode {
            core: self.renderer.new_core(),
            env: child_env,
            child: node,
        }))
    }

    /// A payload this build cannot realize: reported, then an empty node.
    fn unrealized(&self, payload: &str, env: &Environment) -> RenderNode<E> {
        report_unprovided("Hydrolysis", payload);
        RenderNode::build_empty(self.renderer, env)
    }
}

// Accessibility metadata are environment-scoping wrappers, so the tree
// models them as an `Env` node carrying the extended environment — the Text
// node and a11y emission read these from env every flush, and the wrapped
// (possibly reactive) content stays live instead of freezing in `Captured`.
// The *naming* wrappers additionally carry a scope identity, so a container
// that no control spoke for can emit the node naming itself. Accessibility
// state is stored as a *live* `AccessibilityStateSignal` (a static state
// becomes a constant signal), resolved at flush time; only the static state
// bakes a subtree-suppressing `AccessibilityHidden`.
macro_rules! accessibility_scopes {
    ($( $method:ident: $ty:ty => $install:expr; )+) => {
        $(
            fn $method<C: ChildView<Self>>(&mut self, metadata: Metadata<$ty, C>, env: &Environment, (): ()) -> RenderNode<E> {
                let install: fn(&Environment, &$ty) -> Environment = $install;
                let scoped = install(env, &metadata.value);
                self.accessibility_scope(metadata.content.into_child(), scoped)
            }
        )+
    };
}

/// Wraps a held child view in an environment scope overriding `Foreground` to `color`.
pub fn held_with_foreground<E: crate::text::TextEngine>(
    content: HeldChild<E>,
    color: waterui::Color,
) -> HeldChild<E> {
    HeldChild::from_fn(move |backend, env, ()| {
        let mut scoped = env.clone();
        waterui_core::plugin::Plugin::install(
            waterui::theme::ForegroundOverride::new(color),
            &mut scoped,
        );
        backend.env_scope(content, scoped)
    })
}

/// [`HydroBackend`] with its renderer borrow left open.
pub struct HydroFamily<E>(PhantomData<E>);

impl<E: crate::text::TextEngine> NativeBackendFamily for HydroFamily<E> {
    type Output = RenderNode<E>;
    type Ctx = ();
    type Backend<'a> = HydroBackend<'a, E>;

    fn reborrow<'r>(backend: &'r mut HydroBackend<'_, E>) -> HydroBackend<'r, E> {
        HydroBackend::new(backend.renderer)
    }
}

impl<E: crate::text::TextEngine> NativeBackend for HydroBackend<'_, E> {
    type Output = RenderNode<E>;
    type Ctx = ();
    type Family = HydroFamily<E>;
    type Offscreen = crate::view_renderer::HydrolysisOffscreenRenderer;

    fn with_family<R>(&mut self, f: impl for<'a> FnOnce(&mut HydroBackend<'a, E>) -> R) -> R {
        f(self)
    }

    fn offscreen(&self, _env: &Environment) -> Self::Offscreen {
        self.renderer.offscreen_renderer()
    }

    accessibility_scope_table!(accessibility_scopes);

    // `Native<()>` carries no data.
    fn empty(&mut self, (): (), env: &Environment, (): ()) -> RenderNode<E> {
        RenderNode::build_empty(self.renderer, env)
    }

    fn window<C: ChildView<Self>>(
        &mut self,
        native: waterui::window::Window<waterui::window::WindowSlot<C>, Option<C>>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        self.renderer
            .open_window(crate::runner::window::slot_root_window(native.map(
                |slot| slot.map(C::into_child),
                |toolbar| toolbar.map(C::into_child),
            )));
        RenderNode::build_empty(self.renderer, env)
    }

    /// Builds the [`RenderNode::Container`] for a fixed container. A
    /// `BackgroundLayout` container's slot 0 is the background the §7.1 fill
    /// rule extends: its fill is wrapped in [`RenderNode::Fill`], the type
    /// §7.1's paint extension records on. The identification happens here,
    /// while the layout type is still concrete; a declaration wrapping the
    /// fill (`.ignore_safe_area` on the fill itself) stops the predicate, so
    /// it replaces the default rather than stacking with it.
    fn fixed_container<C: ChildView<Self>>(
        &mut self,
        native: waterui::layout::container::FixedContainer<Vec<C>>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let background_slot = native.background_slot();
        let (layout, children) = native.into_inner();
        let core = self.renderer.new_core();
        let (layout_dirty, invalidate) = layout_invalidation(&core.cell);
        let guards = layout.watch_invalidation(invalidate);
        #[cfg(feature = "accessibility")]
        let accessibility_child_env = accessibility_container_child_environment(env);
        #[cfg(feature = "accessibility")]
        let child_env = accessibility_child_env.as_ref().unwrap_or(env);
        #[cfg(not(feature = "accessibility"))]
        let child_env = env;
        let children: Vec<RenderNode<E>> = children
            .into_iter()
            .enumerate()
            .map(|(index, child)| {
                let node = self.build_child(child, child_env);
                if Some(index) == background_slot && is_background_fill_leaf(&node) {
                    RenderNode::Fill(Box::new(FillNode::new(node, self.renderer.new_core())))
                } else {
                    node
                }
            })
            .collect();
        RenderNode::Container(Box::new(ContainerNode {
            memo_gate: Cell::default(),
            memo_slots: RefCell::default(),
            accessibility_identity: Rc::new(()),
            core,
            layout,
            children,
            #[cfg(feature = "accessibility")]
            accessibility_child_env,
            placed: Vec::new(),
            #[cfg(feature = "accessibility")]
            geometry: None,
            layout_dirty,
            _guards: guards,
        }))
    }

    // GPU/effect leaves each own their effect runtime directly (textures,
    // setup state, redraw handle), so a reactive swap renders the new content
    // and a per-frame re-flush re-binds the *same* runtime.
    #[cfg(waterui_feature = "gpu")]
    fn gpu(
        &mut self,
        native: waterui::GpuView<Box<dyn waterui::graphics::GpuContent>>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        #[cfg(hydrolysis_gpu)]
        return RenderNode::build_gpu_content(native, self.renderer, env);
        #[cfg(not(hydrolysis_gpu))]
        {
            drop(native);
            unsupported_gpu_device_view("GpuView", NO_GPU_PAINTER);
            RenderNode::build_empty(self.renderer, env)
        }
    }

    #[cfg(waterui_feature = "gpu")]
    fn external_frame(
        &mut self,
        native: waterui::graphics::gpu::external::ExternalFrameView,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        #[cfg(hydrolysis_gpu)]
        return RenderNode::build_external_frame(native, self.renderer, env);
        #[cfg(not(hydrolysis_gpu))]
        {
            drop(native);
            unsupported_gpu_device_view("ExternalFrameView", NO_GPU_PAINTER);
            RenderNode::build_empty(self.renderer, env)
        }
    }

    // `Picture::body` hands Hydrolysis a `SceneView` under the
    // `SceneViewMergeToParent` its runners install; a bare picture means that
    // marker is missing.
    #[cfg(waterui_feature = "gpu")]
    fn picture(
        &mut self,
        _native: waterui::graphics::backend::Picture,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        self.unrealized("a `Picture` outside `SceneViewMergeToParent`", env)
    }

    // Video is the page's `<video>` element on the web and the GPU player
    // (`video-gpu`) elsewhere; a build with neither draws no video.
    #[cfg(waterui_feature = "video")]
    fn video(
        &mut self,
        native: waterui::video::video::NativeVideoConfig,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        #[cfg(hydrolysis_web_video)]
        {
            self.build(crate::widgets::platform::video::video(native), env)
        }
        #[cfg(hydrolysis_gpu_video)]
        {
            self.build(waterui_video_gpu::video(env, native), env)
        }
        #[cfg(not(any(hydrolysis_web_video, hydrolysis_gpu_video)))]
        {
            let _ = native;
            self.unrealized("a `Video` in a build with no video realization", env)
        }
    }

    #[cfg(waterui_feature = "video")]
    fn video_player(
        &mut self,
        native: waterui::video::video::NativeVideoPlayerConfig,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        #[cfg(hydrolysis_web_video)]
        {
            self.build(crate::widgets::platform::video::video_player(native), env)
        }
        #[cfg(hydrolysis_gpu_video)]
        {
            self.build(waterui_video_gpu::video_player(env, native), env)
        }
        #[cfg(not(any(hydrolysis_web_video, hydrolysis_gpu_video)))]
        {
            let _ = native;
            self.unrealized("a `VideoPlayer` in a build with no video realization", env)
        }
    }

    // This backend bridges no platform map: the GPU map (`map-gpu`) draws
    // it, and a build without one draws no map.
    #[cfg(waterui_feature = "map")]
    fn map(&mut self, native: waterui::map::MapConfig, env: &Environment, (): ()) -> RenderNode<E> {
        #[cfg(feature = "map-gpu")]
        {
            self.build(waterui_map_gpu::map(env, native), env)
        }
        #[cfg(not(feature = "map-gpu"))]
        {
            let _ = native;
            unsupported_map();
            RenderNode::build_empty(self.renderer, env)
        }
    }

    #[cfg(waterui_feature = "webview")]
    fn web_view(
        &mut self,
        native: waterui::webview::WebView,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        #[cfg(hydrolysis_cef_webview)]
        {
            if native.handle().is_unavailable() {
                return RenderNode::build_empty(self.renderer, env);
            }
            self.build(waterui_browser_cef::webview(native, env), env)
        }
        #[cfg(hydrolysis_wpe_webview)]
        {
            if native.handle().is_unavailable() {
                return RenderNode::build_empty(self.renderer, env);
            }
            self.build(waterui_browser_wpe::webview(native, env), env)
        }
        #[cfg(not(any(hydrolysis_cef_webview, hydrolysis_wpe_webview)))]
        {
            RenderNode::build_webview(&native, env, self.renderer)
        }
    }

    fn color(&mut self, native: waterui::Color, env: &Environment, (): ()) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::Color(ColorNode {
            bounds: nami::Binding::container(kurbo::Rect::ZERO),
            core: renderer.new_core(),
            color: native.resolve(env),
        })
    }

    fn text(
        &mut self,
        native: waterui::text::TextConfig,
        _env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        let (content, alignment, line_limit) = native.into_parts();
        let core = renderer.new_core();
        let (layout_dirty, invalidate) = layout_invalidation(&core.cell);
        let guards = [
            content.watch({
                let invalidate = Rc::clone(&invalidate);
                move |_| invalidate()
            }),
            alignment.watch(move |_| invalidate()),
        ];
        RenderNode::Text(Box::new(TextNode {
            memo_gate: Cell::default(),
            memo_slots: RefCell::default(),
            accessibility_identity: Rc::new(()),
            core,
            content,
            alignment,
            line_limit: line_limit.map(core::num::NonZeroUsize::get),
            layout_dirty,
            geometry: None,
            _guards: guards,
            tap_span_targets: RefCell::new(Vec::new()),
        }))
    }

    fn lazy_container<C: ChildView<Self>>(
        &mut self,
        native: waterui::layout::container::LazyContainer<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        let direction = native.direction();
        let (layout, children) = native.into_inner();
        // A function pointer, so the mapped collection does not borrow
        // this backend.
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        let children = children.map_views(into_child);
        // The collection's own layout inputs (stack spacing, absolute
        // pins, …) are signals too: subscribe them through
        // `watch_invalidation` like a FixedContainer's, so a change
        // marks the owning node layout-dirty. Which node that is —
        // collection or lazy stack — is decided after the watch
        // installs, so the guard marks through a shared target the
        // built node fills in.
        let layout_mark = Rc::new(RefCell::new(Weak::<NodeCell<E>>::new()));
        let layout_guards = {
            let layout_mark = Rc::clone(&layout_mark);
            layout.watch_invalidation(Rc::new(move || {
                if let Some(cell) = layout_mark.borrow().upgrade() {
                    cell.mark_layout();
                }
            }))
        };
        // A viewport-virtualizable stack layout becomes a virtualized
        // LazyStack; its rows own their transitions themselves, so a
        // row's declaration never decides the realization.
        if let Some(axis) = lazy_stack_axis_config(layout.as_ref(), direction) {
            return RenderNode::build_lazy_stack(
                axis,
                &children,
                env,
                renderer,
                layout_guards,
                &layout_mark,
            );
        }
        // A non-virtualizable layout (AbsoluteLayout/ZStack overlay): a
        // retained reactive collection that reconciles membership by id
        // (recursing into each item, so inner SceneView/Dynamic reach
        // their dedicated nodes).
        RenderNode::build_collection(
            layout,
            &children,
            env,
            renderer,
            layout_guards,
            &layout_mark,
        )
    }

    fn opacity<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::filter::Opacity, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::Opacity(Box::new(OpacityNode::new(&value, renderer, |renderer| {
            RenderNode::build_slot(content, env, renderer)
        })))
    }

    fn scale<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::style::Scale, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::Scale(Box::new(ScaleNode::new(&value, renderer, |renderer| {
            RenderNode::build_slot(content, env, renderer)
        })))
    }

    fn rotation<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::style::Rotation, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::Rotation(Box::new(RotationNode::new(&value, renderer, |renderer| {
            RenderNode::build_slot(content, env, renderer)
        })))
    }

    fn offset<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::style::Offset, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::Offset(Box::new(OffsetNode::new(&value, renderer, |renderer| {
            RenderNode::build_slot(content, env, renderer)
        })))
    }

    fn environment<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<Environment, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        // The snapshot replaces the environment wholesale, so the
        // accessibility naming scope this build installed above it has
        // to be carried across or the name is lost — see
        // `restore_a11y_naming_scope` — and so do the defaults an enclosing
        // scope gives its whole subtree.
        let scoped_env = apply_snapshot_defaults(env, restore_a11y_naming_scope::<E>(env, value));
        // Carry the scoped environment in the node (not flattened away), so
        // it is also the env used at flush/measure/layout — text shaping and
        // a11y read env every frame.
        let child = RenderNode::build_slot(content, &scoped_env, renderer);
        RenderNode::Env(Box::new(EnvNode {
            core: renderer.new_core(),
            env: scoped_env,
            child,
        }))
    }

    fn retain<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::metadata::Retain, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, mut value } = metadata;
        let child = RenderNode::build_slot(content, env, renderer);
        // A `Retain::resolved` value is built from the environment
        // its content's modifier chain resolves in, as a handler's is.
        value.resolve(RenderNode::resolved_handler_env(&child, env));
        RenderNode::Retain(Box::new(RetainNode {
            core: renderer.new_core(),
            _retain: value,
            child,
            #[cfg(hydrolysis_video_plane)]
            plane: None,
        }))
    }

    // A `.transition(...)` declaration: a transparent node the structural
    // site that swaps this content claims from the built instance. One
    // placed directly over a conditional is that `Dynamic` host's own.
    fn transition<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_structural_transition::TransitionSpec, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        let node = RenderNode::build_wrapper(
            WrapperEffect::Transition(super::transition::TransitionDeclaration::new(value)),
            content,
            env,
            renderer,
        );
        if let RenderNode::Wrapper(wrapper) = &node
            && let WrapperEffect::Transition(declaration) = &wrapper.effect
        {
            super::transition::adopt_by_dynamic(&wrapper.child, declaration);
        }
        node
    }

    // `.selected(...)` is strict `Metadata<Selected>` — it scopes into the
    // environment like the a11y metadata above: the outermost interactive
    // control binding under it claims it (`claim_selected`), and the
    // control's `SELECTED` flag and a11y selected state read it.
    fn selected<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::interaction::Selected, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        let scoped = a11y_scoped_env(env, &value);
        let child = RenderNode::build_slot(content, &scoped, renderer);
        RenderNode::Env(Box::new(EnvNode {
            core: renderer.new_core(),
            env: scoped,
            child,
        }))
    }

    // Passthrough metadata: the dispatch handlers discard the value and just
    // render the content (no-ops in Hydrolysis), so the tree unwraps them to
    // the content directly — fully transparent, keeping reactive descendants live.
    fn secure<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::metadata::secure::Secure, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        RenderNode::build_slot(metadata.content, env, renderer)
    }

    // Dynamic-range metadata used to scope a preference read by the retired
    // GPU-surface path; Cherenkov's engine owns headroom per surface, so
    // both pass through to the content like `Secure` above.
    fn standard_dynamic_range<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::metadata::secure::StandardDynamicRange, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        RenderNode::build_slot(metadata.content, env, renderer)
    }

    fn high_dynamic_range<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::metadata::secure::HighDynamicRange, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        RenderNode::build_slot(metadata.content, env, renderer)
    }

    fn ignore_safe_area<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::layout::safe_area::IgnoreSafeArea, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(WrapperEffect::IgnoreSafeArea(value), content, env, renderer)
    }

    // The link hint tells a native list to draw its destination-following
    // affordance. Hydrolysis's list rows draw none, so the hint has
    // nothing to drive here.
    #[cfg(waterui_feature = "navigation")]
    fn navigation_link_hint<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::navigation::NavigationLinkHint, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        RenderNode::build_slot(metadata.content, env, renderer)
    }

    #[cfg(waterui_feature = "navigation")]
    fn navigation_transition_source<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::navigation::NavigationTransitionSource, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(
            WrapperEffect::NavigationTransitionSource(value.id()),
            content,
            env,
            renderer,
        )
    }

    #[cfg(waterui_feature = "navigation")]
    fn navigation_transition_destination<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::navigation::NavigationTransitionDestination, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(
            WrapperEffect::NavigationTransitionDestination(value.id()),
            content,
            env,
            renderer,
        )
    }

    fn material_background<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::background::MaterialBackground, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata {
            content,
            value: MaterialBackground(material),
        } = metadata;
        let Some(level) = crate::renderer::material::WithinWindowLevel::of(material) else {
            return RenderNode::build_slot(content, env, renderer);
        };
        RenderNode::build_wrapper(WrapperEffect::Material(level), content, env, renderer)
    }

    fn glass_background<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::background::GlassBackground, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata {
            content,
            value: GlassBackground(glass),
        } = metadata;
        RenderNode::build_wrapper(
            WrapperEffect::Glass(crate::renderer::glass::GlassEffect::new(glass)),
            content,
            env,
            renderer,
        )
    }

    fn material_group<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::background::MaterialGroup, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        RenderNode::build_wrapper(
            WrapperEffect::MaterialGroup,
            metadata.content,
            env,
            renderer,
        )
    }

    // Transparent metadata wrappers: each applies its visual/interaction
    // effect every flush and recurses into the child node, so reactive
    // descendants reach their dedicated nodes (instead of freezing inside a
    // one-shot `Captured`). The effect is shared with the dispatch path via
    // the `apply_*` helpers in `metadata.rs`.
    fn clip_shape<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::shape::ClipShape, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(WrapperEffect::Clip(value), content, env, renderer)
    }

    fn border<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::border::Border, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(WrapperEffect::Border(value), content, env, renderer)
    }

    fn shadow<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::style::Shadow, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(WrapperEffect::Shadow(value), content, env, renderer)
    }

    fn anchored_overlay<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::metadata::anchored_overlay::AnchoredOverlay, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        let AnchoredOverlay {
            content: overlay_builder,
            is_presented,
            placed_edge,
            placement,
            dismissal,
        } = value;
        RenderNode::build_wrapper(
            WrapperEffect::AnchoredOverlay(AnchoredOverlayEffect {
                builder: overlay_builder,
                presentations: Rc::default(),
                is_presented,
                placed_edge,
                placement,
                dismissal,
                marker: Rc::new(()),
            }),
            content,
            env,
            renderer,
        )
    }

    #[cfg(waterui_feature = "dialog")]
    fn dialog<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::dialog::Dialog, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        let layer = RetainedSubview::new(AnyView::new(value.modal_layer()));
        RenderNode::build_wrapper(
            WrapperEffect::Dialog(DialogEffect {
                content: Rc::new(RefCell::new(layer)),
                dialog: value,
                marker: Rc::new(()),
            }),
            content,
            env,
            renderer,
        )
    }

    fn layout_priority<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::layout::LayoutPriority, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(WrapperEffect::LayoutPriority(value), content, env, renderer)
    }

    fn cursor<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::cursor::Cursor, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(WrapperEffect::Cursor(value), content, env, renderer)
    }

    fn draggable<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::drag_drop::Draggable, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(
            WrapperEffect::Draggable(Rc::new(value)),
            content,
            env,
            renderer,
        )
    }

    fn drop_destination<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::drag_drop::DropDestination, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(
            WrapperEffect::DropDestination(DropDestinationHandles::from_destination(value)),
            content,
            env,
            renderer,
        )
    }

    #[cfg(waterui_feature = "menu")]
    fn resolved_context_menu<S: ChildView<Self>, C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::metadata::context_menu::ResolvedContextMenu<S>, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let content = metadata.content.into_child();
        let renderer = &mut *self.renderer;
        let ResolvedContextMenu {
            items,
            preview,
            accessory,
            dismiss_requests,
        } = metadata.value;
        RenderNode::build_wrapper(
            WrapperEffect::ContextMenu(ContextMenuEffect {
                items,
                dismiss_requests,
                preview: Rc::new(RefCell::new(
                    preview.map(|child| RetainedSubview::held(child.into_child())),
                )),
                accessory: Rc::new(RefCell::new(
                    accessory.map(|child| RetainedSubview::held(child.into_child())),
                )),
            }),
            content,
            env,
            renderer,
        )
    }

    fn hittable<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::interaction::Hittable, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(WrapperEffect::Hittable(value), content, env, renderer)
    }

    fn on_event<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::event::OnEvent, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(
            WrapperEffect::OnEvent(Rc::new(RefCell::new(value))),
            content,
            env,
            renderer,
        )
    }

    fn on_key_press<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::key::OnKeyPress, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(
            WrapperEffect::OnKeyPress(Rc::new(RefCell::new(value))),
            content,
            env,
            renderer,
        )
    }

    fn gesture_observer<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::gesture::GestureObserver, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        let (gesture, action) = value.into_parts();
        let child = RenderNode::build_slot(content, env, renderer);
        let effect = GestureObserverEffect {
            gesture,
            action: Rc::new(RefCell::new(action)),
            gesture_group_identity: child.gesture_group_identity(),
            gesture_target: Cell::new(None),
            chrome: RefCell::new(None),
            chrome_state: nami::Binding::container(
                waterui_core::interaction::InteractionState::empty(),
            ),
        };
        RenderNode::wrap(WrapperEffect::GestureObserver(effect), child, env, renderer)
    }

    fn focused<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::component::focus::Focused, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_wrapper(WrapperEffect::Focused(value), content, env, renderer)
    }

    fn lifecycle_hook<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::event::LifeCycleHook, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let metadata = metadata.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        let Metadata { content, value } = metadata;
        RenderNode::build_lifecycle(value, content, env, renderer)
    }

    fn scroll<C: ChildView<Self>>(
        &mut self,
        native: waterui::layout::scroll::ScrollView<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let (axis, content, controller, offset) = native.into_inner().into_parts();
        let core = self.renderer.new_core();
        let child = self.build_child(content, env);
        // A scroll surface owns §7.1 for its subtree: the layout
        // pass hands the surface its facts and the child lays out
        // with no safe-area context — the surface insets and
        // scrolls its own content instead.
        RenderNode::Scroll(Box::new(ScrollNode {
            memo_gate: Cell::default(),
            memo_slots: RefCell::default(),
            accessibility_identity: Rc::new(()),
            core,
            axis,
            child,
            controller,
            offset,
            applied_scroll_generation: Cell::new(0),
            handle: RefCell::new(None),
            content_size: Size::zero(),
            viewport: Size::zero(),
            non_scrolling_minimum: Cell::new(None),
            env: env.clone(),
            surface: std::rc::Rc::default(),
        }))
    }

    #[cfg(waterui_feature = "gpu")]
    fn scene(
        &mut self,
        native: waterui::graphics::backend::SceneView,
        _env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::build_scene_view_node(native.into_content(), renderer)
    }

    // A canvas mounts through the same scene node as any scene content.
    fn canvas(
        &mut self,
        native: waterui::graphics::canvas::CanvasView,
        _env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        RenderNode::build_scene_view_node(Box::new(native), self.renderer)
    }

    #[cfg(waterui_feature = "effects")]
    fn filtered<C: ChildView<Self>>(
        &mut self,
        native: waterui::graphics::backend::FilteredView<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let filtered = native.map_content(C::into_child);
        let renderer = &mut *self.renderer;
        RenderNode::build_filtered(filtered, env, renderer)
    }

    fn dynamic<C: ChildView<Self>>(
        &mut self,
        native: waterui_core::Dynamic<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::build_dynamic_host(native, C::into_child, env, renderer)
    }

    // A native widget leaf: build a `Widget` node that re-renders it every flush
    // from a retained, signal-holding config (action retained behind `Rc`), so
    // its handler re-reads live signals — reactive labels/values stay live
    // instead of freezing in a one-shot `Captured` bake.
    fn button<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::button::ButtonConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_button(renderer, native.map_slots(into_child), env)
    }

    #[cfg(waterui_feature = "menu")]
    fn menu<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::menu::ResolvedMenu<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_menu(renderer, native.map_slots(into_child), env)
    }

    #[cfg(waterui_feature = "toggle")]
    fn toggle<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::toggle::ToggleConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_toggle(native.map_slots(into_child), env, renderer)
    }

    #[cfg(waterui_feature = "slider")]
    fn slider<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::slider::SliderConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_slider(native.map_slots(into_child), env, renderer)
    }

    #[cfg(waterui_feature = "stepper")]
    fn stepper<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::stepper::StepperConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_stepper(native.map_slots(into_child), env, renderer)
    }

    fn progress<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::progress::ProgressConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_progress(native.map_slots(into_child), env, renderer)
    }

    // Hydrolysis realizes the calendar and the multi-date picker with
    // their grids composed from `WaterUI` views.
    #[cfg(waterui_feature = "calendar")]
    fn calendar<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::calendar::CalendarConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let (label, calendar) = CalendarGrid::new(native).into_parts(env);
        self.build_vertical_pair(&label, calendar, env)
    }

    #[cfg(waterui_feature = "multi-date-picker")]
    fn multi_date_picker<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::picker::multi_date::MultiDatePickerConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let (label, calendar) = MultiDatePickerGrid::new(native, env).into_parts(env);
        self.build_vertical_pair(&label, calendar, env)
    }

    #[cfg(waterui_feature = "date-picker")]
    fn date_picker<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::picker::date::DatePickerConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_date_picker(native.map_slots(into_child), env, renderer)
    }

    #[cfg(waterui_feature = "color-picker")]
    fn color_picker<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::picker::color::ColorPickerConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_color_picker(native.map_slots(into_child), env, renderer)
    }

    #[cfg(waterui_feature = "picker")]
    fn picker<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::picker::PickerConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_picker(native.map_slots(into_child), env, renderer)
    }

    #[cfg(waterui_feature = "text-field")]
    fn text_field<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::text_field::ResolvedTextFieldConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_text_field(native.map_slots(into_child), env, renderer)
    }

    #[cfg(waterui_feature = "secure-field")]
    fn secure_field<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::secure::SecureFieldConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_secure_field(native.map_slots(into_child), env, renderer)
    }

    fn badge<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::badge::BadgeConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_badge(native.map_slots(into_child), env, renderer)
    }

    #[cfg(waterui_feature = "list")]
    fn list<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::list::ListConfig<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        use waterui_core::backend::MapSlots as _;
        let renderer = &mut *self.renderer;
        // A function pointer, so the mapped rows do not borrow this backend.
        let into_child: fn(C) -> HeldChild<E> = C::into_child;
        RenderNode::build_list(native.map_slots(into_child), env, renderer)
    }

    #[cfg(waterui_feature = "table")]
    fn table(
        &mut self,
        native: waterui::component::table::TableConfig,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::build_table(renderer, native, env)
    }

    fn icon(
        &mut self,
        native: waterui::icon::SystemIcon,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        unsupported_system_icon(&native);
        RenderNode::build_empty(renderer, env)
    }

    fn gradient(
        &mut self,
        native: waterui::graphics::Gradient,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::build_gradient(renderer, native, env)
    }

    fn shape(
        &mut self,
        native: waterui::shape::ResolvedShape,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::build_shape(renderer, native, env)
    }

    // Navigation containers (navigation view / split / stack / tabs): each is a
    // persistent `Widget` node re-rendered every flush from a retained config —
    // the navigation controller, transition slot, and tab-bar state stay live
    // and reactive route/tab selection keeps driving updates, instead of
    // freezing in a one-shot `Captured` bake.
    #[cfg(waterui_feature = "navigation")]
    fn navigation_view<C: ChildView<Self>>(
        &mut self,
        native: waterui::navigation::NavigationView<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let mut native = native.map_content(C::into_child);
        native.resolve_native_fields(env);
        let renderer = &mut *self.renderer;
        RenderNode::build_navigation_view(native, env, renderer)
    }

    #[cfg(waterui_feature = "navigation")]
    fn navigation_split<C: ChildView<Self>>(
        &mut self,
        native: waterui::navigation::split::NavigationSplitLayout<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::build_navigation_split(native.map_slots(C::into_child), env, renderer)
    }

    #[cfg(waterui_feature = "navigation")]
    fn navigation_stack<C: ChildView<Self>>(
        &mut self,
        native: waterui::navigation::NavigationStackLayout<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::build_navigation_stack(renderer, native.map_slot(C::into_child), env)
    }

    #[cfg(waterui_feature = "navigation")]
    fn tabs<C: ChildView<Self>>(
        &mut self,
        native: waterui::navigation::tab::TabsLayout<C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::build_tabs(native.map_slots(C::into_child), env, renderer)
    }

    fn spacer(
        &mut self,
        native: waterui::layout::Spacer,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::build_spacer(renderer, native, env)
    }

    // `Divider` and `Str` are registered renderers (not `Native<…>` leaves), so
    // they are downcast as their value type directly and built into persistent
    // `Widget` nodes that re-render from a retained cell each flush.
    fn divider(
        &mut self,
        native: waterui::layout::Divider,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::build_divider(renderer, native, env)
    }

    fn label(&mut self, native: waterui_core::Str, env: &Environment, (): ()) -> RenderNode<E> {
        let renderer = &mut *self.renderer;
        RenderNode::build_str(renderer, native, env)
    }

    // Hydrolysis's own components: a platform-view embedding leaf, the hosted
    // native view, and the Android GPU video player's surface host.
    fn provided<T: NativeView + 'static>(
        &mut self,
        native: T,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        // A platform-view embedding leaf records its frame onto the host's
        // `PlatformViewSink` each flush; mounting the real native child is
        // the host's work.
        let native = match cast::<T, PlatformView>(native) {
            Ok(platform_view) => {
                return RenderNode::build_platform_view(self.renderer, &platform_view, env);
            }
            Err(native) => native,
        };
        #[cfg(all(hydrolysis_hosted, hydrolysis_gpu))]
        let native = match cast::<T, crate::hosted::HostedView>(native) {
            Ok(hosted) => return RenderNode::build_hosted(hosted, env, self.renderer),
            Err(native) => native,
        };
        // The GPU video player's host.
        #[cfg(hydrolysis_video_plane)]
        let native = match cast::<T, waterui_video_gpu::AndroidVideoSurfaceHost>(native) {
            Ok(host) => {
                let picture = crate::runner::android::video_plane::unwrap_host(Native::new(host));
                return RenderNode::build_view(picture, env, self.renderer);
            }
            Err(native) => native,
        };
        #[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
        if self.renderer.semantic_walk {
            let stretch = waterui_core::NativeView::stretch_axis(&native);
            let mut semantics = waterui_backend_core::ProvidedSemantics::default();
            let native = match native.realize_provided(&mut semantics, env) {
                Ok(()) => {
                    return RenderNode::build_provided_semantics(
                        self.renderer,
                        semantics,
                        stretch,
                        env,
                    );
                }
                Err(native) => native,
            };
            drop(native);
            return self.unrealized(core::any::type_name::<T>(), env);
        }
        drop(native);
        self.unrealized(core::any::type_name::<T>(), env)
    }

    fn provided_dyn(
        &mut self,
        mut native: ErasedPayload<'_>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        if let Some(platform_view) = native.take::<PlatformView>() {
            return RenderNode::build_platform_view(self.renderer, &platform_view, env);
        }
        #[cfg(all(hydrolysis_hosted, hydrolysis_gpu))]
        if let Some(hosted) = native.take::<crate::hosted::HostedView>() {
            return RenderNode::build_hosted(hosted, env, self.renderer);
        }
        #[cfg(hydrolysis_video_plane)]
        if let Some(host) = native.take::<waterui_video_gpu::AndroidVideoSurfaceHost>() {
            let picture = crate::runner::android::video_plane::unwrap_host(Native::new(host));
            return RenderNode::build_view(picture, env, self.renderer);
        }
        #[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
        if self.renderer.semantic_walk {
            let mut semantics = waterui_backend_core::ProvidedSemantics::default();
            if native.realize_provided(&mut semantics, env) {
                return RenderNode::build_provided_semantics(
                    self.renderer,
                    semantics,
                    StretchAxis::None,
                    env,
                );
            }
        }
        self.unrealized(native.type_name(), env)
    }

    fn provided_metadata<K: MetadataKey, C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<K, C>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        #[cfg(any(feature = "menu", hydrolysis_video_plane))]
        let metadata = metadata.map_content(C::into_child);
        #[cfg(feature = "menu")]
        let metadata =
            match cast::<Metadata<K, HeldChild<E>>, Metadata<PopupMenuSurface, HeldChild<E>>>(
                metadata,
            ) {
                Ok(metadata) => return self.popup_menu_surface(metadata, env),
                Err(metadata) => metadata,
            };
        // The picture of the GPU video player with the decoder plane the
        // Android backend places below it.
        #[cfg(hydrolysis_video_plane)]
        let metadata = match cast::<
            Metadata<K, HeldChild<E>>,
            Metadata<crate::runner::android::video_plane::VideoPlaneHost, HeldChild<E>>,
        >(metadata)
        {
            Ok(metadata) => return self.video_plane_host(metadata, env),
            Err(metadata) => metadata,
        };
        drop(metadata);
        self.unrealized(core::any::type_name::<K>(), env)
    }

    fn provided_metadata_dyn(
        &mut self,
        metadata: ErasedPayload<'_>,
        env: &Environment,
        (): (),
    ) -> RenderNode<E> {
        #[cfg(any(feature = "menu", hydrolysis_video_plane))]
        let mut metadata = metadata;
        #[cfg(feature = "menu")]
        if let Some(metadata) = metadata.take::<Metadata<PopupMenuSurface, AnyView>>() {
            return self.popup_menu_surface(metadata.map_content(HeldChild::from_view), env);
        }
        #[cfg(hydrolysis_video_plane)]
        if let Some(metadata) = metadata
            .take::<Metadata<crate::runner::android::video_plane::VideoPlaneHost, AnyView>>()
        {
            return self.video_plane_host(metadata.map_content(HeldChild::from_view), env);
        }
        self.unrealized(metadata.type_name(), env)
    }
}

impl<E: crate::text::TextEngine> HydroBackend<'_, E> {
    #[cfg(feature = "menu")]
    fn popup_menu_surface(
        &mut self,
        metadata: Metadata<PopupMenuSurface, HeldChild<E>>,
        env: &Environment,
    ) -> RenderNode<E> {
        RenderNode::build_wrapper(
            WrapperEffect::PopupMenuSurface(metadata.value.selected_row),
            metadata.content,
            env,
            self.renderer,
        )
    }

    #[cfg(hydrolysis_video_plane)]
    fn video_plane_host(
        &mut self,
        metadata: Metadata<crate::runner::android::video_plane::VideoPlaneHost, HeldChild<E>>,
        env: &Environment,
    ) -> RenderNode<E> {
        let Metadata { content, value } = metadata;
        let planes = crate::runner::android::video_plane::VideoPlanes::in_environment(env);
        RenderNode::Retain(Box::new(RetainNode {
            core: self.renderer.new_core(),
            _retain: Retain::new(()),
            child: RenderNode::build_slot(content, env, self.renderer),
            plane: Some(crate::runner::android::video_plane::VideoPlane::new(
                &planes,
                value.into_bridge(),
            )),
        }))
    }
}
