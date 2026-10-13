//! Retained collection nodes: [`CollectionNode`] (reactive, non-virtualized,
//! reconciled by id) and [`LazyStackNode`] (viewport-virtualized lazy stack).

#[cfg(feature = "accessibility")]
use super::layout::kurbo_rect;
#[cfg(feature = "accessibility")]
use super::layout::resolved_content_rect;
// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use std::rc::Rc;

use nami::collection::CollectionChange;
use waterui_core::transition::{
    Ghost, GhostSlot, TransitionGeometry, TransitionPhase, TransitionSpec, TransitionState,
};
use waterui_core::views::ViewSnapshot;

/// Collect into `out` the ids occupying the positions a [`CollectionChange`]
/// reports as replaced: `change.replaced` names new-snapshot indices, and
/// `snapshot` is the immutable row set the notification carried — index-
/// parallel with the collection it reported on. A producer that knows only a
/// whole-value replacement reports `everything`, so an empty change means
/// nothing needs re-materializing.
pub fn collect_replaced_ids<S: ViewSnapshot>(
    snapshot: &S,
    change: &CollectionChange,
    out: &mut std::collections::HashSet<S::Id>,
) {
    for range in &change.replaced {
        for index in range.start..range.end {
            if let Some(id) = snapshot.get_id(index) {
                out.insert(id);
            }
        }
    }
}

/// Splits a `.transition(...)` metadata layer off a collection item: the item's
/// declared [`TransitionSpec`], and the content to realize. The collection keeps
/// the spec instead of letting it reach the retained tree — the site, not the
/// item's node, drives insertion/removal clocks and retains the removal ghost
/// (layout-spec.md §4.5).
pub(super) fn split_transition(view: AnyView) -> (Option<TransitionSpec>, AnyView) {
    match view.downcast::<waterui_core::Metadata<TransitionSpec>>() {
        Ok(metadata) => (Some(metadata.value), metadata.content),
        Err(view) => (None, view),
    }
}

/// The frame instant a per-clock lifetime last advanced at — owned by the
/// clock itself, so an idle stretch between collection frames never leaks
/// into a fresh clock's first delta.
pub(super) struct TransitionClock {
    last_advanced_at: Instant,
}

impl TransitionClock {
    const fn new(now: Instant) -> Self {
        Self {
            last_advanced_at: now,
        }
    }

    /// Advances `state` by the time elapsed since this clock's last sample.
    fn advance(&mut self, state: &mut TransitionState, now: Instant) -> bool {
        let delta = now.saturating_duration_since(self.last_advanced_at);
        self.last_advanced_at = now;
        state.advance(delta)
    }
}

/// An in-flight insertion: the state and its own clock, so a state cannot
/// exist without the timestamp that drives it.
pub(super) struct CollectionInsertion {
    transition: TransitionState,
    clock: TransitionClock,
}

/// The dimensions a live entry freezes at for its removal ghost: measured at
/// the slot's actual placed proposal, carrying the slot's explicit guides,
/// with size pinned to the placed frame so later proposals cannot reflow it.
fn frozen_placement_dimensions(sub: &dyn SubView, placement: &SubviewPlacement) -> ViewDimensions {
    let mut dimensions = sub.measure(placement.proposal);
    dimensions.size = *placement.frame.size();
    dimensions
}

/// One retained entry of a [`CollectionNode`]: the item's persistent node, its
/// `.transition()` declaration, and the in-flight insertion clock.
pub(super) struct CollectionEntry {
    /// Stable item identity from the source collection.
    pub(super) id: CollectionItemId,
    /// The entry's own mount core: the layer its transition scope writes and
    /// the cell a transition marks, distinct from `node`'s so an entry
    /// transition re-places only the entry, not the subtree inside it.
    pub(crate) core: NodeCore,
    /// The item's retained node, kept across membership changes by id.
    pub(super) node: RenderNode,
    /// The `.transition(...)` spec peeled off the item at materialize; `None`
    /// means this item pops on membership changes.
    pub(super) spec: Option<TransitionSpec>,
    /// Independent insertion clock while the entry animates in; `None` at rest
    /// and forever when the item declared no spec.
    pub(super) insertion: Option<CollectionInsertion>,
    /// The dimensions and layout priority the entry last measured at — the
    /// frozen geometry its removal ghost's [`GhostSlot`] retains.
    last_measured: std::cell::RefCell<(ViewDimensions, i32)>,
}

impl CollectionEntry {
    /// An at-rest entry carrying the item's declared spec. The initial
    /// membership of a collection is built from these (no insertion runs);
    /// later additions get their clock from [`Self::begin_insertion`].
    pub(super) fn new(
        id: CollectionItemId,
        node: RenderNode,
        core: NodeCore,
        spec: Option<TransitionSpec>,
    ) -> Self {
        Self {
            id,
            core,
            node,
            spec,
            insertion: None,
            last_measured: std::cell::RefCell::new((ViewDimensions::default(), 0)),
        }
    }

    /// Starts this entry's insertion clock from its spec, if it has one —
    /// timestamped at reconciliation's frame instant, not the previous frame.
    fn begin_insertion(&mut self, reduce_motion: bool, now: Instant) {
        if let Some(spec) = &self.spec {
            // The reduced form is resolved by `start` first, so a reduced
            // opacity transition is still valid — only a transition that
            // still needs pixel capture is unsupported.
            let transition = spec.start(TransitionPhase::Insertion, reduce_motion);
            assert!(
                !transition.captures_content(),
                "Hydrolysis: pixel-capture transitions are unsupported on collection items (water-rs/waterui#1593)"
            );
            self.insertion = Some(CollectionInsertion {
                transition,
                clock: TransitionClock::new(now),
            });
        }
    }
}

