//! The retained node structs a [`RenderNode`] variant carries, with their
//! small inherent impls (runtime setup, per-frame effect application).

#[cfg(feature = "frame-profile")]
use super::layout::{SignatureHasher, hash_size};
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
    /// The selected offer, independent of the cached frame size.
    laid_out_proposal: Option<ProposalSize>,
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
            laid_out_proposal: None,
            needs_layout: true,
            default_a11y_label: None,
        }
    }

    /// Eagerly build the sub-view's node now (the caller has the renderer). Used
    /// at tree-build time so the later measure path — which only has `&mut
    /// HydroState`, not the renderer — can measure the already-built node.
    pub(crate) fn ensure_built(&mut self, renderer: &mut SemanticCore, env: &Environment) {
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

    /// Whether the sub-view's node has been built.
    pub(crate) fn is_built(&self) -> bool {
        self.node.is_some()
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
        if let Some(node) = &mut self.node {
            node.prepare_for_measure(renderer);
        }
        let theme = renderer.theme();
        self.measure_built(&mut renderer.state, env, &theme)
    }

    /// Measure an already-built sub-view's intrinsic size with only `&mut
    /// HydroState` — the measure-path analogue (no renderer to build on). The node
    /// must already be built (via [`Self::ensure_built`]); an unbuilt one measures
    /// as zero, matching an empty label.
    pub(crate) fn measure_built(
        &self,
        state: &mut HydroState,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> Size {
        let Some(node) = &self.node else {
            return Size::zero();
        };
        node.measure(state, env, theme, ProposalSize::UNSPECIFIED)
            .size
    }

    /// Measure an already-built sub-view at a concrete proposal — the variant for
    /// content-filling sub-views (map/webview) whose composed body wraps text at the
    /// proposed width. Returns the full [`ViewDimensions`]; an unbuilt one measures
    /// as zero.
    pub(crate) fn measure_built_with_proposal(
        &self,
        state: &mut HydroState,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> Size {
        let Some(node) = &self.node else {
            return Size::zero();
        };
        node.measure(state, env, theme, proposal).size
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
        node.prepare_for_measure(renderer);
        let theme = renderer.theme();
        (
            node.measure(&mut renderer.state, env, &theme, proposal)
                .size,
            node.stretch(),
        )
    }

    /// Run the layout-time prepare pass over the sub-view's built node, if any.
    /// Forwards to [`RenderNode::prepare_for_measure`]; an unbuilt sub-view has
    /// nothing to prepare.
    pub(crate) fn prepare_for_measure(&mut self, renderer: &mut HydrolysisRenderer) {
        if let Some(node) = &mut self.node {
            node.prepare_for_measure(renderer);
        }
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

    /// Emit this sub-view's accessibility nodes for the semantic walk: builds
    /// the node on first use, applies pending structural patches (a `Dynamic`
    /// inside a widget label is its own retained tree), then walks it. No
    /// layout and no encode — the sub-view emits exactly the nodes it would
    /// under a rendered flush, minus bounds.
    #[cfg(feature = "accessibility")]
    pub(crate) fn emit_accessibility(&mut self, renderer: &mut SemanticCore, env: &Environment) {
        self.ensure_built(renderer, env);
        let Some(node) = &mut self.node else {
            return;
        };
        let _ = Self::patch_built(node, renderer);
        node.emit_accessibility(renderer, env);
    }

    /// Apply pending reactive structural changes (`Dynamic` content, collection
    /// membership) inside a built sub-view tree. The window refresh pump only
    /// patches the window's own node tree — a widget-owned sub-view is its own
    /// retained tree root, so its flush must run the same patch step or a
    /// `Dynamic`/collection nested in a widget (e.g. a drawer collection inside a
    /// navigation split's sidebar) never applies its pending update. A structural
    /// change is reported to the renderer so the next refresh frame runs the
    /// full prune cycle for the dropped subtrees' animation/measurement slots.
    fn patch_built(node: &mut RenderNode, renderer: &mut SemanticCore) -> bool {
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
    fn patch_for_parent(&mut self, renderer: &mut SemanticCore) -> bool {
        let structural = self.node.as_mut().is_some_and(|node| node.patch(renderer));
        self.needs_layout |= structural;
        structural
    }

    /// Consume the subtree's layout-invalidated mark (`false` for an unbuilt
    /// view). The flush sites fold this into `needs_layout` so a layout-signal
    /// change re-places the subtree at its unchanged rect.
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.node
            .as_mut()
            .is_some_and(RenderNode::take_layout_dirty)
    }

    /// Build (once), patch, lay out (when the rect size or the structure
    /// changed), and flush the sub-view at `rect` under `env`. A zero-area rect
    /// renders nothing, matching the dispatch path's empty-rect guard.
    pub(crate) fn flush_in_rect(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        proposal: ProposalSize,
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
        node.prepare_for_measure(renderer);
        #[allow(clippy::cast_possible_truncation)]
        let size = Size::new(rect.width() as f32, rect.height() as f32);
        self.needs_layout |= structural | node.take_layout_dirty();
        if self.needs_layout || size != self.laid_out || self.laid_out_proposal != Some(proposal) {
            node.layout(renderer, env, proposal, size);
            self.laid_out = size;
            self.laid_out_proposal = Some(proposal);
            self.needs_layout = false;
        }
        let child_ctx = ctx.child(
            vello::kurbo::Affine::translate((rect.x0, rect.y0)),
            vello::kurbo::Rect::new(0.0, 0.0, rect.width(), rect.height()),
        );
        // Record the sub-view's root as the owner of whatever its flush
        // registers: a press the caller registered for the whole sub-view
        // carries the same owner, and the ancestry check tells a gesture
        // inside the sub-view from one attached to the root itself.
        if let Some(identity) = node.accessibility_identity() {
            renderer.push_input_owner(&identity);
            node.flush(renderer, child_ctx, env);
            renderer.pop_input_owner();
        } else {
            node.flush(renderer, child_ctx, env);
        }
    }

    /// The retained identity of the built sub-view's root node — the owner the
    /// input path records for a press registered on the sub-view's behalf, so a
    /// gesture registered inside the sub-view is a strict descendant of it and
    /// one attached to the root itself is not. `None` until the sub-view is
    /// built (or when the root carries no identity).
    pub(crate) fn root_accessibility_identity(&self) -> Option<Rc<()>> {
        self.node
            .as_ref()
            .and_then(RenderNode::accessibility_identity)
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
        proposal: ProposalSize,
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
        node.prepare_for_measure(renderer);
        self.needs_layout |= structural | node.take_layout_dirty();
        if self.needs_layout || size != self.laid_out || self.laid_out_proposal != Some(proposal) {
            node.layout(renderer, env, proposal, size);
            self.laid_out = size;
            self.laid_out_proposal = Some(proposal);
            self.needs_layout = false;
        }
        if let Some(identity) = node.accessibility_identity() {
            renderer.push_input_owner(&identity);
            node.flush(renderer, ctx, env);
            renderer.pop_input_owner();
        } else {
            node.flush(renderer, ctx, env);
        }
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
        node.prepare_for_measure(renderer);
        self.needs_layout |= structural | node.take_layout_dirty();
        let proposal = ProposalSize::new(Some(size.width), Some(size.height));
        if self.needs_layout || size != self.laid_out || self.laid_out_proposal != Some(proposal) {
            node.layout(renderer, env, proposal, size);
            self.laid_out = size;
            self.laid_out_proposal = Some(proposal);
            self.needs_layout = false;
        }
        let local_ctx = RenderContext::with_transforms(
            vello::kurbo::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
            vello::kurbo::Affine::IDENTITY,
            vello::kurbo::Affine::IDENTITY,
        );
        renderer.begin_navigation_scene_capture();
        renderer.push_lazy_viewport(LazyViewport {
            bounds: local_ctx.bounds,
            transform: local_ctx.transform,
        });
        core::mem::swap(renderer.scene_mut(), &mut scene);
        node.flush(renderer, local_ctx, env);
        core::mem::swap(renderer.scene_mut(), &mut scene);
        renderer.pop_lazy_viewport("retained scene capture");
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

    /// Insert the sub-view for `key` without marking it visible this frame —
    /// the patch-time materialization of an item the visible window covers.
    /// An id the frame's flush never reaches is still evicted by `end_frame`.
    pub(crate) fn materialize(
        &mut self,
        key: K,
        build: impl FnOnce() -> AnyView,
    ) -> &mut RetainedSubview {
        self.entries
            .entry(key)
            .or_insert_with(|| RetainedSubview::new(build()))
    }

    /// Look up an already-retained item without marking it visible this frame.
    pub(crate) fn get(&self, key: &K) -> Option<&RetainedSubview> {
        self.entries.get(key)
    }

    /// Drop the retained sub-views of exactly the keys in `ids`. The next
    /// `entry` for a dropped key re-materializes it from the collection's
    /// current data; keys not in `ids` keep their nodes (and their retained
    /// state) untouched.
    pub(crate) fn invalidate_ids(&mut self, ids: &std::collections::HashSet<K>) {
        self.entries.retain(|key, _| !ids.contains(key));
        self.touched.retain(|key| !ids.contains(key));
    }

    /// Run the layout-time prepare pass over every currently retained item.
    pub(crate) fn prepare_for_measure(&mut self, renderer: &mut HydrolysisRenderer) {
        for entry in self.entries.values_mut() {
            entry.prepare_for_measure(renderer);
        }
    }

    /// Patches every currently retained (therefore visible) item before its
    /// virtualized parent is measured.
    pub(crate) fn patch_for_parent(&mut self, renderer: &mut SemanticCore) -> bool {
        self.entries.values_mut().fold(false, |changed, entry| {
            entry.patch_for_parent(renderer) | changed
        })
    }

    /// Consume the layout-invalidated marks over every currently retained item:
    /// `true` when any `FixedContainer` inside a visible item's subtree asked
    /// for a re-measure since the last consume. See [`RenderNode::take_layout_dirty`].
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.entries
            .values_mut()
            .fold(false, |dirty, entry| entry.take_layout_dirty() | dirty)
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
    pub(super) accessibility_identity: Rc<()>,
    pub(super) effect: WrapperEffect,
    pub(super) env: Environment,
    pub(super) child: RenderNode,
}

/// The type-erased behavior of one retained native widget state allocation.
pub(crate) trait WidgetBehavior {
    /// The default layout priority of this native leaf.
    fn priority(&self) -> i32 {
        0
    }

    /// Whether this leaf draws nothing — WaterUI's empty view `()`.
    ///
    /// This is a semantic answer, not a measured size: a `Spacer` squeezed to
    /// zero still renders and still answers `false`. A stack treats a child
    /// answering `true` as a non-member (§4.4: no slot, no spacing).
    fn renders_nothing(&self) -> bool {
        false
    }

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
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> ViewDimensions;

    /// Runs the leaf's layout-time prepare step: applying theme paint that
    /// could not be resolved at tree-build time (build contexts carry no
    /// theme) and building the retained sub-views the measure path then reads.
    /// [`RenderNode::prepare_for_measure`] runs this over a subtree once per
    /// layout or measure entry, while a `&mut HydrolysisRenderer` — hence the
    /// theme — is in hand, before any node below it is measured. A semantic
    /// runtime never runs this pass, so no paint reaches it.
    fn prepare(&self, _renderer: &mut HydrolysisRenderer, _env: &Environment) {}

    /// Emits this leaf's accessibility node(s) for the semantic walk — the
    /// same tree `render` produces under a `RenderContext`, with no bounds and
    /// no draw or hit-target work. The default emits nothing: leaves with no
    /// semantics (spacer, divider, gradient, …) need no override.
    #[cfg(feature = "accessibility")]
    fn emit_accessibility(self: Rc<Self>, _renderer: &mut SemanticCore, _env: &Environment) {}
}

/// A native widget leaf rendered every flush from one retained state allocation.
/// The behavior reuses the widget's existing render and measure functions, which
/// re-read live signals and re-emit interaction targets and accessibility at the
/// current bounds. No bake, no capture-once freeze.
pub(crate) struct WidgetNode {
    pub(super) accessibility_identity: Rc<()>,
    pub(super) behavior: Rc<dyn WidgetBehavior>,
    pub(super) stretch: StretchAxis,
    pub(super) env: Environment,
}

/// The per-flush effect a [`WrapperNode`] re-applies around its child. Each
/// variant defers to the matching `apply_*` helper in `metadata.rs`, so the
/// effect logic is shared byte-for-byte with the dispatch path.
pub(super) enum WrapperEffect {
    /// Purely a layout hint: it changes which child a stack compresses first and
    /// draws nothing, so the flush path renders straight through it.
    LayoutPriority(LayoutPriority),
    NavigationTransitionSource(RawId),
    NavigationTransitionDestination(RawId),
    Clip(ClipShape),
    Border(Border),
    Shadow(Shadow),
    Cursor(Cursor),
    Draggable(Draggable),
    DropDestination(DropDestinationHandles),
    ContextMenu(ContextMenuEffect),
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
    /// The theme's drawn context-menu surface behind the wrapped menu rows —
    /// the panel a `PopupWindowManager` window shows instead of a view-level
    /// fill (water-rs/hydrolysis#200). Draws nothing on targets that lack a
    /// `draw_text_context_menu_panel` implementation.
    PopupMenuSurface,
    /// An `.anchored_overlay(...)` (water-rs/waterui#1275): every flush the
    /// wrapper registers the anchor's live bounds plus the effect's handles
    /// for the post-flush `render_anchored_overlays` pass, which measures,
    /// places and draws the open overlay at window level above all content.
    AnchoredOverlay(AnchoredOverlayEffect),
}

impl WrapperEffect {
    /// Whether this effect carries a callback that captures an environment —
    /// a hover/`on_tap`/gesture handler, a drop destination, a context menu,
    /// an anchored overlay or a lifecycle hook. Such a handler resolves against
    /// the environment its *content* resolves in (water-rs/waterui#1292), and a
    /// handler wrapper is itself transparent to that resolution when it sits in
    /// an outer handler's modifier chain.
    pub(super) const fn captures_environment(&self) -> bool {
        matches!(
            self,
            Self::OnEvent(_)
                | Self::GestureObserver(_)
                | Self::DropDestination(_)
                | Self::ContextMenu(_)
                | Self::AnchoredOverlay(_)
                | Self::LifeCycle(_)
        )
    }
}

/// The node-owned state of an `.anchored_overlay(...)` wrapper. The content
/// slot is `Rc`-shared because the post-flush render pass — not this node —
/// measures and flushes it, and the node keeps it built across closes.
pub(crate) struct AnchoredOverlayEffect {
    /// The overlay content, lazily built on first open.
    pub(crate) content: Rc<RefCell<Option<RetainedSubview>>>,
    /// The presentation binding: read every flush (subscribing the frame to
    /// it), written `false` by outside-interaction dismissal and by the
    /// anchor leaving the tree.
    pub(crate) is_presented: nami::Binding<bool>,
    /// The placement contract.
    pub(crate) placement: waterui::metadata::anchored_overlay::AnchorPlacement,
    /// What besides the binding closes the overlay.
    pub(crate) dismissal: waterui::metadata::anchored_overlay::Dismissal,
    /// Written with the logical `AnchorEdge` the overlay was placed against
    /// after any flip, on every placement.
    pub(crate) placed_edge: nami::Binding<waterui::metadata::anchored_overlay::AnchorEdge>,
    /// Identity shared with the registration, so the render pass can match an
    /// open overlay to the anchor that emitted it — and tell that an anchor
    /// that stopped registering left the tree.
    pub(crate) marker: Rc<()>,
}

/// The node-owned state of a `.context_menu(...)` wrapper: the resolved menu
/// plus the lifted preview and interactive accessory as retained sub-views.
///
/// The slots are `Rc<RefCell<Option<RetainedSubview>>>` because ownership moves
/// with presentation state, not with the node: a context menu's preview and
/// accessory mount into the open presentation and must be handed back to the
/// node when it closes, so a later open mounts them again. The node keeps them
/// across the rest of the tree's lifetime exactly like a widget's label
/// sub-view — built lazily at the first open, patched and re-flushed per frame
/// while presented.
pub(crate) struct ContextMenuEffect {
    /// The resolved menu items the popup is built from.
    pub(crate) items: nami::Computed<Vec<ResolvedMenuItem>>,
    /// Counter of the accessory's dismiss requests; every change closes the
    /// open menu (water-rs/waterui#1245).
    pub(crate) dismiss_requests: nami::Computed<i32>,
    /// The view lifted over the dimmed backdrop at the source's frame while
    /// the menu is open. `None` means lift the source view itself, realized
    /// as a hole punched in the dim backdrop — the source is still drawn by
    /// the owning tree, so it cannot be re-flushed here.
    pub(crate) preview: Rc<RefCell<Option<RetainedSubview>>>,
    /// The interactive view anchored to the lifted preview, mounted outside
    /// the menu so its own pointer, touch and keyboard input reaches it.
    pub(crate) accessory: Rc<RefCell<Option<RetainedSubview>>>,
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
    /// The target the last flush registered. Every scene emit rebuilds the
    /// engine's target list under `clear_targets`, so the next flush
    /// re-registers this target (same recognizer `Rc`) at the new bounds instead
    /// of building a fresh recognizer — a drag that began before the repaint
    /// keeps running. This is the same retained registration
    /// `ListRenderState::row_gestures` performs per row; the node's identity is
    /// the key, so the recognizer dies with the view rather than leaking.
    pub(crate) gesture_target: Cell<Option<crate::gesture::GestureTarget>>,
}

pub(crate) struct ColorNode {
    pub(crate) color: Computed<ResolvedColor>,
}

pub(crate) struct TextNode {
    /// Per-frame measure memo gate: records whether this node's body was
    /// re-probed within a frame, gating `memo_slots` so a node measured
    /// once per frame pays a `Cell` update instead of a `RefCell` borrow.
    pub(crate) memo_gate: Cell<MemoGate>,
    /// The proposal ring `measure` consults once `memo_gate` marks this
    /// node as re-probed. Owned by the node, so a dropped node never
    /// leaves a stale answer behind.
    pub(crate) memo_slots: RefCell<NodeMeasureEntry>,
    pub(crate) accessibility_identity: Rc<()>,
    pub(crate) content: Computed<StyledStr>,
    pub(crate) alignment: Computed<HorizontalAlignment>,
    /// Maximum laid-out lines, from `TextConfig::line_limit`.
    pub(crate) line_limit: Option<usize>,
}

pub(crate) struct ContainerNode {
    /// Per-frame measure memo gate: records whether this node's body was
    /// re-probed within a frame, gating `memo_slots` so a node measured
    /// once per frame pays a `Cell` update instead of a `RefCell` borrow.
    pub(crate) memo_gate: Cell<MemoGate>,
    /// The proposal ring `measure` consults once `memo_gate` marks this
    /// node as re-probed. Owned by the node, so a dropped node never
    /// leaves a stale answer behind.
    pub(crate) memo_slots: RefCell<NodeMeasureEntry>,
    pub(crate) accessibility_identity: Rc<()>,
    pub(crate) layout: Box<dyn Layout>,
    pub(crate) children: Vec<RenderNode>,
    #[cfg(feature = "accessibility")]
    pub(crate) accessibility_child_env: Option<Environment>,
    /// Child frames cached by [`RenderNode::layout`]; reused by
    /// [`RenderNode::flush`] so a geometry-static frame pays only re-encode.
    pub(crate) placed: Vec<Rect>,
    /// Set by this container's `Layout::watch_invalidation` subscription when a
    /// layout input signal changes (shared with the watcher closure). An outer
    /// `RetainedSubview` consumes it through [`RenderNode::take_layout_dirty`]
    /// so a constraint change re-runs `place` even when the slot's rect and
    /// proposal are unchanged — otherwise the value latched at mount is the
    /// only one the container ever sees.
    pub(crate) layout_dirty: Rc<Cell<bool>>,
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
    /// Per-frame measure memo gate: records whether this node's body was
    /// re-probed within a frame, gating `memo_slots` so a node measured
    /// once per frame pays a `Cell` update instead of a `RefCell` borrow.
    pub(crate) memo_gate: Cell<MemoGate>,
    /// The proposal ring `measure` consults once `memo_gate` marks this
    /// node as re-probed. Owned by the node, so a dropped node never
    /// leaves a stale answer behind.
    pub(crate) memo_slots: RefCell<NodeMeasureEntry>,
    pub(super) accessibility_identity: Rc<()>,
    pub(super) axis: ScrollAxis,
    pub(super) child: RenderNode,
    pub(super) controller: Option<ScrollController<Point>>,
    pub(super) applied_scroll_generation: Cell<i32>,
    /// Scroll handle bound at layout (offset persists across frames; scroll
    /// events mutate it via the registered scroll target). `RefCell` because
    /// the semantic accessibility walk also (re)binds it — a walk takes `&self`.
    pub(super) handle: RefCell<Option<ScrollHandle>>,
    /// Full content extent the child is laid out at.
    pub(super) content_size: Size,
    /// The scroll viewport (the node's own bounds).
    pub(super) viewport: Size,
    /// The content's `0`-probe answer on the non-scrolling axis — the
    /// floor `measure` reports when a container probes the scroll's minimum
    /// (layout-spec.md §6). Cached because the `0` probe runs on every
    /// window-limits pass, including pure replay frames where a live
    /// measure would count as re-measurement; `patch` and
    /// `take_layout_dirty` reset it when the subtree changes underneath.
    pub(super) non_scrolling_minimum: Cell<Option<f32>>,
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
    pub(super) accessibility_identity: Rc<()>,
    /// The owned scene content, re-drawn each flush (it reads its own reactive
    /// inputs in `build_scene`). `RefCell` because `build_scene` needs `&mut` but
    /// `flush` takes `&self`.
    pub(super) content: Rc<RefCell<Box<dyn waterui_graphics::SceneContent>>>,
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
    pub(super) accessibility_identity: Rc<()>,
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
    pub(crate) fn flush(
        &self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        #[cfg(feature = "accessibility")] focus_node: Option<AccessibilityNodeId>,
    ) {
        let hit_rect = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.push_gpu_surface_layer(
            GpuSurfaceSource::Owned(Rc::clone(&self.runtime)),
            ctx.transform,
            ctx.bounds,
            hit_rect,
        );
        // A view that handles its own input receives the pointer, keyboard,
        // IME and scroll events landing on this layer directly, and owns the
        // gesture: it gets raw scroll deltas instead of the pan state
        // `GpuFrame` exposes, so the two never both interpret one gesture.
        if self.runtime.borrow().wants_input_events() {
            renderer.register_surface_input_target(
                ctx.bounds,
                ctx.hit_transform,
                Rc::clone(&self.runtime),
                #[cfg(feature = "accessibility")]
                focus_node,
            );
            return;
        }
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
    /// Flush the wrapped child into the frame's capture atlas, queue the filter
    /// to run from that slot, and draw the filter's output image — the node
    /// analogue of the dispatch path's
    /// [`HydrolysisRenderer::render_applied_filter_metadata`], reusing the
    /// runtime's texture-reuse logic verbatim, with no cursor-bound effect slot.
    ///
    /// The filter itself runs when the atlas level is flushed
    /// ([`HydrolysisRenderer::flush_subtree_captures`]), which happens before
    /// the scene that draws the output image is rendered, so one compositor
    /// pass and one submit serve every filter of the level.
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
        let capture_started_at = Instant::now();
        renderer.capture_child_into_atlas(
            &self.child,
            ctx,
            &self.env,
            &self.runtime,
            width,
            height,
        );
        renderer.frame_applied_filter_capture += capture_started_at.elapsed();

        let image = self.runtime.borrow_mut().prepare_output(
            &device,
            &mut renderer.vello_renderer,
            width,
            height,
        );

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
    /// The current expansion of the `Dynamic`'s content. Interior mutability
    /// lets a pass that runs under `&self` (flush, semantic emit) apply a
    /// pending structural change at this node's own entry, instead of waiting
    /// for the next `patch` walk. The shared cell lets the measurement caches
    /// hold a weak handle by `Dynamic` identity, so a dispatch measure that
    /// meets the connected `Dynamic` can re-measure this child for the real
    /// proposal.
    pub(super) child: Rc<RefCell<RenderNode>>,
    /// A mid-pass swap changed the child after the parent's layout already
    /// placed this host. Read through [`RenderNode::take_layout_dirty`], it
    /// propagates the invalidation to the enclosing retained sub-view, which
    /// re-lays out its tree before its next flush — the caller-imposed rect a
    /// `RetainedSubview` flushes at never renegotiates itself.
    pub(super) layout_dirty: Cell<bool>,
}

impl DynamicHostNode {
    /// Take any content the `Dynamic` delivered since this node's last pass and
    /// rebuild the child with it — the fine-grained structural patch. Runs at
    /// this node's own entry into every pass whose leaf reads reach the frame
    /// (`patch`, `flush`, semantic emit), so a `set()` written inside an
    /// earlier pass of the same frame still lands its structural change before
    /// the stale child can paint or emit — leaf updates and structural
    /// patches land in the same presented frame, never one frame apart.
    ///
    /// Passes whose reads cannot reach the presented frame (`measure`,
    /// `layout`, identity collection) do not apply it: a stale measurement or
    /// placement costs nothing visible — a child swapped in there would be
    /// laid out at the placement its already-measured stale child earned — and
    /// `flush`/`emit` apply the patch before anything is encoded. Returns
    /// whether the child changed.
    pub(super) fn apply_pending(&self, renderer: &mut SemanticCore) -> bool {
        let pending = self.pending.borrow_mut().take();
        match pending {
            Some(content) => {
                let node_env = self.env.clone();
                *self.child.borrow_mut() = RenderNode::build(content, &node_env, renderer);
                true
            }
            None => false,
        }
    }

    /// `apply_pending` for a pass reached after the frame's `patch` result was
    /// already folded into the window's structural bookkeeping: the change is
    /// reported to the renderer like `RetainedSubview::patch_built` reports
    /// widget-owned patches, so the next refresh runs the prune cycle for the
    /// dropped subtree's animation/measurement slots. The swap also marks this
    /// host layout-dirty — the parent's placement of this node preceded the
    /// new child, so an ancestor must re-lay out to size it.
    pub(super) fn apply_pending_mid_pass(&self, renderer: &mut SemanticCore) -> bool {
        let applied = self.apply_pending(renderer);
        if applied {
            self.layout_dirty.set(true);
            renderer.note_subview_structural_change();
        }
        applied
    }
}

impl TextNode {
    /// Emit this text leaf's accessibility node, mirroring
    /// [`Native<TextConfig>::accessibility`] so the render-tree path produces the
    /// same a11y tree the dispatch path did. Called from `flush` with the node's
    /// scoped environment (label/role resolution reads env).
    #[cfg(feature = "accessibility")]
    pub(super) fn emit_accessibility(
        &self,
        renderer: &mut crate::renderer::SemanticCore,
        ctx: Option<RenderContext>,
        styled: &StyledStr,
        env: &Environment,
    ) {
        if env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden)
        {
            return;
        }
        let plain = styled.to_semantic().to_string();
        let default_label = (!plain.is_empty()).then_some(plain);
        let Some(label) = renderer.resolve_accessibility_label(env, default_label) else {
            return;
        };
        if renderer.consume_accessibility_descendant_text(env, &label) {
            return;
        }
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Label),
        );
        node.set_label(label);
        let _ = renderer.register_accessibility_leaf(ctx, node, env, None);
    }

    #[cfg(not(feature = "accessibility"))]
    #[allow(
        clippy::unused_self,
        reason = "parity with the accessibility-enabled signature"
    )]
    pub(super) fn emit_accessibility(
        &self,
        _renderer: &mut crate::renderer::SemanticCore,
        _ctx: Option<RenderContext>,
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
///
/// `default_label` is what the drawing says about itself
/// ([`SceneContent::accessibility_label`](waterui_graphics::SceneContent::accessibility_label)):
/// a formula's `MathML`, say. It names the node only when the application named
/// nothing, so `.a11y_label(…)` still wins.
///
/// `default_value` is the drawing's spoken content beside its name
/// ([`SceneContent::accessibility_value`](waterui_graphics::SceneContent::accessibility_value)),
/// emitted under the same precedence: a scoped `.a11y_value(…)` wins it.
#[cfg(feature = "accessibility")]
pub(super) fn emit_graphics_image_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    env: &Environment,
    default_label: Option<String>,
    default_value: Option<String>,
    focusable: bool,
) -> Option<AccessibilityNodeId> {
    if env
        .get::<AccessibilityHidden>()
        .is_some_and(AccessibilityHidden::is_hidden)
    {
        return None;
    }
    let mut node = AccessibilityNode::new(
        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Image),
    );
    if focusable {
        // A surface that takes input is a keyboard-focus target like any
        // other focusable control: assistive `Focus` requests and Tab
        // traversal reach it through this action.
        node.add_action(AccessibilityAction::Focus);
    }
    if let Some(label) = renderer.resolve_accessibility_label(env, default_label) {
        node.set_label(label);
    }
    if let Some(value) = renderer.resolve_accessibility_value(env, default_value) {
        node.set_value(value);
    }
    renderer.register_accessibility_leaf(ctx, node, env, None)
}

