//! The retained node structs a [`RenderNode`] variant carries, with their
//! small inherent impls (runtime setup, per-frame effect application).

use super::*;

/// A retained sub-view a native widget owns and re-renders every flush — the
/// solution for a widget's move-only `AnyView` label sub-views (slider min/max
/// labels, menu label, progress label/value label) which cannot be re-dispatched
/// twice. The source `AnyView` is built into a persistent [`RenderNode`] once
/// (going through the same dispatcher path as everything else, so a reactive label
/// inside it reaches its dedicated `Dynamic`/`Text` node and stays live), then
/// laid out and flushed at the label's rect each frame.
pub(crate) struct RetainedSubview {
    /// The source view, taken on first build (`AnyView` is move-only).
    source: Option<AnyView>,
    /// The built child node, re-laid-out + re-flushed at the label rect each frame.
    node: Option<RenderNode>,
    /// The size the node was last laid out at, so layout re-runs only on a change.
    laid_out: Size,
    /// A structural patch replaced content inside the retained node, so the new
    /// subtree must be laid out even when its outer rect did not change.
    needs_layout: bool,
    /// The default spoken accessibility label extracted from the source view once,
    /// at build time (mirrors `GestureObserverEffect::default_a11y_label`): the
    /// node owns the source after build, so the per-frame a11y path reads this.
    default_a11y_label: Option<String>,
}

impl RetainedSubview {
    pub(crate) fn new(source: AnyView) -> Self {
        Self {
            source: Some(source),
            node: None,
            laid_out: Size::zero(),
            needs_layout: true,
            default_a11y_label: None,
        }
    }

    /// Eagerly build the sub-view's node now (the caller has the renderer). Used
    /// at tree-build time so the later measure path — which only has `&mut
    /// HydroState`, not the renderer — can measure the already-built node.
    pub(crate) fn ensure_built(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        if self.node.is_none()
            && let Some(view) = self.source.take()
        {
            // Extract the default a11y label from the source before it is consumed
            // by `build` (the node owns the view afterward).
            #[cfg(feature = "accessibility")]
            {
                self.default_a11y_label = renderer.accessibility_label_from_view(&view, env);
            }
            // Normalize as the container/collection build paths do, so a layout
            // view (stack/spacer/etc.) inside a label lowers to its native form.
            let view = normalize_layout_view(view, env);
            self.node = Some(RenderNode::build(view, env, renderer));
        }
    }

    /// The default spoken a11y label extracted from the source at build time.
    pub(crate) fn default_a11y_label(&self) -> Option<String> {
        self.default_a11y_label.clone()
    }

    /// Transform the still-unbuilt source view (e.g. apply a default foreground
    /// color before build). Panics if the node has already been built — the source
    /// is consumed at first build, so this must run before any flush/measure.
    pub(crate) fn map_source(&mut self, f: impl FnOnce(AnyView) -> AnyView) {
        let source = self.source.take().expect(
            "RetainedSubview::map_source must run before the sub-view is built (source consumed)",
        );
        self.source = Some(f(source));
    }

    /// Measure the sub-view's intrinsic size (building it once if needed), the
    /// node analogue of [`measure_view_intrinsic`] at the unspecified proposal.
    /// For the render path, which has the renderer to build on first use.
    pub(crate) fn measure_intrinsic(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
    ) -> Size {
        self.ensure_built(renderer, env);
        self.measure_built(&mut renderer.state, env)
    }

    /// Measure an already-built sub-view's intrinsic size with only `&mut
    /// HydroState` — the measure-path analogue (no renderer to build on). The node
    /// must already be built (via [`Self::ensure_built`]); an unbuilt one measures
    /// as zero, matching an empty label.
    pub(crate) fn measure_built(&self, state: &mut HydroState, env: &Environment) -> Size {
        let Some(node) = &self.node else {
            return Size::zero();
        };
        node.measure(state, env, ProposalSize::UNSPECIFIED).size
    }

