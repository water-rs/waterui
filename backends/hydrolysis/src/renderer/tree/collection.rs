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
use waterui_core::transition::{Ghost, GhostSlot, TransitionSpec};
use waterui_core::views::ViewSnapshot;

use super::transition::{
    TransitionPaint, TransitionRun, TransitionRuntime, claim_transition, flush_transitioned,
    start_ghost,
};

/// Collect into `out` the ids of the items a [`CollectionChange`] reports as
/// updated in place — same id, possibly different content: the updates name
/// new-snapshot indices, and `snapshot` is the row set delivered with the
/// notification. Insertions, removals and moves are membership changes the
/// id reconciliation applies; an empty change means nothing needs
/// re-materializing.
pub fn collect_replaced_ids<S>(
    snapshot: &S,
    change: &CollectionChange,
    out: &mut std::collections::HashSet<S::Id>,
) where
    S: ViewSnapshot,
    S::Id: std::hash::Hash,
{
    for range in change.updates() {
        out.extend(range.filter_map(|index| snapshot.get_id(index)));
    }
}

/// The dimensions a live entry freezes at for its removal ghost: measured at
/// the slot's actual placed proposal, carrying the slot's explicit guides,
/// with size pinned to the placed frame so later proposals cannot reflow it.
fn frozen_placement_dimensions(sub: &dyn SubView, placement: &SubviewPlacement) -> ViewDimensions {
    let mut dimensions = sub.measure(placement.proposal);
    dimensions.size = *placement.frame.size();
    dimensions
}

/// One retained entry of a [`CollectionNode`]: the item's persistent node, the
/// `.transition()` declaration claimed from its built root, and the
/// in-flight insertion.
pub(super) struct CollectionEntry<E: crate::text::TextEngine> {
    /// Stable item identity from the source collection.
    pub(super) id: CollectionItemId,
    /// The entry's own mount core: the layer its transition scope writes and
    /// the cell a transition marks, distinct from `node`'s so an entry
    /// transition re-places only the entry, not the subtree inside it.
    pub(crate) core: NodeCore<E>,
    /// The item's retained node, kept across membership changes by id.
    pub(super) node: RenderNode<E>,
    /// The `.transition(...)` the built item declares at its root;
    /// `None` means this item pops on membership changes.
    pub(super) spec: Option<TransitionSpec>,
    /// Independent insertion while the entry animates in; `None` at rest and
    /// forever when the item declared no spec.
    pub(super) insertion: Option<TransitionRun<E>>,
    /// The dimensions and layout priority the entry last measured at — the
    /// frozen geometry its removal ghost's [`GhostSlot`] retains.
    last_measured: std::cell::RefCell<(ViewDimensions, i32)>,
}

impl<E: crate::text::TextEngine> CollectionEntry<E> {
    /// An at-rest entry over a built item node: the item's declaration is
    /// claimed from the node's root, so a row whose declaration sits behind
    /// type erasure is owned the same way; one beneath a modifier is not. The
    /// initial membership of a collection is built from these (no insertion
    /// runs); later additions get their clock from [`Self::begin_insertion`].
    pub(super) fn new(id: CollectionItemId, node: RenderNode<E>, core: NodeCore<E>) -> Self {
        let spec = claim_transition(&node);
        Self {
            id,
            core,
            node,
            spec,
            insertion: None,
            last_measured: std::cell::RefCell::new((ViewDimensions::default(), 0)),
        }
    }

    /// Replaces the entry's node with a rebuilt one for the same id, claiming
    /// the rebuilt item's declaration.
    fn replace_node(&mut self, node: RenderNode<E>) {
        node.core().cell.set_parent(&self.core.cell);
        self.spec = claim_transition(&node);
        self.node = node;
    }

    /// Starts this entry's insertion clock from its spec, if it has one —
    /// timestamped at reconciliation's frame instant, not the previous frame.
    fn begin_insertion(&mut self, reduce_motion: bool, now: Instant) {
        self.insertion = self
            .spec
            .as_ref()
            .map(|spec| TransitionRun::insertion(spec, reduce_motion, now));
    }
}