pub struct CollectionNode {
    /// Per-frame measure memo gate: records whether this node's body was
    /// re-probed within a frame, gating `memo_slots` so a node measured
    /// once per frame pays a `Cell` update instead of a `RefCell` borrow.
    pub(crate) memo_gate: Cell<MemoGate>,
    /// The proposal ring `measure` consults once `memo_gate` marks this
    /// node as re-probed. Owned by the node, so a dropped node never
    /// leaves a stale answer behind.
    pub(crate) memo_slots: RefCell<NodeMeasureEntry>,
    /// The container layout (e.g. `AbsoluteLayout`, `ZStackLayout`).
    pub(super) layout: Box<dyn Layout>,
    /// The immutable row set `entries` reflects: the membership watcher swaps
    /// in the snapshot each event carries, so reconcile, measure, and flush
    /// all read exactly the membership the last applied event reported —
    /// never the live source mid-notification. Shared with the watcher.
    pub(super) snapshot: Rc<RefCell<AnyViewsSnapshot<AnyView>>>,
    /// Environment captured at build, used to materialize items. Already shielded
    /// when this collection carries accessibility naming metadata — the name
    /// belongs to the collection, not to every item in it.
    pub(super) env: Environment,
    /// Stable identity owning this collection's own accessibility node id, so the
    /// id survives membership changes shifting the sibling ordinals.
    pub(super) accessibility_identity: Rc<()>,
    /// The collection's own mount core: cell, placement and subscriptions.
    pub(crate) core: NodeCore,
    /// The unshielded environment when this collection carries accessibility
    /// naming metadata: `Some` means it emits the node naming itself.
    #[cfg(feature = "accessibility")]
    pub(super) accessibility_container_env: Option<Environment>,
    /// Current entries in display order, keyed by id so a membership change
    /// keeps unchanged items' nodes (and their in-flight state) and only
    /// builds/drops the delta. With a transition this also holds still-exiting
    /// entries — anchored after the live id they followed — until their
    /// fade-out completes.
    pub(super) entries: Vec<CollectionEntry>,
    /// Child frames cached by [`RenderNode::layout`], reused by `flush`. During
    /// a transition each frame holds the entry's full extent at its re-stacked
    /// position; flush clips it to the presence factor.
    pub(super) placed: Vec<Rect>,
    /// The extent this collection resolved at layout — the size it answered to
    /// the selected proposal, centred on the assigned frame — kept beside
    /// `placed` so a naming-scope owner reports its own extent when the
    /// parent assigned more than it answered (water-rs/hydrolysis#51).
    #[cfg(feature = "accessibility")]
    pub(super) resolved: Rect,
    /// Detached removals still animating out, each anchored after the live id
    /// it followed (`None` = before the first live entry). A ghost's node is
    /// out of semantics, accessibility, focus and hit-testing — it only keeps
    /// painting inside a `hit: false` scope until its clock completes.
    pub(super) ghosts: Vec<CollectionGhost>,
    /// Ghost slot frames in `ghosts` order, filled by `layout` beside `placed`.
    pub(super) ghost_placed: Vec<Rect>,
    /// Reduce Motion resolved once at build from the item environment: every
    /// clock started here maps through `Transition::reduced` when set.
    pub(super) reduce_motion: bool,
    /// Logical-edge resolution for sampled transition properties.
    pub(super) direction: waterui_core::layout::LayoutDirection,
    /// Stable discriminator for ghost scope keys.
    pub(super) ghost_seq: Cell<u64>,
    /// Set by the membership watcher; consumed by `patch` to trigger a reconcile.
    pub(super) dirty: Rc<Cell<bool>>,
    /// Ids the watcher reported as replaced since the last reconcile — the
    /// items whose content may differ under an unchanged id, so their nodes
    /// are rebuilt while every other surviving id keeps its node and state.
    /// Shared with the watcher closure via `Rc`.
    pub(super) replaced_ids: Rc<RefCell<std::collections::HashSet<CollectionItemId>>>,
    /// Membership-change watcher; a change sets `dirty` and schedules a refresh.
    pub(super) _guard: BoxWatcherGuard,
    /// The collection layout's own `watch_invalidation` subscriptions — a
    /// change to a layout-input signal schedules the refresh that re-derives
    /// the collection's extents and item rects.
    pub(super) _layout_guards: Vec<BoxWatcherGuard>,
}

pub struct LazyStackNode {
    /// Per-frame measure memo gate: records whether this node's body was
    /// re-probed within a frame, gating `memo_slots` so a node measured
    /// once per frame pays a `Cell` update instead of a `RefCell` borrow.
    pub(crate) memo_gate: Cell<MemoGate>,
    /// The proposal ring `measure` consults once `memo_gate` marks this
    /// node as re-probed. Owned by the node, so a dropped node never
    /// leaves a stale answer behind.
    pub(crate) memo_slots: RefCell<NodeMeasureEntry>,
    /// Stack axis + spacing + cross-axis alignment.
    pub(super) axis: LazyStackAxisConfig,
    /// The immutable row set the membership watcher last applied: each event
    /// carries its own snapshot, which the watcher swaps in so the virtual
    /// window, id keys, and materialization all read one coherent membership
    /// — never the live source mid-notification. Shared with the watcher.
    pub(super) snapshot: Rc<RefCell<AnyViewsSnapshot<AnyView>>>,
    /// Environment captured at build, used to materialize and style items. Already
    /// shielded when this stack carries accessibility naming metadata — the name
    /// belongs to the stack, not to every row in it.
    pub(super) env: Environment,
    /// Stable identity owning this stack's own accessibility node id, so the id
    /// survives the visible window shifting the sibling ordinals.
    pub(super) accessibility_identity: Rc<()>,
    /// The stack's own mount core: cell, placement and subscriptions.
    pub(crate) core: NodeCore,
    /// The §7.1 context the stack was last laid out against — node-lifetime
    /// storage, so a stack inside an unchanged retained sub-view still hands
    /// each materialized item its context at flush.
    pub(super) safe_area: Option<Box<safe_area::SafeAreaLayout>>,
    /// The unshielded environment when this stack carries accessibility naming
    /// metadata: `Some` means it emits the node naming itself.
    #[cfg(feature = "accessibility")]
    pub(super) accessibility_container_env: Option<Environment>,
    /// Indexed measured/estimated extents. Deep jumps resolve through its
    /// Fenwick tree without materializing preceding items.
    pub(super) extent_index: RefCell<VirtualExtentIndex>,
    /// Retained node sub-views for the items currently in the visible window, keyed
    /// by stable id so a steady scroll reuses each visible item's node (keeping its
    /// reactive content live) and only builds items entering the window.
    pub(super) item_cache: RefCell<VisibleSubviewCache<CollectionItemId>>,
    /// Index range materialized by the previous flush. Those retained items are
    /// the only rows that need exact pre-layout remeasurement on a reactive
    /// height change; all other rows keep their virtual estimate.
    pub(super) visible_range: RefCell<Range<usize>>,
    /// The main-axis span (in this stack's coordinates) the previous flush
    /// resolved as visible. `patch_visible` reuses it to materialize the
    /// window's items — including ids a membership change slid into it — while
    /// the patch walk is still running, so a builder's reactive writes land in
    /// the patch phase rather than mid-encode (water-rs/hydrolysis#226).
    pub(super) visible_span: Cell<Option<(f64, f64)>>,
    /// Estimated extent for not-yet-measured items, seeded from the first measure;
    /// used to size the scroll content without measuring the whole collection.
    pub(super) estimate: Cell<f64>,
    /// First-item dimensions and the cross-axis query that produced them.
    pub(super) estimate_sample: Cell<Option<(Option<f32>, Size)>>,
    /// First-item main-axis minimum and the cross-axis query that produced it,
    /// sampled the same way `estimate_sample` is. The sum of item minima is
    /// the stack's honest floor: rigid items overflow rather than pretend to
    /// shrink, exactly as the eager stack's `minima_overflow` answers.
    pub(super) floor_sample: Cell<Option<(Option<f32>, f64)>>,
    /// Membership changes reset index-based measurements, including moves that
    /// preserve the collection length.
    pub(super) dirty: Rc<Cell<bool>>,
    /// Ids the watcher reported as replaced since the last consume — the
    /// items whose content may differ under an unchanged id. `patch_visible`
    /// drops exactly those ids' retained rows; untouched rows keep their
    /// nodes. Shared with the watcher closure via `Rc`.
    pub(super) replaced_ids: Rc<RefCell<std::collections::HashSet<CollectionItemId>>>,
    /// Membership-change watcher: a change schedules a window refresh, which
    /// re-resolves the visible window (the collection `len`/items are re-read).
    pub(super) _guard: BoxWatcherGuard,
    /// Direction-change watcher: locale changes immediately mirror placement.
    pub(super) _direction_guard: BoxWatcherGuard,
    /// The stack layout's own `watch_invalidation` subscriptions — e.g. a
    /// `spacing` signal change schedules the refresh that re-derives the
    /// extent index and item rects.
    pub(super) _layout_guards: Vec<BoxWatcherGuard>,
}