    /// Measure an already-built sub-view at a concrete proposal — the variant for
    /// content-filling sub-views (map/webview) whose composed body wraps text at the
    /// proposed width. Returns the full [`ViewDimensions`]; an unbuilt one measures
    /// as zero.
    pub(crate) fn measure_built_with_proposal(
        &self,
        state: &mut HydroState,
        env: &Environment,
        proposal: ProposalSize,
    ) -> Size {
        let Some(node) = &self.node else {
            return Size::zero();
        };
        node.measure(state, env, proposal).size
    }

    /// Patch and measure a retained sub-view under a proposal, returning its
    /// stretch contract alongside the dimensions. Lazy stacks use this for
    /// visible items so a connected `Dynamic` is measured through the retained
    /// node that owns its current content, rather than by re-measuring the
    /// already-connected source view.
    pub(crate) fn patch_and_measure(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
        proposal: ProposalSize,
    ) -> (Size, StretchAxis) {
        self.ensure_built(renderer, env);
        let Some(node) = &mut self.node else {
            return (Size::zero(), StretchAxis::None);
        };
        self.needs_layout |= Self::patch_built(node, renderer);
        (
            node.measure(&mut renderer.state, env, proposal).size,
            node.stretch(),
        )
    }

    /// Stretch contract of an already-built retained sub-view.
    pub(crate) fn stretch_axis(&self) -> StretchAxis {
        self.node
            .as_ref()
            .map_or(StretchAxis::None, RenderNode::stretch)
    }

    fn collect_dynamic_identities_into(&self, out: &mut FxHashSet<usize>) {
        if let Some(node) = &self.node {
            node.collect_dynamic_identities_into(out);
        }
    }

    /// Apply pending reactive structural changes (`Dynamic` content, collection
    /// membership) inside a built sub-view tree. The window refresh pump only
    /// patches the window's own node tree — a widget-owned sub-view is its own
    /// retained tree root, so its flush must run the same patch step or a
    /// `Dynamic`/collection nested in a widget (e.g. a drawer collection inside a
    /// navigation split's sidebar) never applies its pending update. A structural
    /// change is reported to the renderer so the next refresh frame runs the
    /// full prune cycle for the dropped subtrees' animation/measurement slots.
    fn patch_built(node: &mut RenderNode, renderer: &mut HydrolysisRenderer) -> bool {
        let structural = node.patch(renderer);
        if structural {
            renderer.note_subview_structural_change();
        }
        structural
    }

    /// Applies a pending structural update while the owning window tree is already
    /// inside its normal pre-layout patch pass.
    ///
    /// Unlike [`Self::patch_built`], this does not carry the change into another
    /// frame: the parent tree's current patch result already owns the structural
    /// bookkeeping and will lay out the updated child immediately.
    fn patch_for_parent(&mut self, renderer: &mut HydrolysisRenderer) -> bool {
        let structural = self.node.as_mut().is_some_and(|node| node.patch(renderer));
        self.needs_layout |= structural;
        structural
    }