pub struct CollectionNode<E: crate::text::TextEngine> {
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
    pub(crate) core: NodeCore<E>,
    /// The unshielded environment when this collection carries accessibility
    /// naming metadata: `Some` means it emits the node naming itself.
    #[cfg(feature = "accessibility")]
    pub(super) accessibility_container_env: Option<Environment>,
    /// Current entries in display order, keyed by id so a membership change
    /// keeps unchanged items' nodes (and their in-flight state) and only
    /// builds/drops the delta. With a transition this also holds still-exiting
    /// entries — anchored after the live id they followed — until their
    /// fade-out completes.
    pub(super) entries: Vec<CollectionEntry<E>>,
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
    pub(super) ghosts: Vec<CollectionGhost<E>>,
    /// Ghost slot frames in `ghosts` order, filled by `layout` beside `placed`.
    pub(super) ghost_placed: Vec<Rect>,
    /// Reduce Motion resolved once at build from the item environment: every
    /// clock started here maps through `Transition::reduced` when set.
    pub(super) reduce_motion: bool,
    /// Logical-edge resolution for sampled transition properties.
    pub(super) direction: waterui_core::layout::LayoutDirection,
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

pub struct LazyStackNode<E: crate::text::TextEngine> {
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
    pub(crate) core: NodeCore<E>,
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
    pub(super) item_cache: RefCell<VisibleSubviewCache<CollectionItemId, E>>,
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
    /// The structural transitions of the realized rows.
    pub(super) transitions: LazyTransitions<E>,
}

/// A detached outgoing item: its retained node (semantically dead, painted
/// only), the fixed-size [`GhostSlot`] it still occupies, and the runtime of
/// the independent removal clock driving it (layout-spec.md §4.5).
pub(super) struct CollectionGhost<E: crate::text::TextEngine> {
    /// Live id this ghost displays after; `None` places it before the first
    /// live entry.
    anchor: Option<CollectionItemId>,
    ghost: Ghost<RenderNode<E>>,
    /// This ghost's own clock and paint scope — removal runs on its own
    /// timeline, never on the collection's shared tick.
    runtime: TransitionRuntime<E>,
}

/// One layout/flush position in display order: a live entry, or a removal
/// ghost the collection retains in the slot its row held.
#[derive(Clone, Copy)]
enum CollectionSlot {
    Entry(usize),
    Ghost(usize),
}

/// The members of `order` as the layout sees them, `subs` being the entries'
/// sub-views by entry index.
fn slot_refs<'a, S: SubView, E: crate::text::TextEngine>(
    order: &[CollectionSlot],
    subs: &'a [S],
    ghosts: &'a [CollectionGhost<E>],
) -> Vec<&'a dyn SubView> {
    order
        .iter()
        .map(|slot| match *slot {
            CollectionSlot::Entry(index) => &subs[index] as &dyn SubView,
            CollectionSlot::Ghost(index) => &ghosts[index].ghost.slot as &dyn SubView,
        })
        .collect()
}