/// A detached outgoing item: its retained node (semantically dead, painted
/// only), the fixed-size [`GhostSlot`] it still occupies, and the independent
/// removal clock driving it (layout-spec.md §4.5).
pub(super) struct CollectionGhost {
    /// Live id this ghost displays after; `None` places it before the first
    /// live entry.
    anchor: Option<CollectionItemId>,
    ghost: Ghost<RenderNode>,
    /// This ghost's own clock — removal runs on its own timeline, never on
    /// the collection's shared tick.
    clock: TransitionClock,
    /// Stable discriminator for this ghost's paint scope.
    scope_item: u64,
}

/// One layout/flush position in display order: a live entry or a ghost still
/// holding its slot.
#[derive(Clone, Copy)]
enum CollectionSlot {
    Entry(usize),
    Ghost(usize),
}

impl CollectionNode {
    /// Display order for measure, layout and flush: head-anchored ghosts first,
    /// then each live entry followed by the ghosts anchored to it. Ghosts whose
    /// anchor left in the same reconcile trail at the end.
    fn slot_order(&self) -> Vec<CollectionSlot> {
        let mut order = Vec::with_capacity(self.entries.len() + self.ghosts.len());
        for (index, ghost) in self.ghosts.iter().enumerate() {
            if ghost.anchor.is_none() {
                order.push(CollectionSlot::Ghost(index));
            }
        }
        let mut anchored = vec![false; self.ghosts.len()];
        for (index, ghost) in self.ghosts.iter().enumerate() {
            if ghost.anchor.is_none() {
                anchored[index] = true;
            }
        }
        for (entry_index, entry) in self.entries.iter().enumerate() {
            order.push(CollectionSlot::Entry(entry_index));
            for (index, ghost) in self.ghosts.iter().enumerate() {
                if ghost.anchor == Some(entry.id) {
                    order.push(CollectionSlot::Ghost(index));
                    anchored[index] = true;
                }
            }
        }
        for (index, _) in anchored.iter().enumerate().filter(|(_, a)| !**a) {
            order.push(CollectionSlot::Ghost(index));
        }
        order
    }

    /// Build the `NodeSubView` proxies for this collection's entries, sharing one
    /// state cell across the level (mirrors [`ContainerNode`] measurement).
    pub(super) fn measure(
        &self,
        state: &mut HydroState,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let cell = RefCell::new(state);
        let subs: Vec<NodeSubView> = self
            .entries
            .iter()
            .map(|entry| NodeSubView::new(&entry.node, &cell, &self.env, theme))
            .collect();
        // Every member — live entries and still-held ghost slots — is a normal
        // layout member: a ghost keeps the fixed slot it had at removal until
        // its clock completes, and an insertion allocates its slot immediately.
        let order = self.slot_order();
        let refs: Vec<&dyn SubView> = order
            .iter()
            .map(|slot| match *slot {
                CollectionSlot::Entry(index) => &subs[index] as &dyn SubView,
                CollectionSlot::Ghost(index) => &self.ghosts[index].ghost.slot as &dyn SubView,
            })
            .collect();
        ViewDimensions::new(self.layout.size_that_fits(proposal, &refs))
    }

    pub(super) fn layout(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        safe_area: Option<&safe_area::SafeAreaLayout>,
        proposal: ProposalSize,
        size: Size,
    ) {
        let env = self.env.clone();
        let theme = renderer.theme();
        let order = self.slot_order();
        let (placements, mut frozen, priorities) = {
            let cell = RefCell::new(&mut renderer.state);
            let subs: Vec<NodeSubView> = self
                .entries
                .iter()
                .map(|entry| NodeSubView::new(&entry.node, &cell, &env, &theme))
                .collect();
            let priorities: Vec<i32> = subs.iter().map(SubView::priority).collect();
            // Ghost slots stay ordinary layout members at their frozen size:
            // the container's membership only reflows when a ghost's clock
            // completes and its slot is released.
            let refs: Vec<&dyn SubView> = order
                .iter()
                .map(|slot| match *slot {
                    CollectionSlot::Entry(index) => &subs[index] as &dyn SubView,
                    CollectionSlot::Ghost(index) => &self.ghosts[index].ghost.slot as &dyn SubView,
                })
                .collect();
            // Only the accessibility scope reads `resolved` — skip the measure
            // in builds without it.
            #[cfg(feature = "accessibility")]
            {
                self.resolved =
                    resolved_content_rect(self.layout.size_that_fits(proposal, &refs), size);
            }
            let placed = self.layout.place(Rect::from_size(size), proposal, &refs);
            // Freeze each live entry's placed dimensions (size plus explicit
            // guides at the placed proposal) while `subs` is still borrowed —
            // the removal ghost keeps them fixed afterward.
            let mut frozen: Vec<Option<ViewDimensions>> = vec![None; self.entries.len()];
            for (slot, placement) in order.iter().zip(&placed) {
                if let CollectionSlot::Entry(index) = *slot {
                    frozen[index] = Some(frozen_placement_dimensions(&subs[index], placement));
                }
            }
            (placed, frozen, priorities)
        };
        let mut entry_frames = vec![None; self.entries.len()];
        let mut ghost_frames = vec![None; self.ghosts.len()];
        for (slot, placement) in order.iter().zip(&placements) {
            match *slot {
                CollectionSlot::Entry(index) => entry_frames[index] = Some(placement),
                CollectionSlot::Ghost(index) => ghost_frames[index] = Some(placement),
            }
        }
        for (index, (entry, placement)) in self.entries.iter_mut().zip(&entry_frames).enumerate() {
            let Some(placement) = placement else { continue };
            if let Some(dimensions) = frozen[index].take() {
                entry.last_measured.replace((dimensions, priorities[index]));
            }
            let child_area = safe_area.map(|area| area.child(placement.frame));
            entry.node.layout(
                renderer,
                &env,
                child_area,
                placement.proposal,
                *placement.frame.size(),
            );
        }
        let empty_frame = Rect::from_size(Size::new(0.0, 0.0));
        self.placed = entry_frames
            .into_iter()
            .map(|placement| placement.map_or(empty_frame, |p| p.frame))
            .collect();
        self.ghost_placed = ghost_frames
            .into_iter()
            .map(|placement| placement.map_or(empty_frame, |p| p.frame))
            .collect();
    }