    /// Build (once), patch, lay out (when the rect size or the structure
    /// changed), and flush the sub-view at `rect` under `env`. A zero-area rect
    /// renders nothing, matching the dispatch path's empty-rect guard.
    pub(crate) fn flush_in_rect(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        rect: vello::kurbo::Rect,
    ) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        self.ensure_built(renderer, env);
        let Some(node) = &mut self.node else {
            return;
        };
        let structural = Self::patch_built(node, renderer);
        #[allow(clippy::cast_possible_truncation)]
        let size = Size::new(rect.width() as f32, rect.height() as f32);
        self.needs_layout |= structural;
        if self.needs_layout || size != self.laid_out {
            node.layout(renderer, env, size);
            self.laid_out = size;
            self.needs_layout = false;
        }
        let child_ctx = ctx.child(
            vello::kurbo::Affine::translate((rect.x0, rect.y0)),
            vello::kurbo::Rect::new(0.0, 0.0, rect.width(), rect.height()),
        );
        node.flush(renderer, child_ctx, env);
    }

    /// Build (once), lay out at `size` (only when it changes), and flush the
    /// sub-view under a caller-supplied [`RenderContext`] — the variant for a
    /// sub-view drawn under a non-translation transform (the text-field floating
    /// label's animated translate + scale). The caller composes the transform via
    /// [`RenderContext::child`] and passes the local layout `size` the node should
    /// lay out at; a zero-area size renders nothing.
    pub(crate) fn flush_in_ctx(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        size: Size,
    ) {
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        self.ensure_built(renderer, env);
        let Some(node) = &mut self.node else {
            return;
        };
        let structural = Self::patch_built(node, renderer);
        self.needs_layout |= structural;
        if self.needs_layout || size != self.laid_out {
            node.layout(renderer, env, size);
            self.laid_out = size;
            self.needs_layout = false;
        }
        node.flush(renderer, ctx, env);
    }

    /// Build (once), lay out at `size`, and flush the sub-view into a fresh,
    /// standalone [`vello::Scene`] in identity (local) coordinates — the retained
    /// analogue of [`HydrolysisRenderer::render_subtree_scene`] for a node that
    /// must survive across flushes (the navigation-stack root). The renderer's
    /// scene is swapped out, the node flushes into the temporary scene, then the
    /// scene is swapped back, so the returned scene can be replayed by the
    /// navigation transition (cross-fade `from`/`to`) without re-dispatch.
    pub(crate) fn render_built_scene(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
        size: Size,
    ) -> NavigationCapturedScene {
        self.ensure_built(renderer, env);
        let mut scene = vello::Scene::new();
        let Some(node) = &mut self.node else {
            return NavigationCapturedScene::default();
        };
        let structural = Self::patch_built(node, renderer);
        self.needs_layout |= structural;
        if self.needs_layout || size != self.laid_out {
            node.layout(renderer, env, size);
            self.laid_out = size;
            self.needs_layout = false;
        }
        let local_ctx = RenderContext::with_transforms(
            vello::kurbo::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
            vello::kurbo::Affine::IDENTITY,
            vello::kurbo::Affine::IDENTITY,
        );
        renderer.begin_navigation_scene_capture();
        core::mem::swap(renderer.scene_mut(), &mut scene);
        node.flush(renderer, local_ctx, env);
        core::mem::swap(renderer.scene_mut(), &mut scene);
        renderer.finish_navigation_scene_capture(scene)
    }

    /// Renders a retained navigation page that is not currently interactive.
    /// This is used to prepare the immediately preceding page for an edge-swipe
    /// pop without registering hidden hit-test or accessibility targets.
    pub(crate) fn render_built_navigation_scene_inactive(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
        size: Size,
    ) -> NavigationCapturedScene {
        let previous_hit_test_opacity = renderer.hit_test.hit_test_opacity;
        renderer.hit_test.hit_test_opacity = 0.0;
        #[cfg(feature = "accessibility")]
        renderer.push_accessibility_suppression();
        let scene = self.render_built_scene(renderer, env, size);
        #[cfg(feature = "accessibility")]
        renderer.pop_accessibility_suppression();
        renderer.hit_test.hit_test_opacity = previous_hit_test_opacity;
        scene
    }
}

/// A cache of retained node sub-views for a *virtualized* collection (a lazy
/// stack, list, or table): only items in the current visible window are built and
/// retained, keyed by a stable identity, so a long collection costs only its
/// visible rows. Items are built lazily as they scroll into view and evicted once
/// they leave the visible set — matching virtualization, where scrolled-away item
/// state is intentionally not preserved. While an item stays visible its node is reused,
/// so its reactive content stays live through the node's own per-frame re-flush.
pub(crate) struct VisibleSubviewCache<K: Eq + core::hash::Hash + Clone> {
    entries: std::collections::HashMap<K, RetainedSubview>,
    /// Keys touched during the in-progress frame; [`Self::end_frame`] evicts the rest.
    touched: std::collections::HashSet<K>,
}

