//! The retained node structs a [`RenderNode`] variant carries, with their
//! small inherent impls (runtime setup, per-frame effect application).

#[cfg(feature = "frame-profile")]
use super::layout::{SignatureHasher, hash_size};
// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::text::TextLayout as _;
use waterui::gesture::TapGesture;
use waterui_core::transition::TransitionSpec;
use waterui_layout::safe_area::IgnoreSafeArea;
use waterui_text::styled::TapSpan;

/// A retained sub-view a native widget owns and re-renders every flush — the
/// solution for a widget's move-only `AnyView` label sub-views (slider min/max
/// labels, menu label, progress label/value label) which cannot be re-dispatched
/// twice. The source `AnyView` is built into a persistent [`RenderNode`] once
/// (going through the same dispatcher path as everything else, so a reactive label
/// inside it reaches its dedicated `Dynamic`/`Text` node and stays live), then
/// laid out and flushed at the label's rect each frame.
///
/// The lifecycle is [`RetainedSubviewState::Unbuilt`] — the move-only source is
/// held — until [`Self::ensure_built`] transitions it to
/// [`RetainedSubviewState::Built`]. Measure, layout, flush, accessibility and
/// probe paths all require the built state: a pre-build use is a caller
/// ordering bug and panics naming the method, never an empty answer.
pub struct RetainedSubview<E: crate::text::TextEngine> {
    /// The lifecycle state: the unbuilt source, or the built node with its
    /// layout bookkeeping.
    state: RetainedSubviewState<E>,
}

/// The [`RetainedSubview`] lifecycle: the unbuilt source view, or the built
/// node plus the layout bookkeeping its flushes own.
enum RetainedSubviewState<E: crate::text::TextEngine> {
    /// The source view, taken on first build (`AnyView` is move-only).
    Unbuilt { source: AnyView },
    /// The built sub-view: the node and the layout bookkeeping its flushes own.
    Built(Box<BuiltSubview<E>>),
}

/// Everything [`RetainedSubviewState::Built`] carries: the persistent node and
/// the layout bookkeeping that decides whether the next flush re-lays out.
struct BuiltSubview<E: crate::text::TextEngine> {
    /// The built child node, re-laid-out + re-flushed at the label rect each frame.
    node: RenderNode<E>,
    /// The size the node was last laid out at, so layout re-runs only on a change.
    laid_out: Size,
    /// The selected offer, independent of the cached frame size.
    laid_out_proposal: Option<ProposalSize>,
    /// A structural patch replaced content inside the retained node, so the new
    /// subtree must be laid out even when its outer rect did not change.
    needs_layout: bool,
    /// The §7.1 context the node was last laid out against — a change (the
    /// keyboard inset animating under an unchanged rect) re-runs layout so
    /// the subtree's touch tests and surface facts track it.
    laid_out_area: Option<Box<safe_area::SafeAreaLayout>>,
    /// The signal dependencies of the cached layout: every signal the last
    /// layout pass read, kept subscribed for as long as that layout lives —
    /// an update on any of them marks the set dirty so the next flush re-runs
    /// exactly this sub-view's layout (a signal read inside a cached layout,
    /// e.g. a scroll controller's request generation, would otherwise never
    /// reach the sub-view again). `None` only while the set is lent to
    /// [`HydroState::layout_dependencies`] for the duration of a pass.
    layout_dependencies: Option<LayoutDependencies<E>>,
    /// The default spoken accessibility label extracted from the source view once,
    /// at build time (mirrors `GestureObserverEffect::default_a11y_label`): the
    /// node owns the source after build, so the per-frame a11y path reads this.
    default_a11y_label: Option<String>,
}

/// Writes `safe_area` into a node's stored-context slot, overwriting the
/// existing `Box` in place when there is one: a `Some` over `Some` reuses
/// the allocation, so a per-frame re-layout pays no new `Box` — the only
/// allocation is the `None -> Some` transition.
pub(super) fn store_safe_area(
    slot: &mut Option<Box<safe_area::SafeAreaLayout>>,
    safe_area: Option<safe_area::SafeAreaLayout>,
) {
    if let (Some(stored), Some(area)) = (slot.as_deref_mut(), safe_area.as_ref()) {
        stored.clone_from(area);
    } else {
        *slot = safe_area.map(Box::new);
    }
}

impl<E: crate::text::TextEngine> BuiltSubview<E> {
    /// Lays the node out when the structure, the size, the proposal, the
    /// §7.1 context or a signal the last layout pass read changed since the
    /// last layout — recording what it laid out against so an unchanged
    /// re-flush skips the pass.
    fn layout_if_needed(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        env: &Environment,
        safe_area: Option<safe_area::SafeAreaLayout>,
        proposal: ProposalSize,
        size: Size,
    ) {
        if self.needs_layout
            || self
                .layout_dependencies
                .as_ref()
                .is_some_and(LayoutDependencies::is_dirty)
            || size != self.laid_out
            || self.laid_out_proposal != Some(proposal)
            || self.laid_out_area.as_deref() != safe_area.as_ref()
        {
            // Collect this pass's signal reads as dependencies: while the
            // set is installed in `HydroState::layout_dependencies`,
            // `watch_signal` and `measure_signal` record every read into it,
            // so each signal the layout consumed stays watched — and its
            // updates mark the sub-view's layout dirty — for as long as the
            // cached layout lives. Two passes are never live at once
            // (`RenderNode::layout` does not lay out a retained sub-view),
            // so the slot must be empty coming in and full going out.
            let mut dependencies = self
                .layout_dependencies
                .take()
                .expect("hydrolysis renderer: a retained sub-view lost its layout dependency set");
            dependencies.begin_pass();
            // A host may have measured this sub-view earlier in the same
            // flush (a label's `measure_built`, a lazy row's
            // `patch_and_measure`), filling measure memos the pass would
            // hit instead of re-running the reads beneath them — sweeping
            // the dependencies those reads feed. Invalidate them so only
            // memos filled during the pass can answer inside it.
            renderer.state.measurement.begin_dependency_pass();
            assert!(
                renderer.state.layout_dependencies.is_none(),
                "hydrolysis renderer: a retained sub-view's layout pass ran \
                 inside another — RenderNode::layout must not lay out a \
                 retained sub-view"
            );
            renderer.state.layout_dependencies = Some(dependencies);
            self.node
                .layout(renderer, env, safe_area.clone(), proposal, size);
            let mut dependencies =
                renderer.state.layout_dependencies.take().expect(
                    "hydrolysis renderer: a layout pass emptied the sub-view dependency slot",
                );
            dependencies.finish_pass();
            self.layout_dependencies = Some(dependencies);
            self.laid_out = size;
            self.laid_out_proposal = Some(proposal);
            store_safe_area(&mut self.laid_out_area, safe_area);
            self.needs_layout = false;
        }
    }
}

impl<E: crate::text::TextEngine> RetainedSubview<E> {
    pub(crate) const fn new(source: AnyView) -> Self {
        Self {
            state: RetainedSubviewState::Unbuilt { source },
        }
    }

    /// Parents the built node's cell under `owner`: the mark the subtree
    /// raises reaches the owner's `below`, and the placement chain the
    /// node resolves against reads the owner's. The cell is also listed
    /// among `owner`'s subviews, so `retire_unplaced_subviews` at the end
    /// of the owner's record retires the subtree the record left
    /// unplaced. Called once, by `ensure_built`, with the cell reading at
    /// build time — the widget's cell while a record/measuring reader is
    /// installed, the window root otherwise.
    pub(crate) fn attach(&self, owner: &Rc<NodeCell<E>>) {
        let RetainedSubviewState::Built(built) = &self.state else {
            return;
        };
        built.node.core().cell.set_parent(owner);
        owner
            .subviews
            .borrow_mut()
            .push(Rc::downgrade(&built.node.core().cell));
    }

    /// Eagerly build the sub-view's node now (the caller has the renderer),
    /// transitioning [`RetainedSubviewState::Unbuilt`] to
    /// [`RetainedSubviewState::Built`]. Used at tree-build time so the later
    /// measure path — which only has `&mut HydroState`, not the renderer — can
    /// measure the already-built node.
    pub(crate) fn ensure_built(&mut self, renderer: &mut SemanticCore<E>, env: &Environment) {
        let RetainedSubviewState::Unbuilt { source } = &mut self.state else {
            return;
        };
        let view = core::mem::replace(source, AnyView::new(()));
        // Extract the default a11y label from the source before it is consumed
        // by `build` (the node owns the view afterward).
        #[cfg(feature = "accessibility")]
        let default_a11y_label = renderer.accessibility_label_from_view(&view, env);
        #[cfg(not(feature = "accessibility"))]
        let default_a11y_label = None;
        // Normalize as the container/collection build paths do, so a layout
        // view (stack/spacer/etc.) inside a label lowers to its native form.
        let view = normalize_layout_view(view, env);
        let node = RenderNode::build(view, env, renderer);
        let layout_dependencies = Some(LayoutDependencies::new(Rc::downgrade(&node.core().cell)));
        self.state = RetainedSubviewState::Built(Box::new(BuiltSubview {
            node,
            laid_out: Size::zero(),
            laid_out_proposal: None,
            needs_layout: true,
            laid_out_area: None,
            layout_dependencies,
            default_a11y_label,
        }));
        // The current reader — the record or layout pass this build runs
        // inside — is the owning widget's cell; a build with no reader
        // attaches to the window root.
        let owner = renderer
            .reader_cell()
            .unwrap_or_else(|| Rc::clone(renderer.root_cell()));
        self.attach(&owner);
    }