    pub(super) fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext) {
        #[cfg(feature = "accessibility")]
        let container_scope = self.accessibility_container_env.as_ref().map(|env| {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            let (bounds, extent) = (
                ctx.bounds,
                renderer.resolve_window_rect(kurbo_rect(self.resolved)),
            );
            let scope = renderer.begin_accessibility_container(bounds, Some(extent), env);
            renderer.pop_accessibility_owner();
            scope
        });
        for slot in self.slot_order() {
            match slot {
                CollectionSlot::Entry(index) => {
                    let (entry, rect) = (&self.entries[index], self.placed[index]);
                    let delta =
                        kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y())));
                    let child_ctx = ctx.child(
                        delta,
                        kurbo::Rect::new(
                            0.0,
                            0.0,
                            f64::from(rect.width()),
                            f64::from(rect.height()),
                        ),
                    );
                    if let Some(insertion) = &entry.insertion {
                        let geometry = TransitionGeometry {
                            size: *rect.size(),
                            direction: self.direction,
                        };
                        Self::flush_transitioned(
                            renderer,
                            &entry.node,
                            child_ctx,
                            &self.env,
                            insertion.transition.properties(geometry),
                            delta,
                            entry_scope_item(entry),
                            true,
                        );
                    } else {
                        entry.node.flush(renderer, child_ctx, &self.env, delta);
                    }
                }
                CollectionSlot::Ghost(index) => {
                    let (ghost, rect) = (&self.ghosts[index], self.ghost_placed[index]);
                    let delta =
                        kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y())));
                    let child_ctx = ctx.child(
                        delta,
                        kurbo::Rect::new(
                            0.0,
                            0.0,
                            f64::from(rect.width()),
                            f64::from(rect.height()),
                        ),
                    );
                    let geometry = TransitionGeometry {
                        size: *rect.size(),
                        direction: self.direction,
                    };
                    // The ghost is paint-only: no input, no accessibility.
                    renderer.with_suppressed_accessibility(|renderer| {
                        Self::flush_transitioned(
                            renderer,
                            &ghost.ghost.realization,
                            child_ctx,
                            &self.env,
                            ghost.ghost.transition.properties(geometry),
                            delta,
                            ghost.scope_item,
                            false,
                        );
                    });
                }
            }
        }
        #[cfg(feature = "accessibility")]
        if let Some(container_scope) = container_scope {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            renderer.end_accessibility_container(container_scope);
            renderer.pop_accessibility_owner();
        }
    }

    /// Emits every live entry's accessibility nodes for the semantic walk —
    /// ghosts are not entries, so a removed item leaves the tree the moment
    /// the membership change lands, exactly as §4.5 requires.
    #[cfg(feature = "accessibility")]
    pub(super) fn emit_accessibility(&self, renderer: &mut SemanticCore) {
        let container_scope = self.accessibility_container_env.as_ref().map(|env| {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            let scope = renderer.begin_accessibility_container_semantic(env);
            renderer.pop_accessibility_owner();
            scope
        });
        for entry in &self.entries {
            entry.node.emit_accessibility(renderer, &self.env);
        }
        if let Some(container_scope) = container_scope {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            renderer.end_accessibility_container(container_scope);
            renderer.pop_accessibility_owner();
        }
    }

    /// Flushes one entry or ghost inside a scope carrying the sampled
    /// transition properties: `opacity` as the scope alpha, the
    /// translation/scale/rotation compose about the slot's centre, and
    /// `hit_alpha` gates interaction the same way the paint fades. `hit:
    /// false` (ghosts) keeps the scope painted but removes all input. Blur is
    /// not applied — pixel effects render through the capture path (#1593).
    #[expect(
        clippy::too_many_arguments,
        reason = "the scope's transform, alpha, clip and hit state are independent inputs"
    )]
    fn flush_transitioned(
        renderer: &mut HydrolysisRenderer,
        node: &RenderNode,
        child_ctx: RenderContext,
        env: &Environment,
        properties: waterui_core::transition::TransitionProperties,
        delta: kurbo::Affine,
        scope_item: u64,
        hit: bool,
    ) {
        // Blur is a sampled property but has no paint path here — a nonzero
        // radius is an unsupported transition surface, not a no-op.
        assert!(
            properties.blur == 0.0,
            "Hydrolysis: blur transitions are unsupported pending water-rs/waterui#1593"
        );
        let bounds = child_ctx.bounds;
        let centre = (bounds.width() / 2.0, bounds.height() / 2.0);
        // Scale and rotation resolve about the slot's centre, then translation —
        // matching `TransitionProperties`' documented compose order.
        let local = kurbo::Affine::translate((
            f64::from(properties.translation[0]),
            f64::from(properties.translation[1]),
        )) * kurbo::Affine::translate(centre)
            * kurbo::Affine::rotate(f64::from(properties.rotation))
            * kurbo::Affine::scale_non_uniform(
                f64::from(properties.scale[0]),
                f64::from(properties.scale[1]),
            )
            * kurbo::Affine::translate((-centre.0, -centre.1));
        let layer = delta * local;
        let scope = crate::renderer::ScopeDelta {
            transform: layer,
            hit_alpha: properties.opacity,
        };
        let key = crate::renderer::mount::ScopeKey {
            role: "transition",
            item: scope_item,
        };
        renderer.open_scope_with(
            key,
            layer,
            properties.opacity,
            Some(&crate::renderer::frame::ScopeClip::Rect(kurbo::Rect::new(
                0.0,
                0.0,
                bounds.width(),
                bounds.height(),
            ))),
            child_ctx.local * local,
            scope,
            hit,
        );
        node.flush(renderer, child_ctx, env, kurbo::Affine::IDENTITY);
        renderer.close_scope();
    }

    /// Apply a membership change: keep each surviving id's node (and its
    /// in-flight insertion clock), build newly-present ids — starting their
    /// own insertion clock when the item declared a spec — and detach departed
    /// ones into [`CollectionGhost`]s anchored after the live id they
    /// followed. Entries the watcher flagged as replaced keep their id but
    /// their node (and spec) is re-materialized — a same-id content change
    /// rebuilds exactly that row. Re-insertion of a departed id is a new
    /// instance with a new slot; a same-id ghost keeps animating beside it
    /// and is never revived.
    pub(super) fn reconcile(&mut self, renderer: &mut SemanticCore) {
        renderer.state.counters.structural_patches += 1;
        let env = self.env.clone();
        // The retained snapshot is immutable: ids and views read here stay
        // coherent with the membership event that delivered it even when a
        // nested build mutates the live source mid-reconcile.
        let snapshot = self.snapshot.borrow().clone();
        let len = snapshot.len();

        let indices: Vec<usize> = snapshot.range().collect();
        let ids: Vec<CollectionItemId> = indices
            .iter()
            .map(|&index| {
                snapshot
                    .get_id(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} has no id"))
            })
            .collect();
        let live: std::collections::HashSet<CollectionItemId> = ids.iter().copied().collect();

        // Partition the previous display order into entries still live
        // (reusable, keyed by id) and departed ones, which become ghosts when
        // the item declared a removal transition and drop outright otherwise.
        let mut reuse_by_id: std::collections::BTreeMap<CollectionItemId, CollectionEntry> =
            std::collections::BTreeMap::new();
        let mut new_ghosts: Vec<CollectionGhost> = Vec::new();
        {
            let mut last_live: Option<CollectionItemId> = None;
            for entry in self.entries.drain(..) {
                if live.contains(&entry.id) {
                    last_live = Some(entry.id);
                    reuse_by_id.insert(entry.id, entry);
                } else if let Some(spec) = entry.spec {
                    let (dimensions, priority) = entry.last_measured.borrow().clone();
                    let ghost = Ghost::new(
                        entry.node,
                        GhostSlot::new(dimensions, priority),
                        &spec,
                        self.reduce_motion,
                    );
                    // The reduced form is already resolved — a spec that still
                    // needs pixel capture is unsupported here.
                    assert!(
                        !ghost.transition.captures_content(),
                        "Hydrolysis: pixel-capture transitions are unsupported on collection items (water-rs/waterui#1593)"
                    );
                    new_ghosts.push(CollectionGhost {
                        anchor: last_live,
                        ghost,
                        clock: TransitionClock::new(renderer.frame_instant),
                        scope_item: self.ghost_seq.replace(self.ghost_seq.get() + 1),
                    });
                }
            }
        }
        self.ghosts.extend(new_ghosts);

        let mut next = Vec::with_capacity(len);
        let replaced = core::mem::take(&mut *self.replaced_ids.borrow_mut());
        for (index, id) in indices.into_iter().zip(ids) {
            let entry = if let Some(mut previous) = reuse_by_id.remove(&id) {
                if replaced.contains(&id) {
                    // Same id, changed content: re-materialize this row's
                    // node and spec from the current item. The entry keeps
                    // its identity; an in-flight insertion clock keeps running.
                    let view = snapshot
                        .get_view(index)
                        .unwrap_or_else(|| panic!("hydrolysis collection: item {index} missing"));
                    let (spec, content) = split_transition(view);
                    previous.node =
                        RenderNode::build(normalize_layout_view(content, &env), &env, renderer);
                    previous.node.core().cell.set_parent(&previous.core.cell);
                    previous.spec = spec;
                }
                previous
            } else {
                let view = snapshot
                    .get_view(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} missing"));
                let (spec, content) = split_transition(view);
                let core = renderer.new_core();
                core.cell.set_parent(&self.core.cell);
                let node = RenderNode::build(normalize_layout_view(content, &env), &env, renderer);
                node.core().cell.set_parent(&core.cell);
                let mut entry = CollectionEntry::new(id, node, core, spec);
                entry.begin_insertion(self.reduce_motion, renderer.frame_instant);
                entry
            };
            next.push(entry);
        }
        self.entries = next;
    }

    /// Advance every live insertion and ghost clock by the frame delta:
    /// finished insertions settle, finished ghosts release their slot and
    /// drop (a structural change, so the caller runs the prune cycle), and
    /// while anything is still mid-flight a refresh is requested so frames
    /// keep coming until the collection settles. Returns whether the tree
    /// changed shape or is still animating (both need a fresh layout).
    pub(super) fn advance_transitions(&mut self, renderer: &SemanticCore) -> bool {
        let now = renderer.frame_instant;
        let mut animating = false;
        for entry in &mut self.entries {
            if let Some(insertion) = &mut entry.insertion {
                if insertion.clock.advance(&mut insertion.transition, now) {
                    animating = true;
                } else {
                    entry.insertion = None;
                }
            }
        }
        let before = self.ghosts.len();
        self.ghosts
            .retain_mut(|ghost| ghost.clock.advance(&mut ghost.ghost.transition, now));
        let dropped = self.ghosts.len() != before;
        animating |= !self.ghosts.is_empty();
        if animating {
            renderer.context_mark_layout();
        }
        dropped || animating
    }
}