impl<K: Eq + core::hash::Hash + Clone> VisibleSubviewCache<K> {
    pub(crate) fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            touched: std::collections::HashSet::new(),
        }
    }

    /// Begin a frame: forget which keys were visible last frame.
    pub(crate) fn begin_frame(&mut self) {
        self.touched.clear();
    }

    /// Get-or-build the retained sub-view for `key`, marking it visible this frame.
    /// `build` produces the item's source view; it runs only the first time an id
    /// becomes visible (or after it was evicted and scrolled back).
    pub(crate) fn entry(
        &mut self,
        key: K,
        build: impl FnOnce() -> AnyView,
    ) -> &mut RetainedSubview {
        self.touched.insert(key.clone());
        self.entries
            .entry(key)
            .or_insert_with(|| RetainedSubview::new(build()))
    }

    /// Look up an already-retained item without marking it visible this frame.
    pub(crate) fn get(&self, key: &K) -> Option<&RetainedSubview> {
        self.entries.get(key)
    }

    /// Patches every currently retained (therefore visible) item before its
    /// virtualized parent is measured.
    pub(crate) fn patch_for_parent(&mut self, renderer: &mut HydrolysisRenderer) -> bool {
        self.entries.values_mut().fold(false, |changed, entry| {
            entry.patch_for_parent(renderer) | changed
        })
    }

    /// Add every connected `Dynamic` owned by a visible retained item.
    pub(crate) fn collect_dynamic_identities_into(&self, out: &mut FxHashSet<usize>) {
        for entry in self.entries.values() {
            entry.collect_dynamic_identities_into(out);
        }
    }

    /// Evict every sub-view not touched this frame (items scrolled out of view).
    pub(crate) fn end_frame(&mut self) {
        let touched = &self.touched;
        self.entries.retain(|key, _| touched.contains(key));
    }
}

/// A transparent metadata wrapper node: it carries the effect to re-apply each
/// flush, the environment its subtree was built under (effect colors and a11y
/// read env every frame), and the child node it recurses into.
pub(crate) struct WrapperNode {
    pub(super) effect: WrapperEffect,
    pub(super) env: Environment,
    pub(super) child: RenderNode,
}

/// The type-erased behavior of one retained native widget state allocation.
pub(crate) trait WidgetBehavior {
    /// Re-renders the leaf from its retained state.
    fn render(
        self: Rc<Self>,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
    );

    /// Measures the leaf from its retained state.
    fn measure(
        &self,
        state: &mut HydroState,
        proposal: ProposalSize,
        env: &Environment,
    ) -> ViewDimensions;
}

/// A native widget leaf rendered every flush from one retained state allocation.
/// The behavior reuses the widget's existing render and measure functions, which
/// re-read live signals and re-emit interaction targets and accessibility at the
/// current bounds. No bake, no capture-once freeze.
pub(crate) struct WidgetNode {
    pub(super) behavior: Rc<dyn WidgetBehavior>,
    pub(super) stretch: StretchAxis,
    pub(super) env: Environment,
}

/// The per-flush effect a [`WrapperNode`] re-applies around its child. Each
/// variant defers to the matching `apply_*` helper in `metadata.rs`, so the
/// effect logic is shared byte-for-byte with the dispatch path.
pub(super) enum WrapperEffect {
    NavigationTransitionSource(RawId),
    NavigationTransitionDestination(RawId),
    Clip(ClipShape),
    Border(Border),
    Shadow(Shadow),
    Cursor(Cursor),
    Draggable(Draggable),
    DropDestination(DropDestinationHandles),
    ContextMenu(ResolvedContextMenu),
    /// Conditional hit-testing: renders the child, then truncates the interaction
    /// targets it registered when disabled. The bookkeeping counts targets across
    /// the (node-flushed) child render, so reactive descendants stay live.
    Hittable(Hittable),
    /// A hover-enter/move/exit handler re-registered every flush. The handler is
    /// shared so the node can re-register the same `OnEvent` each frame.
    OnEvent(Rc<RefCell<OnEvent>>),
    /// A gesture observer (tap/long-press/drag/…). The two pieces the dispatch
    /// path derives from the (now node-owned) content are resolved at build time
    /// and stored in the effect; see [`GestureObserverEffect`].
    GestureObserver(GestureObserverEffect),
    /// A `.focused(binding)` modifier targeting exactly one text input in the
    /// wrapped subtree. Re-applied every flush through [`apply_focused`]: the
    /// binding is read via `read_signal` (so a focus change schedules a frame) and
    /// the registered text input's `focus_binding`/focus state is set from the
    /// child's just-flushed targets — reactive descendants reach their own nodes.
    Focused(Focused),
    /// An `.on_appear`/`.on_disappear` lifecycle hook, owned by this node rather
    /// than by a frame-ordered slot, so it cannot drift onto another subtree's
    /// hook. Appear fires after the child's first flush; disappear
    /// fires from this effect's [`Drop`] when the node leaves the retained tree (a
    /// `Dynamic` / collection reconcile that drops the subtree, or app teardown).
    LifeCycle(LifeCycleEffect),
}