impl<E: crate::text::TextEngine> CollectionNode<E> {
    /// Display order for measure, layout and flush: head-anchored ghosts first,
    /// then each live entry followed by the ghosts anchored to it. Ghosts
    /// whose anchor left in the same reconcile trail at the end. A
    /// conditional at an entry's root keeps its own ghosts inside the entry
    /// (layout-spec.md §4.5), so the entry is one member either way.
    fn slot_order(&self) -> Vec<CollectionSlot> {
        let mut order = Vec::with_capacity(self.entries.len() + self.ghosts.len());
        let mut anchored = vec![false; self.ghosts.len()];
        for (index, ghost) in self.ghosts.iter().enumerate() {
            if ghost.anchor.is_none() {
                order.push(CollectionSlot::Ghost(index));
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
        state: &mut HydroState<E>,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let cell = RefCell::new(state);
        let subs: Vec<NodeSubView<E>> = self
            .entries
            .iter()
            .map(|entry| NodeSubView::new(&entry.node, &cell, &self.env, theme))
            .collect();
        // Every member — live entries and still-held ghost slots — is a normal
        // layout member: a ghost keeps the fixed slot it had at removal until
        // its clock completes, and an insertion allocates its slot immediately.
        let order = self.slot_order();
        let refs = slot_refs(&order, &subs, &self.ghosts);
        ViewDimensions::new(self.layout.size_that_fits(proposal, &refs))
    }

    pub(super) fn layout(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        safe_area: Option<&safe_area::SafeAreaLayout>,
        proposal: ProposalSize,
        size: Size,
    ) {
        let env = self.env.clone();
        let theme = renderer.theme();
        let order = self.slot_order();
        let (placements, mut frozen, priorities) = {
            let cell = RefCell::new(&mut renderer.state);
            let subs: Vec<NodeSubView<E>> = self
                .entries
                .iter()
                .map(|entry| NodeSubView::new(&entry.node, &cell, &env, &theme))
                .collect();
            let priorities: Vec<i32> = subs.iter().map(SubView::priority).collect();
            // Ghost slots stay ordinary layout members at their frozen size:
            // the container's membership only reflows when a ghost's clock
            // completes and its slot is released.
            let refs = slot_refs(&order, &subs, &self.ghosts);
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

    pub(super) fn flush(&self, renderer: &mut HydrolysisRenderer<E>, ctx: RenderContext) {
        #[cfg(feature = "accessibility")]
        let container_scope = self.accessibility_container_env.as_ref().map(|env| {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            let (bounds, extent) = (
                ctx.bounds(),
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
                    let (child_ctx, delta) = placed_context(ctx, rect);
                    if let Some(insertion) = &entry.insertion {
                        let paint = TransitionPaint::sample(
                            &insertion.state,
                            *rect.size(),
                            self.direction,
                            true,
                            &insertion.runtime,
                            &entry.core.cell,
                        );
                        flush_transitioned(renderer, child_ctx, delta, &paint, |renderer, ctx| {
                            entry
                                .node
                                .flush(renderer, ctx, &self.env, kurbo::Affine::IDENTITY);
                        });
                    } else {
                        entry.node.flush(renderer, child_ctx, &self.env, delta);
                    }
                }
                CollectionSlot::Ghost(index) => {
                    let (ghost, rect) = (&self.ghosts[index], self.ghost_placed[index]);
                    let (child_ctx, delta) = placed_context(ctx, rect);
                    let paint = TransitionPaint::sample(
                        &ghost.ghost.transition,
                        *rect.size(),
                        self.direction,
                        false,
                        &ghost.runtime,
                        &self.core.cell,
                    );
                    // The ghost is paint-only: no input, no accessibility.
                    renderer.with_suppressed_accessibility(|renderer| {
                        flush_transitioned(renderer, child_ctx, delta, &paint, |renderer, ctx| {
                            ghost.ghost.realization.flush(
                                renderer,
                                ctx,
                                &self.env,
                                kurbo::Affine::IDENTITY,
                            );
                        });
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
    pub(super) fn emit_accessibility(&self, renderer: &mut SemanticCore<E>) {
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

    /// Apply a membership change: keep each surviving id's node (and its
    /// in-flight insertion clock), build newly-present ids — starting their
    /// own insertion clock when the built item declares a spec — and detach
    /// departed ones into [`CollectionGhost`]s anchored after the live id they
    /// followed. Entries the watcher flagged as replaced keep their id but
    /// their node (and spec) is re-materialized — a same-id content change
    /// rebuilds exactly that row. Re-insertion of a departed id is a new
    /// instance with a new slot; a same-id ghost keeps animating beside it
    /// and is never revived.
    pub(super) fn reconcile(&mut self, renderer: &mut SemanticCore<E>) {
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
        let mut reuse_by_id: std::collections::BTreeMap<CollectionItemId, CollectionEntry<E>> =
            std::collections::BTreeMap::new();
        let mut new_ghosts: Vec<CollectionGhost<E>> = Vec::new();
        {
            let mut last_live: Option<CollectionItemId> = None;
            for entry in self.entries.drain(..) {
                if live.contains(&entry.id) {
                    last_live = Some(entry.id);
                    reuse_by_id.insert(entry.id, entry);
                } else if let Some(spec) = entry.spec {
                    let (dimensions, priority) = entry.last_measured.borrow().clone();
                    new_ghosts.push(CollectionGhost {
                        anchor: last_live,
                        ghost: start_ghost(
                            entry.node,
                            GhostSlot::new(dimensions, priority),
                            &spec,
                            self.reduce_motion,
                        ),
                        runtime: TransitionRuntime::new(renderer.frame_instant),
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
                    previous.replace_node(RenderNode::build(
                        normalize_layout_view(view, &env),
                        &env,
                        renderer,
                    ));
                }
                previous
            } else {
                let view = snapshot
                    .get_view(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} missing"));
                let core = renderer.new_core();
                core.cell.set_parent(&self.core.cell);
                let node = RenderNode::build(normalize_layout_view(view, &env), &env, renderer);
                node.core().cell.set_parent(&core.cell);
                let mut entry = CollectionEntry::new(id, node, core);
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
    pub(super) fn advance_transitions(&mut self, renderer: &SemanticCore<E>) -> bool {
        let now = renderer.frame_instant;
        let mut animating = false;
        for entry in &mut self.entries {
            if let Some(insertion) = &mut entry.insertion {
                if insertion.advance(now) {
                    animating = true;
                } else {
                    entry.insertion = None;
                }
            }
        }
        let before = self.ghosts.len();
        self.ghosts
            .retain_mut(|ghost| ghost.runtime.advance(&mut ghost.ghost.transition, now));
        let dropped = self.ghosts.len() != before;
        animating |= !self.ghosts.is_empty();
        if animating {
            renderer.context_mark_layout();
        }
        dropped || animating
    }
}

/// The structural transitions of a virtualized row container's rows — a
/// [`LazyStackNode`] or a list (layout-spec.md §4.5). Such a container
/// realizes only its visible rows, so it owns transitions for exactly those: a row's declaration is claimed when the row is built,
/// an id the last membership change inserted starts its insertion when it is
/// first built in that frame, and a removed row that was on screen becomes a
/// ghost holding its main-axis slot in the stack's flow until its removal
/// completes. A removed row that was never realized has no pixels to animate
/// and leaves at once.
pub struct LazyTransitions<E: crate::text::TextEngine, T: View = AnyView> {
    /// Set by the membership watcher; consumed when the container applies
    /// the change.
    pub(crate) membership: Rc<Cell<bool>>,
    /// The row set the container showed before the last membership change.
    previous: RefCell<AnyViewsSnapshot<T>>,
    /// The row set the last membership change replaced, for the frame that
    /// applied it: a row built in that frame whose id it lacks was inserted.
    /// Its id set is derived only when such a row declares a transition.
    replaced_set: RefCell<Option<ReplacedSet<T>>>,
    /// The transition state of every realized row, by id.
    rows: RefCell<std::collections::HashMap<CollectionItemId, LazyRow<E>>>,
    /// Removed rows still animating out, ordered by the row position they
    /// sit before.
    ghosts: RefCell<Vec<LazyGhost<E>>>,
    /// Reduce Motion resolved once at build from the item environment.
    reduce_motion: bool,
    /// Logical-edge resolution for sampled transition properties.
    pub(crate) direction: waterui_core::layout::LayoutDirection,
}

/// The row set a membership change replaced, the frame that applied it and,
/// once a transitioned row asked, the ids it held.
struct ReplacedSet<T: View> {
    snapshot: AnyViewsSnapshot<T>,
    at: Instant,
    ids: Option<std::collections::HashSet<CollectionItemId>>,
}

/// Where a realized row was last placed: the slot it holds in the
/// container's flow, and its content's frame inside that slot. The content
/// is what the row's transition animates; the slot is what its removal
/// ghost keeps.
#[derive(Clone, Copy)]
pub struct RowPlacement {
    /// The row's slot, container-local.
    pub slot: kurbo::Rect,
    /// The content's frame, relative to the slot's origin.
    pub content: kurbo::Rect,
    /// The proposal the content was placed at.
    pub proposal: ProposalSize,
    /// The stretch the row measured with, kept so its ghost is re-placed
    /// with the same cross-axis behavior.
    pub stretch: StretchAxis,
}

/// The transition state of one realized row.
pub struct LazyRow<E: crate::text::TextEngine> {
    /// The declaration claimed from the built row.
    spec: Option<TransitionSpec>,
    /// The row's insertion while it animates in.
    pub insertion: Option<TransitionRun<E>>,
    /// Where the row was last placed — the fixed slot its removal ghost
    /// keeps.
    placed: Option<RowPlacement>,
}

/// A removed row still animating out: its retained sub-view, frozen at the
/// geometry it was last placed at, sitting before row `position` of the
/// current row set.
pub struct LazyGhost<E: crate::text::TextEngine> {
    /// The live id the ghost displays after; `None` places it first.
    anchor: Option<CollectionItemId>,
    /// The row position (current row set) the ghost sits before.
    position: usize,
    pub ghost: Ghost<RetainedSubview<E>>,
    /// The frozen slot and content frame.
    pub placement: RowPlacement,
    pub runtime: TransitionRuntime<E>,
}

impl<E: crate::text::TextEngine> LazyGhost<E> {
    /// The frozen size of the ghost's slot.
    pub const fn size(&self) -> Size {
        kurbo_size(self.placement.slot)
    }
}

/// One position of a virtualized container's display order: a row of the
/// row set, or a removal ghost.
#[derive(Clone, Copy)]
pub enum LazySlot {
    Row(usize),
    Ghost(usize),
}

impl<T: View, E: crate::text::TextEngine> LazyTransitions<E, T> {
    pub(crate) fn new(
        env: &Environment,
        snapshot: AnyViewsSnapshot<T>,
        membership: Rc<Cell<bool>>,
    ) -> Self {
        Self {
            membership,
            previous: RefCell::new(snapshot),
            replaced_set: RefCell::new(None),
            rows: RefCell::new(std::collections::HashMap::new()),
            ghosts: RefCell::new(Vec::new()),
            reduce_motion: env
                .get::<waterui_core::transition::ReduceMotion>()
                .is_some_and(|flag| flag.0),
            direction: waterui_layout::layout_direction(env).snapshot(),
        }
    }

    /// Positions in display order: the rows plus the ghosts.
    pub(crate) fn display_len(&self, rows: usize) -> usize {
        rows + self.ghosts.borrow().len()
    }

    /// The display position of row `row`.
    pub(crate) fn row_display(&self, row: usize) -> usize {
        row + self
            .ghosts
            .borrow()
            .iter()
            .filter(|ghost| ghost.position <= row)
            .count()
    }

    /// What sits at display position `display`. Ghost `g` (ordered by
    /// position) sits at `position + g`.
    pub(crate) fn slot_at(&self, display: usize) -> LazySlot {
        let mut before = 0;
        for (index, ghost) in self.ghosts.borrow().iter().enumerate() {
            let at = ghost.position + index;
            if at == display {
                return LazySlot::Ghost(index);
            }
            if at > display {
                break;
            }
            before += 1;
        }
        LazySlot::Row(display - before)
    }

    /// Resets `index` over the display order and seeds every ghost's frozen
    /// main-axis extent, read from its slot by `main`.
    pub(crate) fn reset_extents(
        &self,
        index: &mut VirtualExtentIndex,
        rows: usize,
        estimate: f64,
        spacing: f64,
        main: impl Fn(Size) -> f64,
    ) {
        index.reset(self.display_len(rows), estimate, spacing);
        for (position, ghost) in self.ghosts.borrow().iter().enumerate() {
            index.set_measured(ghost.position + position, main(ghost.size()));
        }
    }

    /// The row display position `display` shows — or, for a removal ghost,
    /// the row the ghost sits before.
    pub(crate) fn row_at(&self, display: usize) -> usize {
        match self.slot_at(display) {
            LazySlot::Row(row) => row,
            LazySlot::Ghost(ghost) => self.ghosts.borrow()[ghost].position,
        }
    }

    /// The removal ghost at `index`, in display order.
    pub(crate) fn ghost(&self, index: usize) -> std::cell::RefMut<'_, LazyGhost<E>> {
        std::cell::RefMut::map(self.ghosts.borrow_mut(), |ghosts| &mut ghosts[index])
    }

    /// The tracked transition state of realized row `id`.
    pub(crate) fn row(&self, id: CollectionItemId) -> std::cell::RefMut<'_, LazyRow<E>> {
        std::cell::RefMut::map(self.rows.borrow_mut(), |rows| {
            rows.get_mut(&id)
                .expect("hydrolysis virtualized row: a placed row is tracked")
        })
    }

    /// Records where realized row `id` was placed this frame.
    pub(crate) fn place(&self, id: CollectionItemId, placement: RowPlacement) {
        self.row(id).placed = Some(placement);
    }

    /// Drops the transition state of every row `keep` rejects — the rows a
    /// frame evicted.
    pub(crate) fn retain_rows(&self, keep: impl Fn(&CollectionItemId) -> bool) {
        self.rows.borrow_mut().retain(|id, _| keep(id));
    }

    /// The summed main-axis extents of the ghosts and their largest
    /// cross-axis extent.
    fn ghost_extents(&self, axis: &LazyStackAxisConfig) -> (f64, f32) {
        self.ghosts
            .borrow()
            .iter()
            .fold((0.0, 0.0_f32), |(main, cross), ghost| {
                let size = ghost.size();
                let ghost_cross = match axis {
                    LazyStackAxisConfig::Vertical { .. } => size.width,
                    LazyStackAxisConfig::Horizontal { .. } => size.height,
                };
                (main + lazy_main_extent(axis, size), cross.max(ghost_cross))
            })
    }

    /// Tracks a row the stack just realized: claims its declaration from the
    /// built node and, when the last membership change inserted its id,
    /// starts its insertion. A row first realized at flush starts after the
    /// frame's patch walk advanced the clocks, so a started insertion
    /// requests the refresh that drives its next frame.
    pub(crate) fn track(
        &self,
        id: CollectionItemId,
        subview: &RetainedSubview<E>,
        renderer: &SemanticCore<E>,
    ) {
        if self.rows.borrow().contains_key(&id) {
            return;
        }
        let node = subview
            .node()
            .expect("hydrolysis LazyStack: a tracked row is built");
        let spec = claim_transition(node);
        let insertion = spec
            .as_ref()
            .filter(|_| self.was_inserted(id, renderer.frame_instant))
            .map(|spec| TransitionRun::insertion(spec, self.reduce_motion, renderer.frame_instant));
        if insertion.is_some() {
            renderer.context_mark_layout();
        }
        self.rows.borrow_mut().insert(
            id,
            LazyRow {
                spec,
                insertion,
                placed: None,
            },
        );
    }

    /// Whether the membership change applied in the frame at `now` inserted
    /// `id`.
    fn was_inserted(&self, id: CollectionItemId, now: Instant) -> bool {
        let mut replaced = self.replaced_set.borrow_mut();
        let Some(replaced) = replaced.as_mut().filter(|replaced| replaced.at == now) else {
            return false;
        };
        let snapshot = &replaced.snapshot;
        !replaced
            .ids
            .get_or_insert_with(|| {
                snapshot
                    .range()
                    .filter_map(|index| snapshot.get_id(index))
                    .collect()
            })
            .contains(&id)
    }

    /// Applies a membership change: every realized row that left becomes a
    /// ghost under its declaration, holding the slot it was last placed at,
    /// and every ghost re-anchors in the new row set. Returns whether a ghost
    /// was added.
    pub(crate) fn apply_membership(
        &self,
        current: &AnyViewsSnapshot<T>,
        cache: &mut VisibleSubviewCache<CollectionItemId, E>,
        now: Instant,
    ) -> bool {
        let previous = self.previous.replace(current.clone());
        let transitioned = self.rows.borrow().values().any(|row| row.spec.is_some());
        if !transitioned && self.ghosts.borrow().is_empty() {
            *self.replaced_set.borrow_mut() = Some(ReplacedSet {
                snapshot: previous,
                at: now,
                ids: None,
            });
            return false;
        }
        let index_of = |snapshot: &AnyViewsSnapshot<T>| {
            snapshot
                .range()
                .filter_map(|index| snapshot.get_id(index).map(|id| (id, index)))
                .collect::<std::collections::HashMap<_, _>>()
        };
        let live = index_of(current);
        let old = index_of(&previous);
        // The nearest live id at or before old position `index`.
        let anchor_before = |index: Option<usize>| -> Option<CollectionItemId> {
            let mut index = index?;
            loop {
                let id = previous
                    .get_id(index)
                    .expect("hydrolysis LazyStack: a previous row has an id");
                if live.contains_key(&id) {
                    return Some(id);
                }
                index = index.checked_sub(1)?;
            }
        };
        let mut ghosts = self.ghosts.borrow_mut();
        for ghost in ghosts.iter_mut() {
            if let Some(anchor) = ghost.anchor
                && !live.contains_key(&anchor)
            {
                ghost.anchor = anchor_before(old.get(&anchor).copied());
            }
        }
        let mut rows = self.rows.borrow_mut();
        let removed: Vec<CollectionItemId> = rows
            .keys()
            .copied()
            .filter(|id| !live.contains_key(id))
            .collect();
        let mut added = false;
        for id in removed {
            let row = rows
                .remove(&id)
                .expect("hydrolysis LazyStack: a removed row is tracked");
            let subview = cache.take(&id);
            let (Some(spec), Some(placement), Some(subview)) = (row.spec, row.placed, subview)
            else {
                continue;
            };
            ghosts.push(LazyGhost {
                anchor: anchor_before(old.get(&id).and_then(|index| index.checked_sub(1))),
                position: 0,
                ghost: start_ghost(
                    subview,
                    GhostSlot::new(ViewDimensions::new(kurbo_size(placement.content)), 0),
                    &spec,
                    self.reduce_motion,
                ),
                placement,
                runtime: TransitionRuntime::new(now),
            });
            added = true;
        }
        for ghost in ghosts.iter_mut() {
            ghost.position = ghost.anchor.map_or(0, |anchor| live[&anchor] + 1);
        }
        ghosts.sort_by_key(|ghost| ghost.position);
        *self.replaced_set.borrow_mut() = Some(ReplacedSet {
            snapshot: previous,
            at: now,
            ids: Some(old.into_keys().collect()),
        });
        added
    }

    /// Advances every insertion and ghost on its own clock; a finished ghost
    /// releases its slot. Returns `(dropped, animating)`.
    pub(crate) fn advance(&self, now: Instant) -> (bool, bool) {
        let mut animating = false;
        for row in self.rows.borrow_mut().values_mut() {
            if let Some(insertion) = &mut row.insertion {
                if insertion.advance(now) {
                    animating = true;
                } else {
                    row.insertion = None;
                }
            }
        }
        let mut ghosts = self.ghosts.borrow_mut();
        let before = ghosts.len();
        ghosts.retain_mut(|ghost| ghost.runtime.advance(&mut ghost.ghost.transition, now));
        (ghosts.len() != before, animating || !ghosts.is_empty())
    }
}

/// The context of a member placed at `rect` inside `ctx`, and the placement
/// delta that puts it there.
fn placed_context(ctx: RenderContext, rect: Rect) -> (RenderContext, kurbo::Affine) {
    let delta = kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y())));
    (
        ctx.child(
            delta,
            kurbo::Rect::new(0.0, 0.0, f64::from(rect.width()), f64::from(rect.height())),
        ),
        delta,
    )
}

/// The logical size of a placed rect.
const fn kurbo_size(rect: kurbo::Rect) -> Size {
    Size::new(
        crate::num_cast::f64_as_f32(rect.width()),
        crate::num_cast::f64_as_f32(rect.height()),
    )
}

/// The main-axis extent of `size` along the stack's axis.
fn lazy_main_extent(axis: &LazyStackAxisConfig, size: Size) -> f64 {
    match axis {
        LazyStackAxisConfig::Vertical { .. } => f64::from(size.height),
        LazyStackAxisConfig::Horizontal { .. } => f64::from(size.width),
    }
}

impl<E: crate::text::TextEngine> LazyStackNode<E> {
    /// Applies structural updates owned by the currently visible retained items
    /// before the parent scroll view measures this stack. Items whose ids the
    /// stored viewport window covers but that were never materialized — e.g.
    /// slid into view by the same membership change being patched — are built
    /// here too: their builders run inside the patch phase, so a `set()` they
    /// perform lands its `Dynamic` pending while the walk can still reach the
    /// host's patch arm, instead of mid-encode inside `flush`.
    ///
    /// A membership change is also the rows' structural transition event:
    /// realized rows that left become ghosts holding their slot, and rows it
    /// inserted start their insertion when they are first built this frame.
    /// Every row and ghost clock then advances.
    pub(super) fn patch_visible(&self, renderer: &mut SemanticCore<E>) -> bool {
        let transitions = &self.transitions;
        let replaced = core::mem::take(&mut *self.replaced_ids.borrow_mut());
        if !replaced.is_empty() {
            // A same-id content change drops exactly those rows' retained
            // sub-views; they re-materialize from the collection's current
            // data when the visible window next fills them.
            self.item_cache.borrow_mut().invalidate_ids(&replaced);
            transitions.retain_rows(|id| !replaced.contains(id));
            renderer.state.counters.structural_patches += 1;
        }
        let snapshot = self.snapshot.borrow().clone();
        let ghosted = transitions.membership.replace(false)
            && transitions.apply_membership(
                &snapshot,
                &mut self.item_cache.borrow_mut(),
                renderer.frame_instant,
            );
        let mut materialized = false;
        let count = snapshot.len();
        if transitions.display_len(count) > 0
            && let Some((visible_start, visible_end)) = self.visible_span.get()
        {
            let estimate = self.estimate.get();
            let spacing = self.spacing(&mut renderer.state);
            {
                let mut extent_index = self.extent_index.borrow_mut();
                if estimate > 0.0
                    && !extent_index.matches(transitions.display_len(count), estimate, spacing)
                {
                    transitions.reset_extents(
                        &mut extent_index,
                        count,
                        estimate,
                        spacing,
                        |size| lazy_main_extent(&self.axis, size),
                    );
                }
            }
            let window = self
                .extent_index
                .borrow()
                .visible_window(visible_start, visible_end);
            let views = &snapshot;
            let env = &self.env;
            let mut cache = self.item_cache.borrow_mut();
            for display in window.start..window.end {
                let LazySlot::Row(index) = transitions.slot_at(display) else {
                    continue;
                };
                if index >= count {
                    break;
                }
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
                    let subview = cache.materialize(id, || normalize_layout_view(view, env));
                    subview.ensure_built(renderer, env);
                    transitions.track(id, subview, renderer);
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
        let (dropped, animating) = transitions.advance(renderer.frame_instant);
        if animating {
            renderer.context_mark_layout();
        }
        changed | materialized | ghosted | dropped | animating
    }

    fn spacing(&self, state: &mut HydroState<E>) -> f64 {
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
        state: &mut HydroState<E>,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
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
        state: &mut HydroState<E>,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
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
        state: &mut HydroState<E>,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
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
        self.extent_index
            .borrow_mut()
            .set_measured(self.transitions.row_display(0), extent);
        size
    }

    /// Resets the extent index when the display order (rows plus ghosts),
    /// the estimate or the spacing changed — or the membership did.
    fn prepare_extent_index(&self, state: &mut HydroState<E>, count: usize) {
        let estimate = self.estimate.get();
        let display = self.transitions.display_len(count);
        if display == 0 || estimate <= 0.0 {
            return;
        }
        let spacing = self.spacing(state);
        let dirty = self.dirty.replace(false);
        if dirty
            || !self
                .extent_index
                .borrow()
                .matches(display, estimate, spacing)
        {
            self.transitions.reset_extents(
                &mut self.extent_index.borrow_mut(),
                count,
                estimate,
                spacing,
                |size| lazy_main_extent(&self.axis, size),
            );
        }
    }

    /// Re-measures the last visible window after its retained children were
    /// patched. This makes a reactive row-height change part of the same parent
    /// layout pass instead of discovering it later during flush.
    fn refresh_visible_extents(
        &self,
        snapshot: &AnyViewsSnapshot<AnyView>,
        state: &mut HydroState<E>,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
        count: usize,
        cross: Option<f32>,
    ) {
        let visible = self.visible_range.borrow().clone();
        let display = self.transitions.display_len(count);
        let cache = self.item_cache.borrow();
        for position in visible.start.min(display)..visible.end.min(display) {
            let LazySlot::Row(index) = self.transitions.slot_at(position) else {
                continue;
            };
            let id = snapshot
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
            let Some(item) = cache.get(&id) else {
                continue;
            };
            let proposal = self.item_proposal(cross);
            let size = item.measure_built_with_proposal(state, &self.env, theme, proposal);
            let extent = self.main_extent(size);
            self.extent_index
                .borrow_mut()
                .set_measured(position, extent);
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    pub(super) fn measure(
        &self,
        state: &mut HydroState<E>,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        // One snapshot per measure pass: the sampled item, the extent index,
        // and the visible-window refresh all read the same membership.
        let snapshot = self.snapshot.borrow().clone();
        let count = snapshot.len();
        let display = self.transitions.display_len(count);
        if display == 0 {
            return ViewDimensions::new(Size::zero());
        }
        let (cross, offered) = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => (proposal.width, proposal.height),
            LazyStackAxisConfig::Horizontal { .. } => (proposal.height, proposal.width),
        };
        // Removal ghosts hold their frozen slots in the flow until they
        // complete: their extents count toward the stack's extent and floor,
        // and their cross extent toward its cross size.
        let (ghost_main, ghost_cross) = self.transitions.ghost_extents(&self.axis);
        let (sample, row_floor) = if count > 0 {
            let sample = self.ensure_estimate(&snapshot, state, theme, cross);
            let floor = self.ensure_floor(&snapshot, state, theme, cross);
            (sample, floor * crate::num_cast::usize_as_f64(count))
        } else {
            (Size::zero(), 0.0)
        };
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
            crate::num_cast::usize_as_f64(display - 1),
            row_floor + ghost_main,
        );
        let main = match offered {
            Some(offer) if offer.is_finite() => extent.min(f64::from(offer)).max(floor),
            _ => extent,
        } as f32;
        let size = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => Size::new(sample.width.max(ghost_cross), main),
            LazyStackAxisConfig::Horizontal { .. } => {
                Size::new(main, sample.height.max(ghost_cross))
            }
        };
        ViewDimensions::new(size)
    }

    /// Resolves the visible window from the enclosing scroll's pushed viewport and
    /// re-dispatches only those items at their placed rects. Bounded by visible rows.
    pub(super) fn flush(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        _env: &Environment,
    ) {
        // One snapshot for the whole pass: the visible window's ids and
        // materialized views come from the same membership the watcher last
        // applied, coherent under a reentrant source mutation.
        let snapshot = self.snapshot.borrow().clone();
        let count = snapshot.len();
        let display = self.transitions.display_len(count);
        if display == 0 {
            return;
        }
        #[cfg(feature = "accessibility")]
        let container_scope = self.accessibility_container_env.as_ref().map(|env| {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            // A lazy stack places its items at flush (offset-dependent), so no
            // resolved extent is cached for it — `None` keeps the surviving
            // child's own bounds on collapse.
            let bounds = ctx.bounds();
            let scope = renderer.begin_accessibility_container(bounds, None, env);
            renderer.pop_accessibility_owner();
            scope
        });
        let cross = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => {
                Some(crate::num_cast::f64_as_f32(ctx.bounds().width()))
            }
            LazyStackAxisConfig::Horizontal { .. } => {
                Some(crate::num_cast::f64_as_f32(ctx.bounds().height()))
            }
        };
        let theme = renderer.theme();
        if count > 0 {
            self.ensure_estimate(&snapshot, &mut renderer.state, &theme, cross);
        }
        self.prepare_extent_index(&mut renderer.state, count);
        let total_extent_before = self.extent_index.borrow().total_extent();
        self.item_cache.borrow_mut().begin_frame();
        let visible = renderer.lazy.lazy_viewport_stack.last().map_or_else(
            || ctx.bounds(),
            |viewport| {
                (renderer.record_world(ctx.local()).inverse() * viewport.transform)
                    .transform_rect_bbox(viewport.bounds)
            },
        );
        let (visible_start, visible_end) = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => {
                (visible.y0 - ctx.bounds().y0, visible.y1 - ctx.bounds().y0)
            }
            LazyStackAxisConfig::Horizontal { .. }
                if self.axis.direction().snapshot().is_right_to_left() =>
            {
                (
                    ctx.bounds().x0 + ctx.bounds().x1 - visible.x1,
                    ctx.bounds().x0 + ctx.bounds().x1 - visible.x0,
                )
            }
            LazyStackAxisConfig::Horizontal { .. } => {
                (visible.x0 - ctx.bounds().x0, visible.x1 - ctx.bounds().x0)
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
        for position in window.start..window.end {
            let extent = match self.transitions.slot_at(position) {
                LazySlot::Row(index) => {
                    self.flush_row(renderer, ctx, &snapshot, index, cross, cursor, stack_area)
                }
                LazySlot::Ghost(index) => {
                    self.flush_ghost(renderer, ctx, index, cursor, stack_area)
                }
            };
            self.extent_index
                .borrow_mut()
                .set_measured(position, extent);
            cursor += extent;
            if position + 1 < display {
                cursor += spacing;
            }
        }
        self.end_frame();
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

    /// Evicts the rows the frame did not reach, with their transition state.
    fn end_frame(&self) {
        let mut cache = self.item_cache.borrow_mut();
        cache.end_frame();
        self.transitions.retain_rows(|id| cache.get(id).is_some());
    }

    /// Patches, measures and places row `index` at `cursor`, under its
    /// insertion while one runs. Returns the row's main-axis extent.
    #[expect(
        clippy::too_many_arguments,
        reason = "the row's flush context, position and hosting context are independent inputs"
    )]
    fn flush_row(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        snapshot: &AnyViewsSnapshot<AnyView>,
        index: usize,
        cross: Option<f32>,
        cursor: f64,
        stack_area: Option<&safe_area::SafeAreaLayout>,
    ) -> f64 {
        let id = snapshot
            .get_id(index)
            .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
        let proposal = self.item_proposal(cross);
        let env = &self.env;
        let materialize = || {
            let view = snapshot.get_view(index).unwrap_or_else(|| {
                panic!("hydrolysis LazyStack failed to materialize item {index}")
            });
            normalize_layout_view(view, env)
        };
        let (size, stretch) = {
            let mut cache = self.item_cache.borrow_mut();
            let subview = cache.entry(id, materialize);
            let measured = subview.patch_and_measure(renderer, env, proposal);
            self.transitions.track(id, subview, renderer);
            measured
        };
        let child_rect = place_lazy_stack_item(&self.axis, stretch, size, ctx.bounds(), cursor);
        // The item's frame in the context the stack recorded at layout:
        // `child_rect` resolves in `ctx.bounds` space — the same layout-fact
        // mapping `WidgetRenderContext` and the collection layout loop share
        // through `SafeAreaLayout::hosted_frame`.
        let item_area =
            stack_area.map(|area| area.with_frame(area.hosted_frame(ctx.bounds(), child_rect)));
        let slot = kurbo_size(child_rect);
        self.transitions.place(
            id,
            RowPlacement {
                slot: child_rect,
                content: kurbo::Rect::from_origin_size(kurbo::Point::ZERO, child_rect.size()),
                proposal,
                stretch,
            },
        );
        let row = self.transitions.row(id);
        let mut cache = self.item_cache.borrow_mut();
        let subview = cache.entry(id, materialize);
        if let Some(insertion) = &row.insertion {
            let delta = kurbo::Affine::translate((child_rect.x0, child_rect.y0));
            let child_ctx = ctx.child(
                delta,
                kurbo::Rect::new(0.0, 0.0, child_rect.width(), child_rect.height()),
            );
            let paint = TransitionPaint::sample(
                &insertion.state,
                slot,
                self.transitions.direction,
                true,
                &insertion.runtime,
                &self.core.cell,
            );
            flush_transitioned(renderer, child_ctx, delta, &paint, |renderer, inner| {
                subview.place_in_ctx(
                    renderer,
                    inner,
                    env,
                    proposal,
                    slot,
                    item_area,
                    kurbo::Affine::IDENTITY,
                );
            });
        } else {
            subview.place(renderer, ctx, env, proposal, child_rect, item_area);
        }
        lazy_main_extent(&self.axis, slot)
    }

    /// Paints removal ghost `index` in its frozen slot at `cursor`: paint
    /// only, with no input and no accessibility. Returns the slot's
    /// main-axis extent.
    fn flush_ghost(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
        index: usize,
        cursor: f64,
        stack_area: Option<&safe_area::SafeAreaLayout>,
    ) -> f64 {
        let mut ghost = self.transitions.ghost(index);
        let size = ghost.size();
        let LazyGhost {
            ghost,
            placement,
            runtime,
            ..
        } = &mut *ghost;
        let proposal = &placement.proposal;
        let rect = place_lazy_stack_item(&self.axis, placement.stretch, size, ctx.bounds(), cursor);
        let area = stack_area.map(|area| area.with_frame(area.hosted_frame(ctx.bounds(), rect)));
        let delta = kurbo::Affine::translate((rect.x0, rect.y0));
        let child_ctx = ctx.child(
            delta,
            kurbo::Rect::new(0.0, 0.0, rect.width(), rect.height()),
        );
        let paint = TransitionPaint::sample(
            &ghost.transition,
            size,
            self.transitions.direction,
            false,
            runtime,
            &self.core.cell,
        );
        let realization = &mut ghost.realization;
        renderer.with_suppressed_accessibility(|renderer| {
            flush_transitioned(renderer, child_ctx, delta, &paint, |renderer, inner| {
                realization.place_in_ctx(
                    renderer,
                    inner,
                    &self.env,
                    *proposal,
                    size,
                    area,
                    kurbo::Affine::IDENTITY,
                );
            });
        });
        lazy_main_extent(&self.axis, size)
    }

    /// Emits every item's accessibility nodes for the semantic walk. There is
    /// no viewport to bound emission against — the semantic tree contains the
    /// whole collection, so every item materializes its node rather than only
    /// the visible window. Removal ghosts left the tree with their rows.
    #[cfg(feature = "accessibility")]
    pub(super) fn emit_accessibility(&self, renderer: &mut SemanticCore<E>) {
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
            // The row's declaration is claimed before its first walk.
            subview.ensure_built(renderer, env);
            self.transitions.track(id, subview, renderer);
            subview.emit_accessibility(renderer, env);
        }
        self.end_frame();
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
    use waterui_core::layout::{HorizontalAlignment, VerticalAlignment};

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