    /// The built sub-view, or a panic naming `method`: measure, layout, flush,
    /// accessibility and probe paths require the built state, so a pre-build
    /// use fails at the call site instead of answering empty.
    fn expect_built(&self, method: &'static str) -> &BuiltSubview<E> {
        let RetainedSubviewState::Built(built) = &self.state else {
            panic!("RetainedSubview::{method} ran before the sub-view was built");
        };
        built
    }

    /// The built sub-view mutably — the same ordering contract as
    /// [`Self::expect_built`].
    fn expect_built_mut(&mut self, method: &'static str) -> &mut BuiltSubview<E> {
        let RetainedSubviewState::Built(built) = &mut self.state else {
            panic!("RetainedSubview::{method} ran before the sub-view was built");
        };
        built
    }

    /// The default spoken a11y label extracted from the source at build time.
    pub(crate) fn default_a11y_label(&self) -> Option<String> {
        self.expect_built("default_a11y_label")
            .default_a11y_label
            .clone()
    }

    /// Transform the still-unbuilt source view (e.g. apply a default foreground
    /// color before build). Works only on [`RetainedSubviewState::Unbuilt`] —
    /// the source is consumed at build, so running after it panics naming the
    /// misuse.
    pub(crate) fn map_source(&mut self, f: impl FnOnce(AnyView) -> AnyView) {
        let RetainedSubviewState::Unbuilt { source } = &mut self.state else {
            panic!("RetainedSubview::map_source ran after the sub-view was built");
        };
        let source = core::mem::replace(source, AnyView::new(()));
        self.state = RetainedSubviewState::Unbuilt { source: f(source) };
    }

    /// Whether the sub-view's node has been built.
    pub(crate) const fn is_built(&self) -> bool {
        matches!(self.state, RetainedSubviewState::Built(_))
    }

    /// The built child node.
    pub(crate) const fn node(&self) -> Option<&RenderNode<E>> {
        match &self.state {
            RetainedSubviewState::Built(built) => Some(&built.node),
            RetainedSubviewState::Unbuilt { .. } => None,
        }
    }

    /// Measure the sub-view's intrinsic size (building it once if needed), the
    /// node analogue of [`measure_view_intrinsic`] at the unspecified proposal.
    /// For the render path, which has the renderer to build on first use.
    pub(crate) fn measure_intrinsic(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        env: &Environment,
    ) -> Size {
        self.ensure_built(renderer, env);
        let built = self.expect_built_mut("measure_intrinsic");
        built.node.prepare_for_measure(renderer);
        let theme = renderer.theme();
        built
            .node
            .measure(&mut renderer.state, env, &theme, ProposalSize::UNSPECIFIED)
            .size
    }

    /// Measure an already-built sub-view's intrinsic size with only `&mut
    /// HydroState` — the measure-path analogue (no renderer to build on). The
    /// sub-view must already be built (via [`Self::ensure_built`]); running
    /// before that panics, a caller ordering bug rather than a zero answer.
    pub(crate) fn measure_built(
        &self,
        state: &mut HydroState<E>,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> Size {
        self.expect_built("measure_built")
            .node
            .measure(state, env, theme, ProposalSize::UNSPECIFIED)
            .size
    }

    /// Measure an already-built sub-view at a concrete proposal — the variant for
    /// content-filling sub-views (map/webview) whose composed body wraps text at the
    /// proposed width. Returns the full [`ViewDimensions`]; running before build
    /// panics, a caller ordering bug rather than a zero answer.
    pub(crate) fn measure_built_with_proposal(
        &self,
        state: &mut HydroState<E>,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
        proposal: ProposalSize,
    ) -> Size {
        self.expect_built("measure_built_with_proposal")
            .node
            .measure(state, env, theme, proposal)
            .size
    }

    /// Patch and measure a retained sub-view under a proposal, returning its
    /// stretch contract alongside the dimensions. Lazy stacks use this for
    /// visible items so a connected `Dynamic` is measured through the retained
    /// node that owns its current content, rather than by re-measuring the
    /// already-connected source view.
    pub(crate) fn patch_and_measure(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        env: &Environment,
        proposal: ProposalSize,
    ) -> (Size, StretchAxis) {
        self.ensure_built(renderer, env);
        let built = self.expect_built_mut("patch_and_measure");
        built.needs_layout |= Self::patch_built(&mut built.node, renderer);
        built.node.prepare_for_measure(renderer);
        let theme = renderer.theme();
        (
            built
                .node
                .measure(&mut renderer.state, env, &theme, proposal)
                .size,
            built.node.stretch(),
        )
    }

    /// Run the layout-time prepare pass over the sub-view's built node.
    /// Forwards to [`RenderNode::prepare_for_measure`]; running before build
    /// panics, a caller ordering bug rather than a skipped prepare.
    pub(crate) fn prepare_for_measure(&mut self, renderer: &mut HydrolysisRenderer<E>) {
        self.expect_built_mut("prepare_for_measure")
            .node
            .prepare_for_measure(renderer);
    }

    /// Stretch contract of an already-built retained sub-view.
    pub(crate) fn stretch_axis(&self) -> StretchAxis {
        self.expect_built("stretch_axis").node.stretch()
    }

    fn collect_dynamic_identities_into(&self, out: &mut FxHashSet<usize>) {
        self.expect_built("collect_dynamic_identities_into")
            .node
            .collect_dynamic_identities_into(out);
    }

    /// Emit this sub-view's accessibility nodes for the semantic walk: builds
    /// the node on first use, applies pending structural patches (a `Dynamic`
    /// inside a widget label is its own retained tree), then walks it. No
    /// layout and no encode — the sub-view emits exactly the nodes it would
    /// under a rendered flush, minus bounds.
    #[cfg(feature = "accessibility")]
    pub(crate) fn emit_accessibility(&mut self, renderer: &mut SemanticCore<E>, env: &Environment) {
        self.ensure_built(renderer, env);
        let built = self.expect_built_mut("emit_accessibility");
        // The emit walk's placement: a sub-view the walk descends into is
        // placed for the enclosing record — a semantic frame has no
        // `link_placement` to stamp it, and the record's leave retires any
        // subview it did not reach.
        built.node.core().cell.mark_placed(renderer.record_seq());
        let node = &mut built.node;
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
    fn patch_built(node: &mut RenderNode<E>, renderer: &mut SemanticCore<E>) -> bool {
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
    fn patch_for_parent(&mut self, renderer: &mut SemanticCore<E>) -> bool {
        let built = self.expect_built_mut("patch_for_parent");
        let structural = built.node.patch(renderer);
        built.needs_layout |= structural;
        structural
    }

    /// Consume the subtree's layout-invalidated mark, keeping it in
    /// `needs_layout` as well as reporting it. The flush sites fold it into
    /// `needs_layout` so a layout input change re-places the subtree at its
    /// unchanged rect. An enclosing sub-view walking through this one (a lazy
    /// row inside a navigation page) consumes the mark before this sub-view
    /// flushes, and the enclosing layout does not place it — the row is placed
    /// by its own flush — so the mark must survive here for that flush.
    /// Running before build panics, a caller ordering bug rather than a
    /// `false` answer.
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        let built = self.expect_built_mut("take_layout_dirty");
        let dirty = built.node.take_layout_dirty();
        built.needs_layout |= dirty;
        dirty
    }

    /// [`Self::take_layout_dirty`] on a sub-view that may not be built
    /// yet: unbuilt has no marks — it lays out fresh at build, so `false`
    /// is the honest answer. Widgets holding their parts as
    /// `RetainedSubview`s (labels, badges, nav bars) consume their marks
    /// this way through the leaf's `take_layout_dirty` arm
    /// (water-rs/waterui#1957).
    pub(crate) fn take_layout_dirty_if_built(&mut self) -> bool {
        match &mut self.state {
            RetainedSubviewState::Built(_) => self.take_layout_dirty(),
            RetainedSubviewState::Unbuilt { .. } => false,
        }
    }

    /// Build (once), patch, lay out (when the rect size, the structure or the
    /// §7.1 context changed), and flush the sub-view at `rect` under `env`. A
    /// zero-area rect renders nothing, matching the dispatch path's empty-rect
    /// guard. `safe_area` is the context the sub-view lays out against — the
    /// ambient context of where it is placed for an ordinary sub-view, the
    /// host's context inherited with `Covered` edges where its chrome sits —
    /// `Docked` on every edge the chrome draws a bar on (any bar with
    /// extent > 0, keyboard or not), so the edge stays untouchable while
    /// carrying the bar's inner edge as the dock a nested bar on it lands
    /// on — for chrome content (`NavigationView`, `Tabs`), `None` for a
    /// scroll surface's context-free content.
    pub(crate) fn place(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        env: &Environment,
        proposal: ProposalSize,
        rect: kurbo::Rect,
        safe_area: Option<safe_area::SafeAreaLayout>,
    ) {
        let Some((child_ctx, delta)) =
            self.layout_in_rect(renderer, ctx, env, proposal, rect, safe_area)
        else {
            return;
        };
        let built = self.expect_built_mut("place");
        // A retained subtree re-records starting at its own root — a partial
        // descent — so it first replays the enclosing `.material_group()`
        // scopes from the node's ancestry (water-rs/waterui#2268). Record the
        // sub-view's root as the owner of whatever its flush registers: a
        // press the caller registered for the whole sub-view carries the same
        // owner, and the ancestry check tells a gesture inside the sub-view
        // from one attached to the root itself.
        renderer.with_enclosing_material_group_scopes(built.node.core(), |renderer| {
            if let Some(identity) = built.node.accessibility_identity() {
                renderer.push_input_owner(&identity);
                built.node.flush(renderer, child_ctx, env, delta);
                renderer.pop_input_owner();
            } else {
                built.node.flush(renderer, child_ctx, env, delta);
            }
        });
    }

    /// [`place`](Self::place) for a subtree displayed outside
    /// the walk recording it (context menus, anchored overlays): the
    /// sub-view's registrations resolve in window space through the IDENTITY
    /// window anchor, the same answer the deleted
    /// `hit_transform = kurbo::Affine::IDENTITY` entry points gave.
    pub(crate) fn place_detached(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        env: &Environment,
        proposal: ProposalSize,
        rect: kurbo::Rect,
        safe_area: Option<safe_area::SafeAreaLayout>,
    ) {
        let Some((child_ctx, delta)) =
            self.layout_in_rect(renderer, ctx, env, proposal, rect, safe_area)
        else {
            return;
        };
        // The subtree detaches at the current registration frame: an open
        // scope (the exiting overlay's inactive gate, a suppressed preview)
        // stays in the chain and gates its entries; with none open the
        // current placement is the presentation host's own.
        let anchor = renderer.current_placement();
        let index = anchor.take_item();
        let built = self.expect_built_mut("place_detached");
        // Same partial-descent replay as `place` — a detached subtree's
        // enclosing `.material_group()` scopes come from its ancestry, not
        // the stack (#2268).
        renderer.with_enclosing_material_group_scopes(built.node.core(), |renderer| {
            built
                .node
                .flush_anchored(renderer, child_ctx, env, Some(anchor), delta, index);
        });
    }

    /// The shared head of [`place`](Self::place) and
    /// [`place_detached`](Self::place_detached): build on
    /// first use, apply pending patches, lay the sub-view out at `rect`'s
    /// size when anything changed, and return the child context and the
    /// delta that places it at `rect`. `None` for a zero-area rect, which
    /// renders nothing (the dispatch path's empty-rect guard).
    fn layout_in_rect(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        env: &Environment,
        proposal: ProposalSize,
        rect: kurbo::Rect,
        safe_area: Option<safe_area::SafeAreaLayout>,
    ) -> Option<(RenderContext, kurbo::Affine)> {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return None;
        }
        self.ensure_built(renderer, env);
        let built = self.expect_built_mut("layout_in_rect");
        let structural = Self::patch_built(&mut built.node, renderer);
        built.node.prepare_for_measure(renderer);
        let size = Size::new(
            crate::num_cast::f64_as_f32(rect.width()),
            crate::num_cast::f64_as_f32(rect.height()),
        );
        built.needs_layout |= structural | built.node.take_layout_dirty();
        built.layout_if_needed(renderer, env, safe_area, proposal, size);
        let delta = kurbo::Affine::translate((rect.x0, rect.y0));
        let child_ctx = ctx.child(
            delta,
            kurbo::Rect::new(0.0, 0.0, rect.width(), rect.height()),
        );
        Some((child_ctx, delta))
    }

    /// The retained identity of the built sub-view's root node — the owner the
    /// input path records for a press registered on the sub-view's behalf, so a
    /// gesture registered inside the sub-view is a strict descendant of it and
    /// one attached to the root itself is not. `None` when the root carries no
    /// identity; running before build panics, a caller ordering bug.
    pub(crate) fn root_accessibility_identity(&self) -> Option<Rc<()>> {
        self.expect_built("root_accessibility_identity")
            .node
            .accessibility_identity()
    }

    /// Build (once), lay out at `size` (only when it changes), and flush the
    /// sub-view under a caller-supplied [`RenderContext`] — the variant for a
    /// sub-view drawn under a non-translation transform (the text-field floating
    /// label's animated translate + scale). The caller composes the transform via
    /// [`RenderContext::child`] and passes the local layout `size` the node should
    /// lay out at; a zero-area size renders nothing.
    #[expect(
        clippy::too_many_arguments,
        reason = "threads the full flush context; grouping into a struct would not improve clarity"
    )]
    pub(crate) fn place_in_ctx(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        env: &Environment,
        proposal: ProposalSize,
        size: Size,
        safe_area: Option<safe_area::SafeAreaLayout>,
        placement_delta: kurbo::Affine,
    ) {
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        self.ensure_built(renderer, env);
        let built = self.expect_built_mut("place_in_ctx");
        let structural = Self::patch_built(&mut built.node, renderer);
        built.node.prepare_for_measure(renderer);
        built.needs_layout |= structural | built.node.take_layout_dirty();
        built.layout_if_needed(renderer, env, safe_area, proposal, size);
        renderer.with_enclosing_material_group_scopes(built.node.core(), |renderer| {
            if let Some(identity) = built.node.accessibility_identity() {
                renderer.push_input_owner(&identity);
                built.node.flush(renderer, ctx, env, placement_delta);
                renderer.pop_input_owner();
            } else {
                built.node.flush(renderer, ctx, env, placement_delta);
            }
        });
    }

    /// Builds, lays out and records a navigation page into the open page
    /// scope, anchored at the scope's placement.
    pub(crate) fn record_built_page(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        env: &Environment,
        size: Size,
        safe_area: Option<safe_area::SafeAreaLayout>,
    ) {
        self.ensure_built(renderer, env);
        let built = self.expect_built_mut("record_built_page");
        let structural = Self::patch_built(&mut built.node, renderer);
        built.node.prepare_for_measure(renderer);
        built.needs_layout |= structural | built.node.take_layout_dirty();
        let proposal = ProposalSize::new(Some(size.width), Some(size.height));
        built.layout_if_needed(renderer, env, safe_area, proposal, size);
        let anchor = renderer.current_placement();
        let local_ctx = RenderContext::new(
            renderer.program().origin(),
            kurbo::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
        );
        renderer.push_lazy_viewport(LazyViewport {
            bounds: local_ctx.bounds(),
            transform: anchor.resolved_transform(true),
        });
        let index = anchor.take_item();
        // A page records outside the window walk — the partial descent
        // replays the enclosing `.material_group()` scopes from the page
        // root's ancestry so its members join the same scope (#2268).
        renderer.with_enclosing_material_group_scopes(built.node.core(), |renderer| {
            built.node.flush_anchored(
                renderer,
                local_ctx,
                env,
                Some(anchor),
                kurbo::Affine::IDENTITY,
                index,
            );
        });
        renderer.pop_lazy_viewport("retained page record");
    }
}