/// The node-owned state of a lifecycle hook (see [`WrapperEffect::LifeCycle`]).
/// An appear hook is consumed after the child's first flush; a disappear hook is
/// fired exactly once when the node is dropped, so structural presence/removal —
/// not a frame-diff slot cursor — drives lifecycle events.
pub(crate) struct LifeCycleEffect {
    pub(super) appear: Cell<Option<DeferredLifeCycleHook>>,
    pub(super) disappear: Option<DeferredLifeCycleHook>,
}

impl Drop for LifeCycleEffect {
    fn drop(&mut self) {
        if let Some(hook) = self.disappear.take() {
            hook.call();
        }
    }
}

/// The build-resolved state of a `.gesture(...)` observer, shared by the dispatch
/// handler and the retained `Wrapper` node. A node has no `content: AnyView` at
/// flush, so the two pieces the dispatch path derives from `content` are resolved
/// at build time and stored here: `default_a11y_label` (the default spoken label,
/// via `accessibility_label_from_view`) and `gesture_group_identity` (via
/// `gesture_group_identity`). The action is shared (`Rc<RefCell<…>>`) so the node
/// can re-register the same action every flush.
pub(crate) struct GestureObserverEffect {
    pub(crate) gesture: Gesture,
    pub(crate) action: Rc<RefCell<BoxedAction<()>>>,
    #[cfg(feature = "accessibility")]
    pub(crate) default_a11y_label: Option<String>,
    pub(crate) gesture_group_identity: usize,
}

pub(crate) struct ColorNode {
    pub(crate) color: Computed<ResolvedColor>,
}

pub(crate) struct TextNode {
    pub(crate) content: Computed<StyledStr>,
    pub(crate) alignment: Computed<HorizontalAlignment>,
}

pub(crate) struct ContainerNode {
    pub(crate) layout: Box<dyn Layout>,
    pub(crate) children: Vec<RenderNode>,
    #[cfg(feature = "accessibility")]
    pub(crate) accessibility_child_env: Option<Environment>,
    /// Child frames cached by [`RenderNode::layout`]; reused by
    /// [`RenderNode::flush`] so a geometry-static frame pays only re-encode.
    pub(crate) placed: Vec<Rect>,
    /// Precise layout-signal subscriptions owned by this retained container.
    pub(crate) _guards: Vec<BoxWatcherGuard>,
}

/// An animated-opacity wrapper: re-samples its alpha each flush and pushes a
/// layer around the child. Layout-transparent (the child measures/places as if
/// the wrapper were absent), matching the SwiftUI/WaterUI transform model.
pub(crate) struct OpacityNode {
    pub(crate) value: Opacity,
    pub(crate) child: RenderNode,
}

pub(crate) struct ScaleNode {
    pub(crate) value: Scale,
    pub(crate) child: RenderNode,
}

pub(crate) struct RotationNode {
    pub(crate) value: Rotation,
    pub(crate) child: RenderNode,
}

pub(crate) struct OffsetNode {
    pub(crate) value: Offset,
    pub(crate) child: RenderNode,
}