/// Stable scope-key discriminator for a live entry's transition layer.
fn entry_scope_item(entry: &CollectionEntry) -> u64 {
    use core::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    entry.id.hash(&mut hasher);
    hasher.finish()
}

impl LazyStackNode {
    /// Applies structural updates owned by the currently visible retained items
    /// before the parent scroll view measures this stack. Items whose ids the
    /// stored viewport window covers but that were never materialized — e.g.
    /// slid into view by the same membership change being patched — are built
    /// here too: their builders run inside the patch phase, so a `set()` they
    /// perform lands its `Dynamic` pending while the walk can still reach the
    /// host's patch arm, instead of mid-encode inside `flush`.
    pub(super) fn patch_visible(&self, renderer: &mut SemanticCore) -> bool {
        let replaced = core::mem::take(&mut *self.replaced_ids.borrow_mut());
        if !replaced.is_empty() {
            // A same-id content change drops exactly those rows' retained
            // sub-views; they re-materialize from the collection's current
            // data when the visible window next fills them.
            self.item_cache.borrow_mut().invalidate_ids(&replaced);
            renderer.state.counters.structural_patches += 1;
        }
        let mut materialized = false;
        let snapshot = self.snapshot.borrow().clone();
        let count = snapshot.len();
        if count > 0
            && let Some((visible_start, visible_end)) = self.visible_span.get()
        {
            let estimate = self.estimate.get();
            let spacing = self.spacing(&mut renderer.state);
            {
                let mut extent_index = self.extent_index.borrow_mut();
                if estimate > 0.0 && !extent_index.matches(count, estimate, spacing) {
                    extent_index.reset(count, estimate, spacing);
                }
            }
            let window = self
                .extent_index
                .borrow()
                .visible_window(visible_start, visible_end);
            let views = &snapshot;
            let env = &self.env;
            let mut cache = self.item_cache.borrow_mut();
            for index in window.start..window.end.min(count) {
                let id = views
                    .get_id(index)
                    .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
                if cache.get(&id).is_none() {
                    let view = views.get_view(index).unwrap_or_else(|| {
                        panic!("hydrolysis LazyStack failed to materialize item {index}")
                    });
                    // Build now, not at flush: a cached-but-unbuilt entry
                    // measures as zero and is skipped by `patch_for_parent`,
                    // so the row would place at a collapsed rect this frame.
                    cache
                        .materialize(id, || normalize_layout_view(view, env))
                        .ensure_built(renderer, env);
                    materialized = true;
                }
            }
        }
        let changed = self.item_cache.borrow_mut().patch_for_parent(renderer);
        if changed || materialized {
            self.estimate_sample.set(None);
            self.floor_sample.set(None);
            renderer.state.counters.structural_patches += 1;
        }
        changed | materialized
    }