/// A cache of retained node sub-views for a *virtualized* collection (a lazy
/// stack, list, or table): only items in the current visible window are built and
/// retained, keyed by a stable identity, so a long collection costs only its
/// visible rows. Items are built lazily as they scroll into view and evicted once
/// they leave the visible set — matching virtualization, where scrolled-away item
/// state is intentionally not preserved. While an item stays visible its node is reused,
/// so its reactive content stays live through the node's own per-frame re-flush.
pub struct VisibleSubviewCache<K: Eq + core::hash::Hash + Clone, E: crate::text::TextEngine> {
    entries: std::collections::HashMap<K, RetainedSubview<E>>,
    /// Keys touched during the in-progress frame; [`Self::end_frame`] evicts the rest.
    touched: std::collections::HashSet<K>,
}

impl<K: Eq + core::hash::Hash + Clone, E: crate::text::TextEngine> VisibleSubviewCache<K, E> {
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
    ) -> &mut RetainedSubview<E> {
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
    ) -> &mut RetainedSubview<E> {
        self.entries
            .entry(key)
            .or_insert_with(|| RetainedSubview::new(build()))
    }

    /// Look up an already-retained item without marking it visible this frame.
    pub(crate) fn get(&self, key: &K) -> Option<&RetainedSubview<E>> {
        self.entries.get(key)
    }

    /// Removes the retained sub-view of `key` from the cache, handing it to
    /// the caller — a removed row's realization moving into its ghost.
    pub(crate) fn take(&mut self, key: &K) -> Option<RetainedSubview<E>> {
        self.touched.remove(key);
        self.entries.remove(key)
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
    pub(crate) fn prepare_for_measure(&mut self, renderer: &mut HydrolysisRenderer<E>) {
        for entry in self.entries.values_mut() {
            entry.prepare_for_measure(renderer);
        }
    }

    /// Patches every currently retained (therefore visible) item before its
    /// virtualized parent is measured.
    pub(crate) fn patch_for_parent(&mut self, renderer: &mut SemanticCore<E>) -> bool {
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
pub struct WrapperNode<E: crate::text::TextEngine> {
    pub(super) accessibility_identity: Rc<()>,
    /// The node's own mount core: its cell keys the engine mounts a
    /// `Material` wrapper (or any keyed effect) presents under, and a
    /// `MaterialGroup` wrapper keys its backdrop-group scope by it.
    pub(crate) core: NodeCore<E>,
    pub(super) effect: WrapperEffect<E>,
    pub(super) env: Environment,
    /// The per-edge release [`safe_area::SafeAreaLayout::release`] computed for an
    /// `IgnoreSafeArea` wrapper — the flush mirrors it into the child's
    /// context. Zero for every other effect.
    pub(super) released_offsets: Cell<safe_area::EdgeOffsets>,
    pub(super) child: RenderNode<E>,
}

/// The type-erased behavior of one retained native widget state allocation.
pub trait WidgetBehavior<E: crate::text::TextEngine> {
    /// The default layout priority of this native leaf.
    fn priority(&self) -> i32 {
        0
    }

    /// §7.1's scroll-surface facts, handed from layout each pass — `None`
    /// where the leaf has no safe-area context (inside a scroll surface's
    /// content, or in the semantic pipeline). The surfaces that own a
    /// [`crate::scroll::ScrollHandle`] (list, table) store it; the default
    /// is a no-op for every other widget.
    fn update_scroll_surface(&self, _facts: Option<safe_area::ScrollSurfaceFacts>) {}

    /// Whether this leaf draws nothing — `WaterUI`'s empty view `()`.
    ///
    /// This is a semantic answer, not a measured size: a `Spacer` squeezed to
    /// zero still renders and still answers `false`. A stack treats a child
    /// answering `true` as a non-member (§4.4: no slot, no spacing).
    fn renders_nothing(&self) -> bool {
        false
    }

    /// Re-renders the leaf from its retained state. `safe_area` is the §7.1
    /// context this widget was laid out against — `None` inside a scroll
    /// surface's context-free content — which retained sub-views it hosts
    /// (a chrome container's content, a label, an overlay) derive their own
    /// context from.
    fn render(
        self: Rc<Self>,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        env: &Environment,
        safe_area: Option<safe_area::SafeAreaLayout>,
    );

    /// Measures the leaf from its retained state.
    fn measure(
        &self,
        state: &mut HydroState<E>,
        proposal: ProposalSize,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> ViewDimensions;

    /// Consume the layout-dirty marks of the [`RetainedSubview`]s this
    /// leaf owns — labels, bar parts, badge content — and report whether
    /// any fired since their last consume. The sub-views' trees hang off
    /// this leaf's measure, so their marks decide the leaf's measured
    /// size: an enclosing `RetainedSubview` reads them through the
    /// `Widget` arm of [`RenderNode::take_layout_dirty`], and each
    /// consumed sub-view keeps the mark in its own `needs_layout` for
    /// its next flush (water-rs/waterui#1957). `false` for leaves that
    /// own no sub-views.
    fn take_layout_dirty(&self) -> bool {
        false
    }

    /// Runs the leaf's layout-time prepare step: applying theme paint that
    /// could not be resolved at tree-build time (build contexts carry no
    /// theme) and building the retained sub-views the measure path then reads.
    /// [`RenderNode::prepare_for_measure`] runs this over a subtree once per
    /// layout or measure entry, while a `&mut HydrolysisRenderer` — hence the
    /// theme — is in hand, before any node below it is measured. A semantic
    /// runtime never runs this pass, so no paint reaches it.
    fn prepare(&self, _renderer: &mut HydrolysisRenderer<E>, _env: &Environment) {}

    /// Claims the transition declared on the root of this leaf's page
    /// content, for the navigation stack that owns the leaf as a page
    /// (layout-spec.md §4.5). `None` for every leaf that is not a page.
    fn claim_page_transition(&self) -> Option<waterui_core::transition::TransitionSpec> {
        None
    }

    /// Emits this leaf's accessibility node(s) for the semantic walk — the
    /// same tree `render` produces under a `RenderContext`, with no bounds and
    /// no draw or hit-target work. The default emits nothing: leaves with no
    /// semantics (spacer, divider, gradient, …) need no override.
    #[cfg(feature = "accessibility")]
    fn emit_accessibility(self: Rc<Self>, _renderer: &mut SemanticCore<E>, _env: &Environment) {}
}

/// A native widget leaf rendered every flush from one retained state allocation.
/// The behavior reuses the widget's existing render and measure functions, which
/// re-read live signals and re-emit interaction targets and accessibility at the
/// current bounds. No bake, no capture-once freeze.
pub struct WidgetNode<E: crate::text::TextEngine> {
    pub(super) accessibility_identity: Rc<()>,
    pub(crate) core: NodeCore<E>,
    /// The §7.1 context the widget was last laid out against — node-lifetime
    /// storage: a widget inside an unchanged retained sub-view is not
    /// re-laid out on steady frames, but its flush still reads the context
    /// here for `safe_area_for`/`content_area_for` and its scroll surface.
    pub(super) safe_area: Option<Box<safe_area::SafeAreaLayout>>,
    pub(super) behavior: Rc<dyn WidgetBehavior<E>>,
    pub(super) stretch: StretchAxis,
    /// Whether this leaf paints a fill — the gradient — marking it
    /// eligible for §7.1's background-slot paint extension. Set at build
    /// by [`RenderNode::build_gradient`]; a `Color` leaf is a fill by its
    /// own variant instead.
    pub(super) fill_leaf: bool,
    pub(super) env: Environment,
}

/// The background-slot fill (§7.1): wraps the node `build_fixed_container`
/// identified as the slot's paint fill — a `Color` or the gradient leaf,
/// possibly inside layout-transparent wrappers — and records the paint
/// extension its laid-out frame earns. Extension is a paint fact, not a
/// layout fact: layout computes the per-edge distances once and the flush
/// grows the paint rect by exactly that, under unchanged transforms.
pub struct FillNode<E: crate::text::TextEngine> {
    pub(super) child: RenderNode<E>,
    /// The fill's own mount core: cell, placement and subscriptions.
    pub(crate) core: NodeCore<E>,
    /// The paint extension [`safe_area::SafeAreaLayout::touched_edge_offsets`]
    /// computed for the wrapped frame — `None` until the first layout.
    pub(super) extension: Cell<Option<safe_area::EdgeOffsets>>,
}

impl<E: crate::text::TextEngine> FillNode<E> {
    pub(crate) const fn new(child: RenderNode<E>, core: NodeCore<E>) -> Self {
        Self {
            child,
            core,
            extension: Cell::new(None),
        }
    }
}

/// The per-flush effect a [`WrapperNode`] re-applies around its child. Each
/// variant defers to the matching `apply_*` helper in `metadata.rs`, so the
/// effect logic is shared byte-for-byte with the dispatch path.
pub(super) enum WrapperEffect<E: crate::text::TextEngine> {
    /// Purely a layout hint: it changes which child a stack compresses first and
    /// draws nothing, so the flush path renders straight through it.
    LayoutPriority(LayoutPriority),
    /// `.ignore_safe_area(regions.on(edges))` — the window's safe-area
    /// insets are released through the regions and edges the declaration
    /// names: layout offers the child the window bounds expanded by the
    /// released depths, and flush shifts its frame back the same amount, so
    /// the subtree reaches through the ignored zones and stops at the
    /// nearest unignored one (layout-spec.md §7.1).
    IgnoreSafeArea(IgnoreSafeArea),
    NavigationTransitionSource(RawId),
    NavigationTransitionDestination(RawId),
    Clip(ClipShape),
    Border(Border),
    Shadow(Shadow),
    Cursor(Cursor),
    /// A drag source. The `Rc` lets the registered hit-test action read the
    /// payload live — `Draggable::payload` snapshots its signal at the moment
    /// the drag begins.
    Draggable(Rc<Draggable>),
    DropDestination(DropDestinationHandles),
    ContextMenu(ContextMenuEffect<E>),
    /// Conditional hit-testing: renders the child, then truncates the interaction
    /// targets it registered when disabled. The bookkeeping counts targets across
    /// the (node-flushed) child render, so reactive descendants stay live.
    Hittable(Hittable),
    /// A hover-enter/move/exit handler re-registered every flush. The handler is
    /// shared so the node can re-register the same `OnEvent` each frame.
    OnEvent(Rc<RefCell<OnEvent>>),
    /// An `.on_key_press` ancestor scope: pushed onto the renderer's key
    /// handler stack while the child flushes, so a focused input inside the
    /// subtree bubbles its unconsumed keys here.
    OnKeyPress(Rc<RefCell<OnKeyPress>>),
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
    /// A within-window `Material` background (water-rs/waterui#1854): every
    /// flush closes the scene segment painted so far — the content behind
    /// the view — and presents a keyed mount that samples a backdrop group
    /// inside the view's bounds, then flushes the child on top. The level is
    /// part of the member's backdrop-group key.
    Material(crate::renderer::material::WithinWindowLevel),
    /// A `.material_group()` scope (water-rs/waterui#1999): the wrapper
    /// carries no parameters — its `render_id` is the group-scope identity
    /// the flush pushes while the child flushes, so the members of one
    /// modifier instance join one shared backdrop group and two instances
    /// are two groups.
    MaterialGroup,
    /// An `.anchored_overlay(...)` (water-rs/waterui#1275): every flush the
    /// wrapper registers the anchor's live bounds plus the effect's handles
    /// for the post-flush `render_anchored_overlays` pass, which measures,
    /// places and draws the open overlay at window level above all content.
    AnchoredOverlay(AnchoredOverlayEffect<E>),
    /// `.dialog(...)`: reports the dialog's presentation to the window's
    /// dialog stack each flush; the post-flush `render_dialogs` pass draws
    /// the one on screen in the modal layer above all content — its scrim
    /// plus the composed card.
    Dialog(DialogEffect<E>),
    /// A `.transition(...)` declaration (layout-spec.md §4.5): paint- and
    /// layout-transparent. The structural site that swaps the content claims
    /// it from the built instance; one nobody owns fails when reached.
    Transition(super::transition::TransitionDeclaration),
}

impl<E: crate::text::TextEngine> WrapperEffect<E> {
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
                | Self::Dialog(_)
                | Self::LifeCycle(_)
        )
    }
}

/// The node-owned state of an `.anchored_overlay(...)` wrapper. The content
/// slot is `Rc`-shared because the post-flush render pass — not this node —
/// measures and flushes it, and the node keeps it built across closes.
pub struct AnchoredOverlayEffect<E: crate::text::TextEngine> {
    /// Builds a fresh view for each presentation: a presentation started
    /// while an older exit is still animating gets its own instance.
    pub(crate) builder: waterui_core::handler::AnyViewBuilder,
    /// The overlay's presentations: the one presented now, built from
    /// `builder` on each new presentation, and the dismissed ones still
    /// leaving the screen.
    pub(crate) presentations: Rc<RefCell<crate::renderer::AnchoredPresentations<E>>>,
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

/// The node-owned state of a `.dialog(...)` wrapper. The content slot is
/// `Rc`-shared because the post-flush render pass — not this node — measures
/// and flushes the modal layer, and the node keeps it built across closes.
pub struct DialogEffect<E: crate::text::TextEngine> {
    /// The layer content — the dialog's scrim-plus-card modal layer.
    pub(crate) content: Rc<RefCell<RetainedSubview<E>>>,
    /// The dialog being presented: its cancel path is what the modal
    /// environment routes Escape and the Android system back to. Its
    /// presentation binding is read every flush (subscribing the frame to
    /// it) and written `false` by the dialog's own dismiss paths and by the
    /// node leaving the tree.
    pub(crate) dialog: waterui::dialog::Dialog,
    /// The node's identity in the window's dialog stack — a node that stops
    /// reporting left the tree.
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
pub struct ContextMenuEffect<E: crate::text::TextEngine> {
    /// The resolved menu items the popup is built from.
    pub(crate) items: nami::Computed<Vec<ResolvedMenuItem>>,
    /// Counter of the accessory's dismiss requests; every change closes the
    /// open menu (water-rs/waterui#1245).
    pub(crate) dismiss_requests: nami::Computed<i32>,
    /// The view lifted over the dimmed backdrop at the source's frame while
    /// the menu is open. `None` means lift the source view itself, realized
    /// as a hole punched in the dim backdrop — the source is still drawn by
    /// the owning tree, so it cannot be re-flushed here.
    pub(crate) preview: Rc<RefCell<Option<RetainedSubview<E>>>>,
    /// The interactive view anchored to the lifted preview, mounted outside
    /// the menu so its own pointer, touch and keyboard input reaches it.
    pub(crate) accessory: Rc<RefCell<Option<RetainedSubview<E>>>>,
}

/// The node-owned state of a lifecycle hook (see [`WrapperEffect::LifeCycle`]).
/// An appear hook is consumed after the child's first flush; a disappear hook is
/// fired exactly once when the node is dropped, so structural presence/removal —
/// not a frame-diff slot cursor — drives lifecycle events.
pub struct LifeCycleEffect {
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
pub struct GestureObserverEffect {
    pub gesture: Gesture,
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

pub struct ColorNode<E: crate::text::TextEngine> {
    pub(crate) bounds: nami::Binding<kurbo::Rect>,
    pub(crate) core: NodeCore<E>,
    pub(crate) color: Computed<waterui_graphics::draw::WorkingColor>,
}

pub struct TextNode<E: crate::text::TextEngine> {
    /// Per-frame measure memo gate: records whether this node's body was
    /// re-probed within a frame, gating `memo_slots` so a node measured
    /// once per frame pays a `Cell` update instead of a `RefCell` borrow.
    pub(crate) memo_gate: Cell<MemoGate>,
    /// The proposal ring `measure` consults once `memo_gate` marks this
    /// node as re-probed. Owned by the node, so a dropped node never
    /// leaves a stale answer behind.
    pub(crate) memo_slots: RefCell<NodeMeasureEntry>,
    pub(crate) accessibility_identity: Rc<()>,
    pub(crate) core: NodeCore<E>,
    pub(crate) content: Computed<StyledStr>,
    pub(crate) alignment: Computed<HorizontalAlignment>,
    /// Maximum laid-out lines, from `TextConfig::line_limit`.
    pub(crate) line_limit: Option<usize>,
    /// Set by the `content`/`alignment` subscriptions when a measurement input
    /// changes. The text's size is a function of what it says, so an outer
    /// `RetainedSubview` consumes this through [`RenderNode::take_layout_dirty`]
    /// and re-places its tree: otherwise the leaf keeps the box it measured at
    /// mount while the flush paints the new string wrapped inside it.
    pub(crate) layout_dirty: Rc<Cell<bool>>,
    /// The `content` and `alignment` subscriptions that arm `layout_dirty`,
    /// owned by this retained leaf.
    pub(crate) _guards: [BoxWatcherGuard; 2],
    /// The tap targets the last flush registered for the content's
    /// [`TapSpan`]s — one entry per laid-out span rectangle, reused in order
    /// each flush like [`GestureObserverEffect::gesture_target`] so a tap
    /// that began before the repaint keeps its recognizer.
    pub(crate) tap_span_targets: RefCell<Vec<TapSpanTarget>>,
}

/// One tap-span rectangle's retained registration: the [`GestureTarget`]
/// re-registered every flush and the slot its recognizer's action reads.
/// The slot is refreshed per flush, so a target index that now covers a
/// different span never fires the stale span's action.
pub struct TapSpanTarget {
    pub(crate) target: crate::gesture::GestureTarget,
    pub(crate) slot: Rc<RefCell<TapSpanSlot>>,
}

/// What a span tap target fires: the span and the environment captured at
/// registration, both refreshed on every flush.
pub struct TapSpanSlot {
    pub(crate) span: TapSpan,
    pub(crate) env: Environment,
}

pub struct ContainerNode<E: crate::text::TextEngine> {
    /// Per-frame measure memo gate: records whether this node's body was
    /// re-probed within a frame, gating `memo_slots` so a node measured
    /// once per frame pays a `Cell` update instead of a `RefCell` borrow.
    pub(crate) memo_gate: Cell<MemoGate>,
    /// The proposal ring `measure` consults once `memo_gate` marks this
    /// node as re-probed. Owned by the node, so a dropped node never
    /// leaves a stale answer behind.
    pub(crate) memo_slots: RefCell<NodeMeasureEntry>,
    pub(crate) accessibility_identity: Rc<()>,
    pub(crate) core: NodeCore<E>,
    pub(crate) layout: Box<dyn Layout>,
    pub(crate) children: Vec<RenderNode<E>>,
    #[cfg(feature = "accessibility")]
    pub(crate) accessibility_child_env: Option<Environment>,
    /// Child frames cached by [`RenderNode::layout`]; reused by
    /// [`RenderNode::flush`] so a geometry-static frame pays only re-encode.
    pub(crate) placed: Vec<Rect>,
    /// The extent this container resolved at layout — the size it answered to
    /// the selected proposal, centred on the assigned frame — kept beside
    /// `placed` so a naming-scope owner reports its own extent when the
    /// parent assigned more than it answered (water-rs/hydrolysis#51).
    #[cfg(feature = "accessibility")]
    pub(crate) resolved: Rect,
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
pub struct OpacityNode<E: crate::text::TextEngine> {
    pub(crate) core: NodeCore<E>,
    pub(crate) value: Opacity,
    pub(crate) child: RenderNode<E>,
}

pub struct ScaleNode<E: crate::text::TextEngine> {
    pub(crate) core: NodeCore<E>,
    pub(crate) value: Scale,
    pub(crate) child: RenderNode<E>,
}

pub struct RotationNode<E: crate::text::TextEngine> {
    pub(crate) core: NodeCore<E>,
    pub(crate) value: Rotation,
    pub(crate) child: RenderNode<E>,
}

pub struct OffsetNode<E: crate::text::TextEngine> {
    pub(crate) core: NodeCore<E>,
    pub(crate) value: Offset,
    pub(crate) child: RenderNode<E>,
}

pub struct ScrollNode<E: crate::text::TextEngine> {
    /// Per-frame measure memo gate: records whether this node's body was
    /// re-probed within a frame, gating `memo_slots` so a node measured
    /// once per frame pays a `Cell` update instead of a `RefCell` borrow.
    pub(crate) memo_gate: Cell<MemoGate>,
    /// The proposal ring `measure` consults once `memo_gate` marks this
    /// node as re-probed. Owned by the node, so a dropped node never
    /// leaves a stale answer behind.
    pub(crate) memo_slots: RefCell<NodeMeasureEntry>,
    pub(super) accessibility_identity: Rc<()>,
    pub(crate) core: NodeCore<E>,
    pub(super) axis: ScrollAxis,
    pub(super) child: RenderNode<E>,
    pub(super) controller: Option<ScrollController<Point>>,
    /// Binding `ScrollView::report_offset` connected to this scroll view —
    /// the scroll handle writes the content offset, in points, into it
    /// whenever the offset changes.
    pub(super) offset: Option<Binding<Point>>,
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
    /// Environment captured at build — for scroll-target accessibility and
    /// the child layout/flush environment.
    pub(super) env: Environment,
    /// §7.1's scroll-surface bookkeeping: the extension and clearance
    /// bounds layout computed once (the handle is rebound exactly once per
    /// layout with them), plus the focused-field state the flush drives.
    pub(super) surface: std::rc::Rc<safe_area::ScrollSurfaceArea>,
}

pub struct RetainNode<E: crate::text::TextEngine> {
    pub(crate) core: NodeCore<E>,
    pub(super) _retain: Retain,
    pub(super) child: RenderNode<E>,
}

pub struct EnvNode<E: crate::text::TextEngine> {
    pub(crate) core: NodeCore<E>,
    /// The scoped environment this subtree was built under, used to override the
    /// inherited environment at every measure/layout/flush.
    pub(super) env: Environment,
    pub(super) child: RenderNode<E>,
}

pub struct SceneViewNode<E: crate::text::TextEngine> {
    pub(super) accessibility_identity: Rc<()>,
    pub(crate) core: NodeCore<E>,
    /// The owned scene content, re-drawn each flush (it reads its own reactive
    /// inputs in `build_scene`). `RefCell` because `build_scene` needs `&mut` but
    /// `flush` takes `&self`.
    pub(super) content: Rc<RefCell<Box<dyn waterui_graphics::SceneContent>>>,
    /// The semantic invalidator installed at build — the compositor re-installs
    /// it after a `rebuild_for_engine` clears the content's engine-bound hooks,
    /// so signal-driven frame requests keep working across a device swap.
    pub(super) invalidator: waterui_graphics::SceneInvalidator,
    /// The engine resource-table identity `content` last recorded against —
    /// `None` until its first mount, a `Weak` that expires with the pooled
    /// engine's table. Shared into the node's recorded run so every commit
    /// sees the same recorded identity.
    pub(super) association:
        Rc<RefCell<Option<std::rc::Weak<crate::renderer::recording::SceneResources>>>>,
    /// The `(proposal, answer)` this leaf last measured at — the mark
    /// [`RenderNode::take_layout_dirty`] reads through
    /// [`Self::size_drifted`]: a scene whose natural size changes after
    /// mount (a reloaded image's viewBox) must not keep its first box
    /// inside a retained sub-view (water-rs/waterui#1957).
    pub(super) measured: Cell<Option<(ProposalSize, Size)>>,
}

impl<E: crate::text::TextEngine> SceneViewNode<E> {
    /// The leaf's natural-size answer under `proposal` — the
    /// `measure_body` arm's own computation.
    pub(super) fn intrinsic_dimensions(&self, proposal: ProposalSize) -> ViewDimensions {
        let resolved = waterui_graphics::resolve_scene_proposal(
            self.content.borrow().intrinsic_size(),
            proposal,
        );
        ViewDimensions::new(Size::new(
            resolved.width.unwrap_or(0.0),
            resolved.height.unwrap_or(0.0),
        ))
    }

    /// Whether a fresh probe at the last proposal answers a different
    /// size than `measured` latched — the leaf's implicit layout mark:
    /// there is no consumable flag; the next `measure` rewrites
    /// `measured` and ends the drift report by itself.
    pub(super) fn size_drifted(&self) -> bool {
        self.measured
            .get()
            .is_some_and(|(proposal, size)| self.intrinsic_dimensions(proposal).size != size)
    }
}

/// A `GpuContentView` leaf that OWNS its [`GpuContentRuntime`] — the node
/// analogue of [`SceneViewNode`], for a `Native<GpuContentView>` reached
/// through the retained tree. Identity is structural: a reactive swap builds
/// a fresh node with a fresh runtime (and a fresh one-shot engine-content
/// install), while a per-frame re-flush re-binds the *same* runtime via an
/// `Rc`-carrying compositor layer, so there is no cursor-ordered slot to
/// desync.
#[cfg(hydrolysis_gpu)]
pub struct GpuContentNode<E: crate::text::TextEngine> {
    pub(super) accessibility_identity: Rc<()>,
    pub(crate) core: NodeCore<E>,
    pub(super) runtime: Rc<RefCell<crate::gpu_view::GpuContentRuntime>>,
    /// The `(proposal, answer)` this leaf last measured at — the mark
    /// [`RenderNode::take_layout_dirty`] reads through
    /// [`Self::size_drifted`], as on [`SceneViewNode`]
    /// (water-rs/waterui#1957).
    pub(super) measured: Cell<Option<(ProposalSize, Size)>>,
}

#[cfg(hydrolysis_gpu)]
impl<E: crate::text::TextEngine> GpuContentNode<E> {
    /// The leaf's live measure under `proposal` — the `measure_body`
    /// arm's own call. [`GpuContentView::measure`] asks the content
    /// itself, so a source whose size resolves after mount answers its
    /// new natural size instead of the `intrinsic_size` snapshot
    /// [`GpuContentView::new`] took (water-rs/waterui#1958).
    pub(super) fn intrinsic_dimensions(&self, proposal: ProposalSize) -> ViewDimensions {
        self.runtime.borrow().view.measure(proposal)
    }

    /// Whether a fresh probe at the last proposal answers a different
    /// size than `measured` latched — the leaf's implicit layout mark.
    pub(super) fn size_drifted(&self) -> bool {
        self.measured
            .get()
            .is_some_and(|(proposal, size)| self.intrinsic_dimensions(proposal).size != size)
    }
}

/// An `ExternalFrameView` leaf that OWNS its [`crate::gpu_view::ExternalFrameRuntime`]
/// — the node analogue of [`GpuContentNode`], for a `Native<ExternalFrameView>`
/// reached through the retained tree. The compositor mounts it as its own
/// engine layer and starts the stream's source on first install; each frame
/// the layer drains the receiver's mailbox and presents the newest frame.
#[cfg(hydrolysis_gpu)]
pub struct ExternalFrameNode<E: crate::text::TextEngine> {
    pub(super) accessibility_identity: Rc<()>,
    pub(crate) core: NodeCore<E>,
    pub(super) runtime: Rc<RefCell<crate::gpu_view::ExternalFrameRuntime>>,
    /// The `(proposal, answer)` this leaf last measured at — the mark
    /// [`RenderNode::take_layout_dirty`] reads through
    /// [`Self::size_drifted`]: a stream that learns its size only after
    /// decoding starts must not keep its first box inside a retained
    /// sub-view (water-rs/waterui#1957).
    pub(super) measured: Cell<Option<(ProposalSize, Size)>>,
}

#[cfg(hydrolysis_gpu)]
impl<E: crate::text::TextEngine> ExternalFrameNode<E> {
    /// The source's measure under `proposal` — the `measure_body` arm's
    /// own call, repeated to detect drift.
    pub(super) fn intrinsic_dimensions(&self, proposal: ProposalSize) -> ViewDimensions {
        self.runtime.borrow().view.measure(proposal)
    }

    /// Whether a fresh probe at the last proposal answers a different
    /// size than `measured` latched — the leaf's implicit layout mark.
    pub(super) fn size_drifted(&self) -> bool {
        self.measured
            .get()
            .is_some_and(|(proposal, size)| self.intrinsic_dimensions(proposal).size != size)
    }
}

/// A `FilteredView` wrapper that OWNS its [`FilteredRuntime`]
/// (the unregistered effect source plus its `ParamGuards`, then the engine
/// `Filter` handle after first registration) and builds its wrapped child as
/// a persistent [`RenderNode`]. Layout-transparent: it measures, lays out,
/// and patches the child exactly as the child would on its own. Each flush
/// presents the child's layers inside a keyed filtered mount — the engine
/// applies the filter across the whole group.
pub struct FilteredNode<E: crate::text::TextEngine> {
    pub(crate) core: NodeCore<E>,
    pub(super) runtime: Rc<RefCell<crate::renderer::effects::FilteredRuntime>>,
    pub(super) child: RenderNode<E>,
    pub(super) env: Environment,
}

pub struct DynamicHostNode<E: crate::text::TextEngine> {
    pub(crate) core: NodeCore<E>,
    /// The §7.1 context the host was last laid out against — the flush's
    /// mid-pass layout for a child that applied its pending reuses it.
    pub(super) safe_area: Option<Box<safe_area::SafeAreaLayout>>,
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
    pub(super) child: Rc<RefCell<RenderNode<E>>>,
    /// A mid-pass swap changed the child after the parent's layout already
    /// placed this host. Read through [`RenderNode::take_layout_dirty`], it
    /// propagates the invalidation to the enclosing retained sub-view, which
    /// re-lays out its tree before its next flush — the caller-imposed rect a
    /// `RetainedSubview` flushes at never renegotiates itself.
    pub(super) layout_dirty: Cell<bool>,
    /// The structural transitions of the content instances this host swaps
    /// (layout-spec.md §4.5).
    pub(super) transitions: DynamicTransitions<E>,
}

/// A `Dynamic` host's transition state: which declaration applies to the
/// current instance, its insertion clock, the frame its removal would paint
/// at, and the ghosts of instances already swapped out.
pub struct DynamicTransitions<E: crate::text::TextEngine> {
    /// A `.transition(...)` declared over the conditional itself, adopted
    /// when that declaration's node is built: it applies to every content
    /// instance that declares none of its own.
    declared: RefCell<Option<TransitionSpec>>,
    /// The declaration the current content instance carries on its own root
    /// chain, claimed when the instance is built.
    claimed: RefCell<Option<TransitionSpec>>,
    /// The current instance's insertion while it animates in.
    insertion: RefCell<Option<super::transition::TransitionRun<E>>>,
    /// Removed instances still animating out, oldest first. The host owns
    /// them as its own children whatever layout encloses it: a ghost takes
    /// no part in layout and is out of semantics, accessibility, focus and
    /// hit testing; it only paints, over the slot the live content sizes.
    ghosts: RefCell<Vec<DynamicGhost<E>>>,
    /// The context the live instance was last flushed at — its frame in the
    /// drawing space of the nearest enclosing layer, where its ghost paints
    /// once it leaves.
    last_context: Cell<RenderContext>,
    /// Reduce Motion resolved once at build from the host environment.
    reduce_motion: bool,
    /// Logical-edge resolution for sampled transition properties.
    direction: waterui_core::layout::LayoutDirection,
}

/// One removed content instance of a `Dynamic` host, painted until its own
/// removal clock completes.
pub struct DynamicGhost<E: crate::text::TextEngine> {
    /// The outgoing instance, detached from semantics.
    node: RenderNode<E>,
    /// Its removal clock and paint runtime.
    removal: super::transition::TransitionRun<E>,
    /// The context the instance was last flushed at: its frame in the
    /// drawing space of the host's nearest enclosing layer.
    context: RenderContext,
}

impl<E: crate::text::TextEngine> DynamicTransitions<E> {
    pub(super) fn new(env: &Environment) -> Self {
        Self {
            declared: RefCell::new(None),
            claimed: RefCell::new(None),
            insertion: RefCell::new(None),
            ghosts: RefCell::new(Vec::new()),
            last_context: Cell::new(RenderContext::new(
                kurbo::Affine::IDENTITY,
                kurbo::Rect::ZERO,
            )),
            reduce_motion: env
                .get::<waterui_core::transition::ReduceMotion>()
                .is_some_and(|flag| flag.0),
            direction: waterui_layout::layout_direction(env).snapshot(),
        }
    }

    /// The declaration governing the current instance: its own, else the one
    /// declared over the conditional.
    fn effective(&self) -> Option<TransitionSpec> {
        self.claimed
            .borrow()
            .clone()
            .or_else(|| self.declared.borrow().clone())
    }
}

impl<E: crate::text::TextEngine> DynamicHostNode<E> {
    /// Adopts a `.transition(...)` declared over this conditional.
    pub(super) fn adopt(&self, spec: TransitionSpec) {
        *self.transitions.declared.borrow_mut() = Some(spec);
    }

    /// Records the current instance's declaration, claimed from its built
    /// root.
    pub(super) fn claim_initial(&self) {
        *self.transitions.claimed.borrow_mut() =
            super::transition::claim_transition(&self.child.borrow());
    }

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
    ///
    /// The swap is a structural transition event (§4.5): an outgoing instance
    /// under a removal declaration leaves the semantic tree here and keeps
    /// painting as a ghost in its frozen slot; an incoming instance under a
    /// declaration starts its own insertion clock. Re-inserting during an
    /// exit is always a new instance beside the old ghost.
    pub(super) fn apply_pending(&self, renderer: &mut SemanticCore<E>) -> bool {
        let pending = self.pending.borrow_mut().take();
        match pending {
            Some(content) => {
                let node_env = self.env.clone();
                let child = RenderNode::build(content, &node_env, renderer);
                // The rebuilt child re-roots its marks under this host —
                // without it the subtree is an orphan whose dirty bits can
                // never reach the window root.
                child.core().cell.set_parent(&self.core.cell);
                let claimed = super::transition::claim_transition(&child);
                let outgoing = core::mem::replace(&mut *self.child.borrow_mut(), child);
                self.retire(outgoing, renderer.frame_instant);
                let transitions = &self.transitions;
                *transitions.claimed.borrow_mut() = claimed;
                *transitions.insertion.borrow_mut() = transitions
                    .effective()
                    .filter(|_| !self.child.borrow().is_empty())
                    .map(|spec| {
                        super::transition::TransitionRun::insertion(
                            &spec,
                            transitions.reduce_motion,
                            renderer.frame_instant,
                        )
                    });
                renderer.state.counters.structural_patches += 1;
                true
            }
            None => false,
        }
    }

    /// Detaches the outgoing instance: under a removal declaration it becomes
    /// a ghost painting at the frame it was last flushed at; otherwise it
    /// drops at once. Either way the slot is the live content's alone.
    fn retire(&self, outgoing: RenderNode<E>, now: Instant) {
        let transitions = &self.transitions;
        let Some(spec) = transitions.effective() else {
            return;
        };
        if outgoing.is_empty() {
            return;
        }
        transitions.ghosts.borrow_mut().push(DynamicGhost {
            node: outgoing,
            removal: super::transition::TransitionRun::removal(
                &spec,
                transitions.reduce_motion,
                now,
            ),
            context: transitions.last_context.get(),
        });
        transitions.insertion.borrow_mut().take();
    }

    /// `apply_pending` for a pass reached after the frame's `patch` result was
    /// already folded into the window's structural bookkeeping: the change is
    /// reported to the renderer like `RetainedSubview::patch_built` reports
    /// widget-owned patches, so the next refresh runs the prune cycle for the
    /// dropped subtree's animation/measurement slots. The swap also marks this
    /// host layout-dirty — the parent's placement of this node preceded the
    /// new child, so an ancestor must re-lay out to size it.
    pub(super) fn apply_pending_mid_pass(&self, renderer: &mut SemanticCore<E>) -> bool {
        let applied = self.apply_pending(renderer);
        if applied {
            self.layout_dirty.set(true);
            renderer.note_subview_structural_change();
        }
        applied
    }

    /// Advances the insertion and every ghost on their own clocks: a settled
    /// insertion paints untransformed, a finished ghost releases its slot.
    /// While anything is in flight the host's layout is marked so frames keep
    /// coming. Returns whether a ghost left (a structural change) or anything
    /// is still animating — both need a fresh layout.
    pub(super) fn advance_transitions(&self, renderer: &SemanticCore<E>) -> bool {
        let now = renderer.frame_instant;
        let transitions = &self.transitions;
        let mut animating = false;
        {
            let mut insertion = transitions.insertion.borrow_mut();
            if let Some(running) = insertion.as_mut() {
                if running.advance(now) {
                    animating = true;
                } else {
                    *insertion = None;
                }
            }
        }
        let mut ghosts = transitions.ghosts.borrow_mut();
        let before = ghosts.len();
        ghosts.retain_mut(|ghost| ghost.removal.advance(now));
        let dropped = ghosts.len() != before;
        animating |= !ghosts.is_empty();
        if animating {
            renderer.context_mark_layout();
        }
        dropped || animating
    }

    /// Flushes the live instance, under its insertion transition while one
    /// runs. This host's ghosts paint first, beneath it, each at the frame it
    /// left at: they overlap the slot the live content alone sizes and take
    /// no part in layout (layout-spec.md §4.5), so the enclosing layout sees
    /// this host as its one child whatever that layout is.
    pub(super) fn flush_content(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        env: &Environment,
    ) {
        let child = self.child.borrow();
        let (slot_ctx, delta) = super::transition::slot_context(ctx);
        for ghost in self.transitions.ghosts.borrow().iter() {
            self.flush_ghost(renderer, ctx, ghost);
        }
        if !child.is_empty() {
            self.transitions.last_context.set(ctx);
        }
        {
            let insertion = self.transitions.insertion.borrow();
            if let Some(insertion) = insertion.as_ref() {
                let paint = super::transition::TransitionPaint::sample(
                    &insertion.state,
                    slot_size(slot_ctx),
                    self.transitions.direction,
                    true,
                    &insertion.runtime,
                    &self.core.cell,
                );
                super::transition::flush_transitioned(
                    renderer,
                    slot_ctx,
                    delta,
                    &paint,
                    |renderer, ctx| {
                        child.flush(renderer, ctx, env, kurbo::Affine::IDENTITY);
                    },
                );
            } else {
                child.flush(renderer, ctx, env, kurbo::Affine::IDENTITY);
            }
        }
    }

    /// Paints `ghost` at the frame it left at. Both that frame and the host's
    /// flush context `ctx` live in the drawing space of the host's nearest
    /// enclosing layer, so the ghost stays where it was placed in that space
    /// even when the host's own slot moves, and follows the layer's movement
    /// (scrolling, ancestors). Paint only: no input, no accessibility.
    fn flush_ghost(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        ghost: &DynamicGhost<E>,
    ) {
        // The left frame relative to the host's current placement.
        let relative = ctx.local().inverse() * ghost.context.local();
        let (slot_ctx, slot_delta) =
            super::transition::slot_context(ctx.child(relative, ghost.context.bounds()));
        let delta = relative * slot_delta;
        let paint = super::transition::TransitionPaint::sample(
            &ghost.removal.state,
            slot_size(slot_ctx),
            self.transitions.direction,
            false,
            &ghost.removal.runtime,
            &self.core.cell,
        );
        let env = &self.env;
        renderer.with_suppressed_accessibility(|renderer| {
            super::transition::flush_transitioned(
                renderer,
                slot_ctx,
                delta,
                &paint,
                |renderer, ctx| {
                    ghost
                        .node
                        .flush(renderer, ctx, env, kurbo::Affine::IDENTITY);
                },
            );
        });
    }
}

/// The logical size of a slot context's bounds.
const fn slot_size(ctx: RenderContext) -> Size {
    Size::new(
        crate::num_cast::f64_as_f32(ctx.bounds().width()),
        crate::num_cast::f64_as_f32(ctx.bounds().height()),
    )
}

impl<E: crate::text::TextEngine> TextNode<E> {
    /// Emit this text leaf's accessibility node, mirroring
    /// [`Native<TextConfig>::accessibility`] so the render-tree path produces the
    /// same a11y tree the dispatch path did. Called from `flush` with the node's
    /// scoped environment (label/role resolution reads env).
    ///
    /// Returns the node's id — the parent per-span `Link` children attach
    /// under — or `None` when the leaf published no node (hidden, empty, or
    /// its label donated to a naming claim).
    #[cfg(feature = "accessibility")]
    pub(super) fn emit_accessibility(
        renderer: &mut crate::renderer::SemanticCore<E>,
        ctx: Option<RenderContext>,
        styled: &StyledStr,
        env: &Environment,
    ) -> Option<AccessibilityNodeId> {
        if env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden)
        {
            return None;
        }
        let plain = styled.to_semantic().to_string();
        let default_label = (!plain.is_empty()).then_some(plain);
        let label = renderer.resolve_accessibility_label(env, default_label);
        let value = renderer.resolve_accessibility_value(env, None);
        if label.is_none() && value.is_none() {
            return None;
        }
        if let Some(label) = &label
            && renderer.consume_accessibility_descendant_text(env, label)
        {
            return None;
        }
        let mut node = AccessibilityNode::new(SemanticCore::<E>::resolve_accessibility_role(
            env,
            AccessibilityNodeRole::Label,
        ));
        if let Some(label) = label {
            node.set_label(label);
        }
        if let Some(value) = value {
            node.set_value(value);
        }
        renderer.register_accessibility_leaf(ctx, node, env, None)
    }

    /// Emit one `Link` accessibility node per [`TapSpan`] of `styled` —
    /// children of the text's own node when `parent` carries it, siblings
    /// under the ambient parent when the leaf published none. `span_rects`
    /// is each span's node-local rectangles on the rendered walk (a span the
    /// line-limit ellipsis cut entirely emits nothing); `None` is the
    /// semantic walk's no-geometry emission of the same nodes.
    #[cfg(feature = "accessibility")]
    pub(super) fn emit_tap_span_accessibility(
        renderer: &mut crate::renderer::SemanticCore<E>,
        span_rects: Option<&[Vec<kurbo::Rect>]>,
        tap_spans: &[TapSpan],
        plain: &str,
        env: &Environment,
        parent: Option<AccessibilityNodeId>,
    ) {
        if tap_spans.is_empty()
            || env
                .get::<AccessibilityHidden>()
                .is_some_and(AccessibilityHidden::is_hidden)
        {
            return;
        }
        let disabled = env
            .get::<waterui_core::interaction::Disabled>()
            .is_some_and(|disabled| renderer.read_signal(disabled.signal()));
        if let Some(parent) = parent {
            renderer.push_accessibility_parent(parent);
        }
        for (index, span) in tap_spans.iter().enumerate() {
            let bounds = span_rects.and_then(|rects| {
                let rects = rects.get(index)?;
                (!rects.is_empty()).then(|| {
                    rects
                        .iter()
                        .skip(1)
                        .fold(rects[0], |union, rect| union.union(*rect))
                })
            });
            if span_rects.is_some() && bounds.is_none() {
                continue;
            }
            let mut node = AccessibilityNode::new(AccessibilityNodeRole::Link);
            node.set_label(plain[span.range()].to_string());
            node.add_action(AccessibilityAction::Focus);
            let action_target = if disabled {
                node.set_disabled();
                None
            } else {
                node.add_action(AccessibilityAction::Click);
                let span = span.clone();
                let captured_env = env.clone();
                Some(AccessibilityActionTarget::Activate {
                    action: Rc::new(RefCell::new(
                        move |_renderer: &mut crate::renderer::SemanticCore<E>,
                              runtime_env: &Environment| {
                            span.activate(&captured_env.layered_on(runtime_env));
                            true
                        },
                    )),
                })
            };
            if let Some(bounds) = bounds {
                let _ = renderer.register_accessibility_node(node, bounds, env, action_target);
            } else {
                let _ = renderer.register_accessibility_node_semantic(node, env, action_target);
            }
        }
        if parent.is_some() {
            renderer.pop_accessibility_parent();
        }
    }

    /// Re-register each [`TapSpan`]'s tap targets at the rectangles `layout`
    /// reports for its byte range — one target per rectangle, so a wrapped
    /// span taps on every line it covers — then emit the spans' `Link`
    /// accessibility children. The targets sit at this node's depth, so a
    /// tap inside a span reaches the span's action while a tap outside every
    /// span hits no target here and falls through to whatever lies behind.
    /// Called from `flush` after the text recorded, under the node's render
    /// owner so target ancestry matches the node that emitted them.
    pub(super) fn flush_tap_spans(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        layout: &E::Layout,
        styled: &StyledStr,
        env: &Environment,
        #[cfg(feature = "accessibility")] text_node_id: Option<AccessibilityNodeId>,
    ) {
        let tap_spans = styled.tap_spans();
        if tap_spans.is_empty() {
            return;
        }
        let disabled = env
            .get::<waterui_core::interaction::Disabled>()
            .is_some_and(|disabled| renderer.read_signal(disabled.signal()));
        // The recording's origin is the bounds' top-left: a layout-space
        // rectangle lands there offset into this node's local space.
        let origin = ctx.bounds().origin().to_vec2();
        let group_id = renderer
            .gesture_group_id_for_identity(Rc::as_ptr(&self.accessibility_identity) as usize);
        let plain = styled.to_plain();
        let mut targets = self.tap_span_targets.borrow_mut();
        let mut index = 0usize;
        #[cfg(feature = "accessibility")]
        let mut span_rects = Vec::with_capacity(tap_spans.len());
        for span in tap_spans {
            let range = span.range();
            let mut rects = Vec::new();
            layout.selection_rects(
                layout.selection(
                    crate::text::TextPosition::in_text(range.start, plain.len()),
                    crate::text::TextPosition::in_text(range.end, plain.len()),
                ),
                |rect| rects.push(rect + origin),
            );
            if !disabled {
                for rect in rects.iter().copied() {
                    if let Some(entry) = targets.get_mut(index) {
                        *entry.slot.borrow_mut() = TapSpanSlot {
                            span: span.clone(),
                            env: env.clone(),
                        };
                        renderer.register_retained_gesture_target(&entry.target, rect, group_id);
                    } else {
                        let slot = Rc::new(RefCell::new(TapSpanSlot {
                            span: span.clone(),
                            env: env.clone(),
                        }));
                        let action: BoxedAction<()> = Box::new({
                            let slot = Rc::clone(&slot);
                            move |runtime_env: &Environment| {
                                let slot = slot.borrow();
                                slot.span.activate(&slot.env.layered_on(runtime_env));
                            }
                        });
                        let target = renderer.register_gesture_target(
                            rect,
                            group_id,
                            Gesture::Tap(TapGesture::new()),
                            action,
                        );
                        targets.push(TapSpanTarget { target, slot });
                    }
                    index += 1;
                }
            }
            #[cfg(feature = "accessibility")]
            span_rects.push(rects);
        }
        targets.truncate(index);
        #[cfg(feature = "accessibility")]
        Self::emit_tap_span_accessibility(
            renderer,
            Some(&span_rects),
            tap_spans,
            &plain,
            env,
            text_node_id,
        );
    }

    #[cfg(not(feature = "accessibility"))]
    // empty where the accessibility feature is off
    #[allow(clippy::missing_const_for_fn)]
    pub(super) fn emit_accessibility(
        _renderer: &mut crate::renderer::SemanticCore<E>,
        _ctx: Option<RenderContext>,
        _styled: &StyledStr,
        _env: &Environment,
    ) {
    }
}

/// Emit an `Image`-role accessibility node for a self-drawn graphics leaf
/// (`Canvas`/`SceneView`, `GpuSurface`, shapes/gradients) at its bounds, reading
/// the role/label/value scoped into `env` by any `.a11y_role()` /
/// `.a11y_label()` / `.a11y_value()` wrappers. These leaves draw their own
/// pixels, so the node tree is the only place their semantic node can be
/// emitted — mirroring `TextNode::emit_accessibility` for the text leaf.
/// Suppressed when the subtree is accessibility-hidden.
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
pub(super) fn emit_graphics_image_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
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
    let mut node = AccessibilityNode::new(SemanticCore::<E>::resolve_accessibility_role(
        env,
        AccessibilityNodeRole::Image,
    ));
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
pub(super) fn emit_graphics_image_accessibility<E: crate::text::TextEngine>(
    _renderer: &mut crate::renderer::SemanticCore<E>,
    _ctx: Option<RenderContext>,
    _env: &Environment,
    _default_label: Option<String>,
    _default_value: Option<String>,
    _focusable: bool,
) {
}

#[cfg(feature = "frame-profile")]
impl<E: crate::text::TextEngine> RetainedSubview<E> {
    /// Contributes this retained sub-view's last layout answer and its node's
    /// placed geometry to the frame's layout digest. Runs only on trees that
    /// already laid out, so the sub-view is built by then.
    pub(super) fn signature_into(&self, hasher: &mut SignatureHasher) {
        use std::hash::Hash;
        let built = self.expect_built("signature_into");
        hash_size(hasher, built.laid_out);
        built.laid_out_proposal.is_some().hash(hasher);
        if let Some(proposal) = built.laid_out_proposal {
            proposal.width.map(f32::to_bits).hash(hasher);
            proposal.height.map(f32::to_bits).hash(hasher);
        }
        built
            .node
            .signature_into(Rect::from_size(built.laid_out), hasher);
    }
}

#[cfg(feature = "frame-profile")]
impl<K: Eq + core::hash::Hash + Clone, E: crate::text::TextEngine> VisibleSubviewCache<K, E> {
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