pub(crate) struct ScrollNode {
    pub(super) axis: ScrollAxis,
    pub(super) child: RenderNode,
    pub(super) controller: Option<ScrollController<Point>>,
    pub(super) applied_scroll_generation: Cell<i32>,
    /// Scroll handle bound at layout (offset persists across frames; scroll
    /// events mutate it via the registered scroll target).
    pub(super) handle: Option<ScrollHandle>,
    /// Full content extent the child is laid out at.
    pub(super) content_size: Size,
    /// The scroll viewport (the node's own bounds).
    pub(super) viewport: Size,
    /// Environment captured at build, for scroll-target accessibility.
    pub(super) env: Environment,
}

pub(crate) struct RetainNode {
    pub(super) _retain: Retain,
    pub(super) child: RenderNode,
}

pub(crate) struct EnvNode {
    /// The scoped environment this subtree was built under, used to override the
    /// inherited environment at every measure/layout/flush.
    pub(super) env: Environment,
    pub(super) child: RenderNode,
}

pub(crate) struct SceneViewNode {
    /// The owned scene content, re-drawn each flush (it reads its own reactive
    /// inputs in `build_scene`). `RefCell` because `build_scene` needs `&mut` but
    /// `flush` takes `&self`.
    pub(super) content: RefCell<Box<dyn waterui_graphics::SceneContent>>,
}

/// An embedded `GpuSurface` leaf that OWNS its `EmbeddedGpuSurfaceRuntime`
/// (textures, setup state, redraw handle) — the node analogue of
/// [`SceneViewNode`], for a `Native<GpuSurface>` reached through the retained
/// tree. Identity is structural: a reactive swap builds a fresh node with a
/// fresh runtime, and a per-frame re-flush re-binds the *same* runtime via an
/// `Rc`-carrying compositor layer, so there is no cursor-ordered slot to desync.
/// The runtime is shared (`Rc<RefCell<…>>`) with the renderer's node-surface
/// registry so its off-thread redraw handle is polled even on frames that do not
/// re-flush the tree.
pub(crate) struct GpuSurfaceNode {
    pub(super) runtime: Rc<RefCell<EmbeddedGpuSurfaceRuntime>>,
}

/// A `ViewEffect` leaf that OWNS its `ViewEffectRuntime` (the effect renderer +
/// setup state) and builds its captured child as a persistent [`RenderNode`], so
/// reactive descendants inside the effect's content reach their own dedicated
/// nodes and stay live. Each flush renders the child node into an input texture,
/// runs the effect into an output texture, and draws the output image — mirroring
/// the dispatch path's `render_view_effect` exactly, but with no cursor-bound
/// effect slot.
pub(crate) struct ViewEffectNode {
    pub(super) runtime: Rc<RefCell<ViewEffectRuntime>>,
    /// The effect's content, built once as a persistent node (recursed into, not
    /// baked), re-rendered into the input texture each flush.
    pub(super) child: RefCell<RenderNode>,
    /// The size `child` was last laid out at, so layout re-runs only on a change.
    pub(super) laid_out: Cell<Size>,
    pub(super) env: Environment,
}

/// An `AppliedFilter` metadata wrapper that OWNS its `AppliedFilterRuntime`
/// (input/output textures, setup state, output image) and builds its wrapped
/// child as a persistent [`RenderNode`]. Layout-transparent: it measures, lays
/// out, and patches the child exactly as the child would on its own. Each flush
/// renders the child into the runtime's input texture, runs the filter into the
/// output texture, and draws the resulting image — reusing the runtime's
/// texture-reuse logic verbatim, with no cursor-bound effect slot.
pub(crate) struct AppliedFilterNode {
    pub(super) runtime: Rc<RefCell<AppliedFilterRuntime>>,
    pub(super) child: RenderNode,
    pub(super) env: Environment,
}