    fn spacing(&self, state: &mut HydroState) -> f64 {
        match &self.axis {
            LazyStackAxisConfig::Vertical { spacing, .. }
            | LazyStackAxisConfig::Horizontal { spacing, .. } => {
                f64::from(state.measure_signal(spacing))
            }
        }
    }

    fn item_proposal(&self, cross: Option<f32>) -> ProposalSize {
        match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => ProposalSize::new(cross, None),
            LazyStackAxisConfig::Horizontal { .. } => ProposalSize::new(None, cross),
        }
    }

    /// The per-item probe that answers each item's main-axis minimum — what
    /// the eager stack's `with_main(0)` probe would ask of it.
    fn item_min_proposal(&self, cross: Option<f32>) -> ProposalSize {
        match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => ProposalSize::new(cross, Some(0.0)),
            LazyStackAxisConfig::Horizontal { .. } => ProposalSize::new(Some(0.0), cross),
        }
    }

    /// Measures item `index` under the given proposal against `snapshot` —
    /// the retained row set the current pass is reconciling — returning its
    /// measured size and stretch axis (both needed to place it).
    fn measure_item(
        &self,
        snapshot: &AnyViewsSnapshot<AnyView>,
        state: &mut HydroState,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        index: usize,
        proposal: ProposalSize,
    ) -> (Size, StretchAxis) {
        let id = snapshot
            .get_id(index)
            .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
        if let Some(item) = self.item_cache.borrow().get(&id)
            && item.is_built()
        {
            return (
                item.measure_built_with_proposal(state, &self.env, theme, proposal),
                item.stretch_axis(),
            );
        }

        let view = snapshot
            .get_view(index)
            .unwrap_or_else(|| panic!("hydrolysis LazyStack failed to materialize item {index}"));
        let view = normalize_layout_view(view, &self.env);
        state.measurement.begin_transient_measurement();
        let result = {
            let bound = RefCell::new(&mut *state);
            let subview = HydroSubview::from_view(&view, &bound, &self.env, theme);
            (subview.measure(proposal).size, subview.stretch_axis())
        };
        state.measurement.end_transient_measurement();
        result
    }

    fn main_extent(&self, size: Size) -> f64 {
        match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => f64::from(size.height),
            LazyStackAxisConfig::Horizontal { .. } => f64::from(size.width),
        }
    }

    /// The first item's main-axis minimum under the same cross query, cached
    /// alongside `estimate_sample`: what the stack answers at `with_main(0)`.
    /// Content that cannot shrink — a fixed-size row — keeps its intrinsic
    /// extent here, so the stack overflows instead of collapsing.
    fn ensure_floor(
        &self,
        snapshot: &AnyViewsSnapshot<AnyView>,
        state: &mut HydroState,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        cross: Option<f32>,
    ) -> f64 {
        if let Some((previous, floor)) = self.floor_sample.get()
            && previous == cross
        {
            return floor;
        }
        let (size, _) = self.measure_item(snapshot, state, theme, 0, self.item_min_proposal(cross));
        let floor = self.main_extent(size);
        self.floor_sample.set(Some((cross, floor)));
        floor
    }

    /// Keeps estimates scoped to the cross-axis query and collection lifetime.
    fn ensure_estimate(
        &self,
        snapshot: &AnyViewsSnapshot<AnyView>,
        state: &mut HydroState,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        cross: Option<f32>,
    ) -> Size {
        if let Some((previous, size)) = self.estimate_sample.get()
            && previous == cross
            && !self.dirty.get()
        {
            return size;
        }
        let (size, _) = self.measure_item(snapshot, state, theme, 0, self.item_proposal(cross));
        let extent = self.main_extent(size);
        self.estimate.set(extent.max(1.0));
        self.estimate_sample.set(Some((cross, size)));
        self.floor_sample.set(None);
        self.dirty.set(true);
        self.prepare_extent_index(state, snapshot.len());
        self.extent_index.borrow_mut().set_measured(0, extent);
        size
    }

    fn prepare_extent_index(&self, state: &mut HydroState, count: usize) {
        let estimate = self.estimate.get();
        if count == 0 || estimate <= 0.0 {
            return;
        }
        let spacing = self.spacing(state);
        let dirty = self.dirty.replace(false);
        if dirty || !self.extent_index.borrow().matches(count, estimate, spacing) {
            self.extent_index
                .borrow_mut()
                .reset(count, estimate, spacing);
        }
    }

    /// Re-measures the last visible window after its retained children were
    /// patched. This makes a reactive row-height change part of the same parent
    /// layout pass instead of discovering it later during flush.
    fn refresh_visible_extents(
        &self,
        snapshot: &AnyViewsSnapshot<AnyView>,
        state: &mut HydroState,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        count: usize,
        cross: Option<f32>,
    ) {
        let visible = self.visible_range.borrow().clone();
        let cache = self.item_cache.borrow();
        for index in visible.start.min(count)..visible.end.min(count) {
            let id = snapshot
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
            let Some(item) = cache.get(&id) else {
                continue;
            };
            let proposal = self.item_proposal(cross);
            let size = item.measure_built_with_proposal(state, &self.env, theme, proposal);
            let extent = self.main_extent(size);
            self.extent_index.borrow_mut().set_measured(index, extent);
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    pub(super) fn measure(
        &self,
        state: &mut HydroState,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        // One snapshot per measure pass: the sampled item, the extent index,
        // and the visible-window refresh all read the same membership.
        let snapshot = self.snapshot.borrow().clone();
        let count = snapshot.len();
        if count == 0 {
            return ViewDimensions::new(Size::zero());
        }
        let (cross, offered) = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => (proposal.width, proposal.height),
            LazyStackAxisConfig::Horizontal { .. } => (proposal.height, proposal.width),
        };
        let sample = self.ensure_estimate(&snapshot, state, theme, cross);
        self.prepare_extent_index(state, count);
        self.refresh_visible_extents(&snapshot, state, theme, count, cross);
        // The extent index holds the items' intrinsic main extents, so the
        // unclamped total is the stack's ideal, not its minimum: a lazy
        // stack virtualizes — it fits a finite main-axis offer below its
        // extent by showing fewer items — while an open axis reads the full
        // extent. It never reports below the items' summed minima: content
        // that cannot shrink keeps its extent, as the eager stack's
        // `minima_overflow` answer does.
        let extent = self.extent_index.borrow().total_extent();
        let floor = self.spacing(state).mul_add(
            crate::num_cast::usize_as_f64(count - 1),
            self.ensure_floor(&snapshot, state, theme, cross)
                * crate::num_cast::usize_as_f64(count),
        );
        let main = match offered {
            Some(offer) if offer.is_finite() => extent.min(f64::from(offer)).max(floor),
            _ => extent,
        } as f32;
        let size = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => Size::new(sample.width, main),
            LazyStackAxisConfig::Horizontal { .. } => Size::new(main, sample.height),
        };
        ViewDimensions::new(size)
    }

    /// Resolves the visible window from the enclosing scroll's pushed viewport and
    /// re-dispatches only those items at their placed rects. Bounded by visible rows.
    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    pub(super) fn flush(
        &self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        _env: &Environment,
    ) {
        // One snapshot for the whole pass: the visible window's ids and
        // materialized views come from the same membership the watcher last
        // applied, coherent under a reentrant source mutation.
        let snapshot = self.snapshot.borrow().clone();
        let count = snapshot.len();
        if count == 0 {
            return;
        }
        #[cfg(feature = "accessibility")]
        let container_scope = self.accessibility_container_env.as_ref().map(|env| {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            // A lazy stack places its items at flush (offset-dependent), so no
            // resolved extent is cached for it — `None` keeps the surviving
            // child's own bounds on collapse.
            let bounds = ctx.bounds;
            let scope = renderer.begin_accessibility_container(bounds, None, env);
            renderer.pop_accessibility_owner();
            scope
        });
        let cross = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => {
                Some(crate::num_cast::f64_as_f32(ctx.bounds.width()))
            }
            LazyStackAxisConfig::Horizontal { .. } => {
                Some(crate::num_cast::f64_as_f32(ctx.bounds.height()))
            }
        };
        let theme = renderer.theme();
        self.ensure_estimate(&snapshot, &mut renderer.state, &theme, cross);
        self.prepare_extent_index(&mut renderer.state, count);
        let total_extent_before = self.extent_index.borrow().total_extent();
        self.item_cache.borrow_mut().begin_frame();
        let visible = renderer
            .lazy
            .lazy_viewport_stack
            .last()
            .map_or(ctx.bounds, |viewport| {
                (renderer.record_world(ctx.local).inverse() * viewport.transform)
                    .transform_rect_bbox(viewport.bounds)
            });
        let (visible_start, visible_end) = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => {
                (visible.y0 - ctx.bounds.y0, visible.y1 - ctx.bounds.y0)
            }
            LazyStackAxisConfig::Horizontal { .. }
                if self.axis.direction().snapshot().is_right_to_left() =>
            {
                (
                    ctx.bounds.x0 + ctx.bounds.x1 - visible.x1,
                    ctx.bounds.x0 + ctx.bounds.x1 - visible.x0,
                )
            }
            LazyStackAxisConfig::Horizontal { .. } => {
                (visible.x0 - ctx.bounds.x0, visible.x1 - ctx.bounds.x0)
            }
        };
        self.visible_span.set(Some((visible_start, visible_end)));
        let spacing = self.spacing(&mut renderer.state);
        let window = self
            .extent_index
            .borrow()
            .visible_window(visible_start, visible_end);
        *self.visible_range.borrow_mut() = window.start..window.end;
        let mut cursor = window.leading_offset;
        // The context the stack laid out against: one borrow for the whole
        // visible window — every item derives its hosted frame from it.
        let stack_area = self.safe_area.as_deref();
        for index in window.start..window.end {
            let id = snapshot
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
            let proposal = self.item_proposal(cross);
            let (size, stretch) = {
                let env = &self.env;
                let views = &snapshot;
                let mut cache = self.item_cache.borrow_mut();
                cache
                    .entry(id, || {
                        let view = views.get_view(index).unwrap_or_else(|| {
                            panic!("hydrolysis LazyStack failed to materialize item {index}")
                        });
                        normalize_layout_view(view, env)
                    })
                    .patch_and_measure(renderer, env, proposal)
            };
            let child_rect = place_lazy_stack_item(&self.axis, stretch, size, ctx.bounds, cursor);
            let extent = match &self.axis {
                LazyStackAxisConfig::Vertical { .. } => child_rect.height(),
                LazyStackAxisConfig::Horizontal { .. } => child_rect.width(),
            };
            self.extent_index.borrow_mut().set_measured(index, extent);
            {
                let env = &self.env;
                let views = &snapshot;
                let mut cache = self.item_cache.borrow_mut();
                let subview = cache.entry(id, || {
                    let view = views.get_view(index).unwrap_or_else(|| {
                        panic!("hydrolysis LazyStack failed to materialize item {index}")
                    });
                    normalize_layout_view(view, env)
                });
                // The item's frame in the context the stack recorded at
                // layout: `child_rect` resolves in `ctx.bounds` space — the
                // same layout-fact mapping `WidgetRenderContext` and the
                // collection layout loop share through
                // `SafeAreaLayout::hosted_frame`.
                let item_area = stack_area
                    .map(|area| area.with_frame(area.hosted_frame(ctx.bounds, child_rect)));
                subview.place(renderer, ctx, env, proposal, child_rect, item_area);
            }
            cursor += extent;
            if index + 1 < count {
                cursor += spacing;
            }
        }
        self.item_cache.borrow_mut().end_frame();
        #[cfg(feature = "accessibility")]
        if let Some(container_scope) = container_scope {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            renderer.end_accessibility_container(container_scope);
            renderer.pop_accessibility_owner();
        }
        // Materializing entering rows replaced their estimates with measured
        // extents. If that moved the container's total extent, the scroll
        // extents the last layout negotiated are stale: escalate to one layout
        // pass. Uniform rows never trigger this, so a steady scroll stays on
        // the re-encode path.
        let total_extent_after = self.extent_index.borrow().total_extent();
        if (total_extent_after - total_extent_before).abs() > 0.5 {
            renderer.context_mark_layout();
        }
    }

    /// Emits every item's accessibility nodes for the semantic walk. There is
    /// no viewport to bound emission against — the semantic tree contains the
    /// whole collection, so every item materializes its node rather than only
    /// the visible window.
    #[cfg(feature = "accessibility")]
    pub(super) fn emit_accessibility(&self, renderer: &mut SemanticCore) {
        let snapshot = self.snapshot.borrow().clone();
        let count = snapshot.len();
        if count == 0 {
            return;
        }
        let container_scope = self.accessibility_container_env.as_ref().map(|env| {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            let scope = renderer.begin_accessibility_container_semantic(env);
            renderer.pop_accessibility_owner();
            scope
        });
        self.item_cache.borrow_mut().begin_frame();
        for index in snapshot.range() {
            let id = snapshot
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
            let env = &self.env;
            let views = &snapshot;
            let mut cache = self.item_cache.borrow_mut();
            let subview = cache.entry(id, || {
                let view = views.get_view(index).unwrap_or_else(|| {
                    panic!("hydrolysis LazyStack failed to materialize item {index}")
                });
                normalize_layout_view(view, env)
            });
            subview.emit_accessibility(renderer, env);
        }
        self.item_cache.borrow_mut().end_frame();
        if let Some(container_scope) = container_scope {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            renderer.end_accessibility_container(container_scope);
            renderer.pop_accessibility_owner();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use waterui_core::layout::{HorizontalAlignment, VerticalAlignment};
    use waterui_core::transition::{AnyTransition, PropertyTransition};

    fn removal_spec(milliseconds: u64) -> TransitionSpec {
        TransitionSpec {
            transition: AnyTransition::new(PropertyTransition::opacity()),
            animation: Animation::linear(Duration::from_millis(milliseconds)),
        }
    }

    /// A fresh clock records its creation instant: sampling it then reports
    /// zero elapsed, while an older clock over the same spec completes —
    /// an idle stretch between collection frames cannot leak into a new
    /// clock's first delta.
    #[test]
    fn a_clock_born_after_idle_carries_no_elapsed_time() {
        let t0 = Instant::now();
        let spec = removal_spec(1_000);
        let mut old_state = spec.start(TransitionPhase::Removal, false);
        let mut old_clock = TransitionClock::new(t0);
        let mut fresh_state = spec.start(TransitionPhase::Removal, false);
        let mut fresh_clock = TransitionClock::new(t0 + Duration::from_secs(10));

        // The older clock over a 1s removal is done by t0+10s; the fresh
        // clock sampled at the same instant advanced nothing.
        assert!(!old_clock.advance(&mut old_state, t0 + Duration::from_secs(10)));
        assert!(fresh_clock.advance(&mut fresh_state, t0 + Duration::from_secs(10)));

        // +500ms on the fresh clock: half progress for a linear 1s removal.
        assert!(fresh_clock.advance(
            &mut fresh_state,
            t0 + Duration::from_secs(10) + Duration::from_millis(500)
        ));
        let progress = fresh_state.progress();
        assert!((progress - 0.5).abs() < 0.02, "{progress}");
    }

    /// A zero-duration state completes on its first advance regardless of the
    /// clock's delta.
    #[test]
    fn zero_duration_state_completes_on_first_advance() {
        let t0 = Instant::now();
        let spec = removal_spec(0);
        let mut state = spec.start(TransitionPhase::Removal, false);
        let mut clock = TransitionClock::new(t0);
        assert!(!clock.advance(&mut state, t0));
        assert!(state.is_complete());
    }

    /// A stub child whose measured size differs from its placed frame and
    /// which reports nondefault explicit guides on both axes.
    struct GuidedSubView;

    impl SubView for GuidedSubView {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            // Distinguishes measuring at the placed proposal from measuring
            // at the parent's container proposal.
            assert_eq!(
                proposal,
                ProposalSize::new(Some(50.0), Some(60.0)),
                "frozen_placement_dimensions must measure at the placed proposal"
            );
            ViewDimensions::new(Size::new(10.0, 20.0))
                .with_horizontal(HorizontalAlignment::Center, 7.5)
                .with_vertical(VerticalAlignment::Top, 3.5)
        }

        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }

        fn priority(&self) -> i32 {
            0
        }
    }

    /// `frozen_placement_dimensions` pins the placed frame size and preserves
    /// both explicit guides; through `GhostSlot::measure` later proposals
    /// cannot reflow the frozen slot.
    #[test]
    fn frozen_dimensions_hold_the_placed_size_and_guides() {
        let sub = GuidedSubView;
        let placement = SubviewPlacement::new(
            Rect::new(Point::new(0.0, 0.0), Size::new(50.0, 60.0)),
            ProposalSize::new(Some(50.0), Some(60.0)),
        );
        let frozen = frozen_placement_dimensions(&sub, &placement);
        assert_eq!(frozen.size, Size::new(50.0, 60.0));
        assert_eq!(
            frozen.explicit_horizontal(HorizontalAlignment::Center),
            Some(7.5)
        );
        assert_eq!(frozen.explicit_vertical(VerticalAlignment::Top), Some(3.5));

        let slot = GhostSlot::new(frozen.clone(), 0);
        let remeasured = slot.measure(ProposalSize::new(Some(500.0), Some(500.0)));
        assert_eq!(remeasured.size, frozen.size);
        assert_eq!(
            remeasured.explicit_horizontal(HorizontalAlignment::Center),
            Some(7.5)
        );
        assert_eq!(
            remeasured.explicit_vertical(VerticalAlignment::Top),
            Some(3.5)
        );
    }
}