#[cfg(not(feature = "accessibility"))]
pub(super) fn emit_graphics_image_accessibility(
    _renderer: &mut crate::renderer::SemanticCore,
    _ctx: Option<RenderContext>,
    _env: &Environment,
    _default_label: Option<String>,
    _default_value: Option<String>,
    _focusable: bool,
) {
}

#[cfg(feature = "frame-profile")]
impl RetainedSubview {
    /// Contributes this retained sub-view's last layout answer and its node's
    /// placed geometry to the frame's layout digest.
    pub(super) fn signature_into(&self, hasher: &mut SignatureHasher) {
        use std::hash::Hash;
        hash_size(hasher, self.laid_out);
        self.laid_out_proposal.is_some().hash(hasher);
        if let Some(proposal) = self.laid_out_proposal {
            proposal.width.map(f32::to_bits).hash(hasher);
            proposal.height.map(f32::to_bits).hash(hasher);
        }
        if let Some(node) = &self.node {
            node.signature_into(Rect::from_size(self.laid_out), hasher);
        }
    }
}

#[cfg(feature = "frame-profile")]
impl<K: Eq + core::hash::Hash + Clone> VisibleSubviewCache<K> {
    /// Order-independent fold of every retained item's signature — the map's
    /// iteration order is not stable, so per-entry digests combine by sum.
    pub(super) fn signature_into(&self, hasher: &mut SignatureHasher) {
        use std::hash::Hash;
        self.entries.len().hash(hasher);
        let mut combined = 0u64;
        for (key, subview) in &self.entries {
            let mut entry_hasher = SignatureHasher::new();
            key.hash(&mut entry_hasher);
            subview.signature_into(&mut entry_hasher);
            combined = combined.wrapping_add(std::hash::Hasher::finish(&entry_hasher));
        }
        hasher.mix(combined);
    }
}