impl GpuSurfaceNode {
    /// Push a GPU-surface compositor layer that carries the node-owned runtime by
    /// `Rc` (no cursor-ordered slot). Mirrors the dispatch path's
    /// [`HydrolysisRenderer::render_gpu_surface`] exactly, but with an `Owned`
    /// layer source so a per-frame re-flush re-binds the same runtime.
    pub(crate) fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext) {
        let hit_rect = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.push_gpu_surface_layer(
            GpuSurfaceSource::Owned(Rc::clone(&self.runtime)),
            ctx.transform,
            ctx.bounds,
            hit_rect,
        );
        let runtime = Rc::clone(&self.runtime);
        renderer.register_trackpad_pan_target(hit_rect, move |dx, dy, phase| {
            runtime.borrow_mut().handle_trackpad_pan(dx, dy, phase)
        });
    }
}

impl ViewEffectNode {
    /// Render the captured child node into an input texture, run the effect into
    /// an output texture, and draw the output image — the node analogue of the
    /// dispatch path's [`HydrolysisRenderer::render_view_effect`], with the
    /// runtime and child owned by this node (no cursor-bound effect slot).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(crate) fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext) {
        let (device, queue) = {
            let (device, queue) = renderer.state().frame_resources();
            (device.clone(), queue.clone())
        };
        if !ViewEffectRuntime::ensure_setup(
            &self.runtime,
            renderer.effect_setup_resources(&device, &queue),
            renderer.frame_signals(),
        ) {
            return;
        }
        let mut runtime = self.runtime.borrow_mut();

        let input_width = (ctx.bounds.width().max(1.0).round()) as u32;
        let input_height = (ctx.bounds.height().max(1.0).round()) as u32;
        let output_size = runtime.effect().output_size();
        let (output_width, output_height) = output_size.compute(input_width, input_height);
        assert!(
            !(output_width == 0 || output_height == 0),
            "hydrolysis ViewEffect requires non-zero output dimensions"
        );

        let (input_texture, input_view) = {
            let (texture, view) = runtime.input_texture(&device, input_width, input_height);
            (texture.clone(), view.clone())
        };
        renderer.render_child_node_to_texture(
            &self.child.borrow(),
            ctx,
            &self.env,
            ChildTextureTarget {
                texture: &input_texture,
                view: &input_view,
                format: wgpu::TextureFormat::Rgba8Unorm,
                width: input_width,
                height: input_height,
            },
        );

        let (output_texture, output_view) = {
            let (texture, view) = runtime.output_texture(&device, output_width, output_height);
            (texture.clone(), view.clone())
        };

        let input = ViewEffectInput {
            device: &device,
            queue: &queue,
            texture: &input_texture,
            view: input_view,
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: input_width,
            height: input_height,
        };
        let output = ViewEffectOutput {
            device: &device,
            queue: &queue,
            texture: &output_texture,
            view: output_view,
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: output_width,
            height: output_height,
        };
        let needs_redraw = runtime.effect_mut().render(&input, &output);
        if needs_redraw {
            renderer.signals.request_refresh();
        }

        let image = runtime.register_output_image(
            &mut renderer.vello_renderer,
            output_texture,
            output_width,
            output_height,
        );
        drop(runtime);
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
}

impl AppliedFilterNode {
    /// Render the wrapped child node into the runtime's input texture, run the
    /// filter into the output texture, and draw the resulting image — the node
    /// analogue of the dispatch path's
    /// [`HydrolysisRenderer::render_applied_filter_metadata`], reusing the
    /// runtime's texture-reuse logic verbatim, with no cursor-bound effect slot.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(crate) fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext) {
        let (device, queue) = {
            let (device, queue) = renderer.state().frame_resources();
            (device.clone(), queue.clone())
        };
        if !AppliedFilterRuntime::ensure_setup(
            &self.runtime,
            renderer.effect_setup_resources(&device, &queue),
            renderer.frame_signals(),
        ) {
            return;
        }

        let width = (ctx.bounds.width().max(1.0).round()) as u32;
        let height = (ctx.bounds.height().max(1.0).round()) as u32;
        // A tree flush always recaptures the child: whole-scene redraw is the
        // renderer's contract, and skipping the capture is exactly how a
        // filtered subtree freezes at stale pixels. The redraw-only refresh
        // path (which never re-flushes the tree) is the one place the cached
        // input is legitimately reused.
        let (input_texture, input_view) = {
            let mut runtime = self.runtime.borrow_mut();
            let (texture, view) = runtime.input_texture(&device, width, height);
            (texture.clone(), view.clone())
        };
        let capture_started_at = Instant::now();
        renderer.render_child_node_to_texture(
            &self.child,
            ctx,
            &self.env,
            ChildTextureTarget {
                texture: &input_texture,
                view: &input_view,
                format: wgpu::TextureFormat::Rgba8Unorm,
                width,
                height,
            },
        );
        renderer.frame_applied_filter_capture += capture_started_at.elapsed();

        let effect_started_at = Instant::now();
        let (image, needs_redraw) = self.runtime.borrow_mut().render_output(
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
}

pub(crate) struct DynamicHostNode {
    /// The source `Dynamic`, kept alive so its identity cannot be reused while
    /// this node lives — otherwise a freed identity could be reallocated to a
    /// different `Dynamic` and confused for this one. Also read by
    /// [`RenderNode::collect_dynamic_identities`] to keep the measure-path dynamic
    /// dimension cache pruned to the identities still live in the retained tree.
    pub(super) source: waterui_core::dynamic::Dynamic,
    /// Latest content delivered by the `Dynamic`, awaiting a patch.
    pub(super) pending: Rc<RefCell<Option<AnyView>>>,
    /// Environment captured at build, used to rebuild the child on a change.
    pub(super) env: Environment,
    /// The current expansion of the `Dynamic`'s content.
    pub(super) child: RenderNode,
}

impl TextNode {
    /// Emit this text leaf's accessibility node, mirroring
    /// [`Native<TextConfig>::accessibility`] so the render-tree path produces the
    /// same a11y tree the dispatch path did. Called from `flush` with the node's
    /// scoped environment (label/role resolution reads env).
    #[cfg(feature = "accessibility")]
    pub(super) fn emit_accessibility(
        &self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        styled: &StyledStr,
        env: &Environment,
    ) {
        if env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden)
        {
            return;
        }
        let plain = styled.to_plain().to_string();
        let default_label = (!plain.is_empty()).then_some(plain);
        let Some(label) = renderer.resolve_accessibility_label(env, default_label) else {
            return;
        };
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Label),
        );
        node.set_label(label);
        let _ = renderer.register_accessibility_node(
            node,
            transformed_rect(ctx.hit_transform, ctx.bounds),
            env,
            None,
        );
    }

    #[cfg(not(feature = "accessibility"))]
    #[allow(
        clippy::unused_self,
        reason = "parity with the accessibility-enabled signature"
    )]
    pub(super) fn emit_accessibility(
        &self,
        _renderer: &mut HydrolysisRenderer,
        _ctx: RenderContext,
        _styled: &StyledStr,
        _env: &Environment,
    ) {
    }
}

/// Emit an `Image`-role accessibility node for a self-drawn graphics leaf
/// (`Canvas`/`SceneView`, `GpuSurface`, shapes/gradients) at its bounds, reading
/// the role/label scoped into `env` by any `.a11y_role()` / `.a11y_label()`
/// wrappers. These leaves draw their own pixels, so the node tree is the only place
/// their semantic node can be emitted — mirroring `TextNode::emit_accessibility`
/// for the text leaf. Suppressed when the subtree is accessibility-hidden.
#[cfg(feature = "accessibility")]
pub(super) fn emit_graphics_image_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    env: &Environment,
) {
    if env
        .get::<AccessibilityHidden>()
        .is_some_and(AccessibilityHidden::is_hidden)
    {
        return;
    }
    let mut node = AccessibilityNode::new(
        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Image),
    );
    if let Some(label) = renderer.resolve_accessibility_label(env, None) {
        node.set_label(label);
    }
    let _ = renderer.register_accessibility_node(
        node,
        transformed_rect(ctx.hit_transform, ctx.bounds),
        env,
        None,
    );
}

#[cfg(not(feature = "accessibility"))]
pub(super) fn emit_graphics_image_accessibility(
    _renderer: &mut HydrolysisRenderer,
    _ctx: RenderContext,
    _env: &Environment,
) {
}
