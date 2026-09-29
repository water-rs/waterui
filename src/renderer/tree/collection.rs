//! Retained collection nodes: [`CollectionNode`] (reactive, non-virtualized,
//! reconciled by id) and [`LazyStackNode`] (viewport-virtualized lazy stack).

use super::*;
use std::rc::Rc;

use nami::collection::CollectionChange;
use waterui_core::animation::Animation;
use waterui_core::layout::Point;
use waterui_layout::collection_transition::CollectionTransition;

/// Collect into `out` the ids occupying the positions a [`CollectionChange`]
/// reports as replaced: `change.replaced` names new-snapshot indices, and
/// `ids` is the notified snapshot slice — index-parallel with the collection.
/// A producer that knows only a whole-value replacement reports
/// `everything`, so an empty change means nothing needs re-materializing.
pub(crate) fn collect_replaced_ids<Id: Copy + Eq + core::hash::Hash>(
    ids: &[Id],
    change: &CollectionChange,
    out: &mut std::collections::HashSet<Id>,
) {
    for range in &change.replaced {
        let start = range.start.min(ids.len());
        let end = range.end.min(ids.len());
        out.extend(ids[start..end].iter().copied());
    }
}

/// The membership-transition phase of a retained collection entry. An entry with
/// a transition fades and (along the stack axis) collapses in while `Entering`
/// and out while `Exiting`; `Stable` entries render at full size and opacity.
/// Without a transition every entry is `Stable`.
#[derive(Clone, Copy)]
pub(super) enum EntryPhase {
    Stable,
    /// Animating in since this instant.
    Entering(Instant),
    /// Animating out since this instant; dropped when the animation completes.
    Exiting(Instant),
}

impl EntryPhase {
    /// The 0..=1 presence factor at `now`: opacity and stack-axis size scale.
    fn factor(self, now: Instant, animation: &Animation) -> f32 {
        match self {
            Self::Stable => 1.0,
            Self::Entering(start) => animation.progress(now.saturating_duration_since(start)),
            Self::Exiting(start) => 1.0 - animation.progress(now.saturating_duration_since(start)),
        }
    }

    fn is_transitioning(self, now: Instant, animation: &Animation) -> bool {
        match self {
            Self::Stable => false,
            Self::Entering(start) | Self::Exiting(start) => {
                !animation.is_complete(now.saturating_duration_since(start))
            }
        }
    }

    fn is_finished_exit(self, now: Instant, animation: &Animation) -> bool {
        matches!(self, Self::Exiting(start) if animation.is_complete(now.saturating_duration_since(start)))
    }

    /// Whether an entry in this phase is kept out of the accessibility tree. The
    /// tree reflects the collection's settled *logical* membership: an entering
    /// entry was just added, so it belongs in the tree immediately (its fade-in
    /// is purely visual), while an exiting entry — already removed from the
    /// membership — is suppressed while it collapses out.
    const fn suppresses_accessibility(self) -> bool {
        matches!(self, Self::Exiting(_))
    }
}

/// The resolved transition timing for a collection: the animation curve and, for
/// a stack layout, the axis and spacing used to interpolate placement while
/// entries enter and exit.
pub(super) struct CollectionTransitionRuntime {
    animation: Animation,
    /// `Some` for a vertical/horizontal stack (enables axis collapse), `None`
    /// for any other layout (fade-only in place).
    axis: Option<TransitionAxis>,
}

#[derive(Clone, Copy)]
struct TransitionAxis {
    vertical: bool,
    spacing: f64,
}

/// One retained entry of a [`CollectionNode`]: the item's persistent node plus
/// its membership-transition state.
pub(super) struct CollectionEntry {
    /// Stable item identity from the source collection.
    pub(super) id: CollectionItemId,
    /// The item's retained node, kept across membership changes by id.
    pub(super) node: RenderNode,
    /// Membership-transition phase. Always `Stable` when the collection has no
    /// transition configured.
    phase: EntryPhase,
    /// This frame's presence factor, resolved once per frame by
    /// [`CollectionNode::advance_transitions`] from the frame clock so measure,
    /// layout, and flush all see the same value.
    factor: f32,
}

impl CollectionEntry {
    /// An at-rest entry: full presence, no transition. The initial membership
    /// of a collection is built from these.
    pub(super) fn stable(id: CollectionItemId, node: RenderNode) -> Self {
        Self {
            id,
            node,
            phase: EntryPhase::Stable,
            factor: 1.0,
        }
    }
}

/// Resolves the opt-in [`CollectionTransition`] for a collection from the
/// environment, pairing it with the stack axis/spacing when the layout is a
/// vertical or horizontal stack (so entries collapse along the axis; other
/// layouts cross-fade in place).
pub(super) fn collection_transition_runtime(
    env: &Environment,
    layout: &dyn Layout,
) -> Option<CollectionTransitionRuntime> {
    let transition = env.get::<CollectionTransition>()?;
    let axis = lazy_stack_axis_config(
        layout,
        nami::Computed::constant(waterui_core::layout::LayoutDirection::default()),
    )
    .map(|config| match config {
        LazyStackAxisConfig::Vertical { spacing, .. } => TransitionAxis {
            vertical: true,
            spacing: f64::from(spacing.snapshot()),
        },
        LazyStackAxisConfig::Horizontal { spacing, .. } => TransitionAxis {
            vertical: false,
            spacing: f64::from(spacing.snapshot()),
        },
    });
    Some(CollectionTransitionRuntime {
        animation: transition.animation.clone(),
        axis,
    })
}

/// The phase a reused live entry should carry after a reconcile: `Stable`
/// without a transition, a fresh enter when it was leaving, otherwise its
/// current phase (so an in-flight enter keeps running).
fn next_live_phase(
    transition: Option<&CollectionTransitionRuntime>,
    previous: EntryPhase,
    now: Instant,
) -> EntryPhase {
    match (transition.is_some(), previous) {
        (false, _) => EntryPhase::Stable,
        (true, EntryPhase::Exiting(_)) => EntryPhase::Entering(now),
        (true, phase) => phase,
    }
}

pub(crate) struct CollectionNode {
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
    /// The reactive item collection (`len`/`get_view`/`get_id`, watched).
    pub(super) views: AnyViews<AnyView>,
    /// Environment captured at build, used to materialize items. Already shielded
    /// when this collection carries accessibility naming metadata — the name
    /// belongs to the collection, not to every item in it.
    pub(super) env: Environment,
    /// Stable identity owning this collection's own accessibility node id, so the
    /// id survives membership changes shifting the sibling ordinals.
    pub(super) accessibility_identity: Rc<()>,
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
    /// Resolved membership transition, or `None` when the collection pops.
    pub(super) transition: Option<CollectionTransitionRuntime>,
    /// Set by the membership watcher; consumed by `patch` to trigger a reconcile.
    pub(super) dirty: Rc<Cell<bool>>,
    /// Ids the watcher reported as replaced since the last reconcile — the
    /// items whose content may differ under an unchanged id, so their nodes
    /// are rebuilt while every other surviving id keeps its node and state.
    /// Shared with the watcher closure via `Rc`.
    pub(super) replaced_ids: Rc<RefCell<std::collections::HashSet<CollectionItemId>>>,
    /// Stable allocation whose address is this collection's patch dirty-key.
    pub(super) _dirty_key: Rc<()>,
    /// Membership-change watcher; a change sets `dirty` and schedules a refresh.
    pub(super) _guard: BoxWatcherGuard,
    /// The collection layout's own `watch_invalidation` subscriptions — a
    /// change to a layout-input signal schedules the refresh that re-derives
    /// the collection's extents and item rects.
    pub(super) _layout_guards: Vec<BoxWatcherGuard>,
}

pub(crate) struct LazyStackNode {
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
    /// The reactive item collection (`len`/`get_view`, watched for membership).
    pub(super) views: AnyViews<AnyView>,
    /// Environment captured at build, used to materialize and style items. Already
    /// shielded when this stack carries accessibility naming metadata — the name
    /// belongs to the stack, not to every row in it.
    pub(super) env: Environment,
    /// Stable identity owning this stack's own accessibility node id, so the id
    /// survives the visible window shifting the sibling ordinals.
    pub(super) accessibility_identity: Rc<()>,
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
    /// Stable allocation whose address is this collection's patch dirty-key,
    /// owned so the key cannot be reused by another allocation while it lives.
    pub(super) _dirty_key: Rc<()>,
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

impl CollectionNode {
    /// Whether any entry is mid enter/exit, i.e. this frame's measure, layout,
    /// and flush must take the transitioning paths.
    fn has_active_transition(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| !matches!(entry.phase, EntryPhase::Stable))
    }

    /// Build the `NodeSubView` proxies for this collection's entries, sharing one
    /// state cell across the level (mirrors [`ContainerNode`] measurement).
    pub(super) fn measure(
        &self,
        state: &mut HydroState,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        if self.has_active_transition()
            && let Some(runtime) = &self.transition
            && let Some(axis) = runtime.axis
        {
            return ViewDimensions::new(
                self.measure_transitioning_stack(state, theme, proposal, axis),
            );
        }
        // At rest — and for fade-only (non-stack) transitions, whose exiting
        // entries keep their place while fading — the layout measures every
        // entry at full extent.
        let cell = RefCell::new(state);
        let subs: Vec<NodeSubView> = self
            .entries
            .iter()
            .map(|entry| NodeSubView::new(&entry.node, &cell, &self.env, theme))
            .collect();
        let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();
        ViewDimensions::new(self.layout.size_that_fits(proposal, &refs))
    }

    /// The animated stack size while entries enter/exit: each entry's main-axis
    /// extent (and its preceding gap) scales with its presence factor, so the
    /// container itself grows and shrinks smoothly and — layout being a full
    /// per-frame pass — the surrounding content reflows with it, releasing an
    /// exiting entry's space over the animation instead of popping.
    fn measure_transitioning_stack(
        &self,
        state: &mut HydroState,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
        axis: TransitionAxis,
    ) -> LayoutSize {
        let cell = RefCell::new(state);
        let (item_proposal, min_item_proposal, offered) = if axis.vertical {
            (
                ProposalSize::new(proposal.width, None),
                ProposalSize::new(proposal.width, Some(0.0)),
                proposal.height,
            )
        } else {
            (
                ProposalSize::new(None, proposal.height),
                ProposalSize::new(Some(0.0), proposal.height),
                proposal.width,
            )
        };
        let mut main = 0.0_f64;
        let mut floor = 0.0_f64;
        let mut cross = 0.0_f64;
        let mut first_visible = true;
        for entry in &self.entries {
            let factor = f64::from(entry.factor);
            if factor <= f64::EPSILON {
                continue;
            }
            let sub = NodeSubView::new(&entry.node, &cell, &self.env, theme);
            let size = sub.measure(item_proposal).size;
            let min_size = sub.measure(min_item_proposal).size;
            let (item_main, item_cross, min_main) = if axis.vertical {
                (
                    f64::from(size.height),
                    f64::from(size.width),
                    f64::from(min_size.height),
                )
            } else {
                (
                    f64::from(size.width),
                    f64::from(size.height),
                    f64::from(min_size.width),
                )
            };
            if !first_visible {
                main += axis.spacing * factor;
                floor += axis.spacing * factor;
            }
            first_visible = false;
            main += item_main * factor;
            floor += min_main * factor;
            cross = cross.max(item_cross);
        }
        // As in `LazyStackNode::measure`: the blended total is the stack's
        // ideal, not its minimum — a finite main-axis offer caps it, and the
        // items' blended minima floor it.
        let main = match offered {
            Some(offer) if offer.is_finite() => main.min(f64::from(offer)).max(floor),
            _ => main,
        };
        #[allow(clippy::cast_possible_truncation)]
        if axis.vertical {
            LayoutSize::new(cross as f32, main as f32)
        } else {
            LayoutSize::new(main as f32, cross as f32)
        }
    }

    pub(super) fn layout(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        proposal: ProposalSize,
        size: Size,
    ) {
        let env = self.env.clone();
        let theme = renderer.theme();
        let mut placements = {
            let cell = RefCell::new(&mut renderer.state);
            let subs: Vec<NodeSubView> = self
                .entries
                .iter()
                .map(|entry| NodeSubView::new(&entry.node, &cell, &env, &theme))
                .collect();
            let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();
            self.layout.place(Rect::from_size(size), proposal, &refs)
        };
        if self.has_active_transition()
            && let Some(runtime) = &self.transition
            && let Some(axis) = runtime.axis
        {
            // The layout placed every entry at full extent, which supplies the
            // cross-axis position and size (alignment). Re-accumulate the
            // main-axis positions from the presence factors: each entry keeps
            // its full extent in `placed` (flush clips it to the factor), but
            // occupies only its scaled extent — neighbours slide smoothly as
            // it enters or exits.
            let mut cursor = 0.0_f32;
            let mut first_visible = true;
            for (entry, placement) in self.entries.iter().zip(&mut placements) {
                let rect = &mut placement.frame;
                let factor = entry.factor;
                if factor <= f32::EPSILON {
                    // Fully absent this frame: park it at the cursor with its
                    // full extent; flush skips it entirely.
                    *rect = place_on_axis(*rect, axis, cursor);
                    continue;
                }
                #[allow(clippy::cast_possible_truncation)]
                if !first_visible {
                    cursor += axis.spacing as f32 * factor;
                }
                first_visible = false;
                *rect = place_on_axis(*rect, axis, cursor);
                let extent = if axis.vertical {
                    rect.height()
                } else {
                    rect.width()
                };
                cursor += extent * factor;
            }
        }
        for (entry, placement) in self.entries.iter_mut().zip(&placements) {
            entry
                .node
                .layout(renderer, &env, placement.proposal, *placement.frame.size());
        }
        self.placed = placements
            .into_iter()
            .map(|placement| placement.frame)
            .collect();
    }

    pub(super) fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext) {
        #[cfg(feature = "accessibility")]
        let container_scope = self.accessibility_container_env.as_ref().map(|env| {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            let scope = renderer.begin_accessibility_container(
                transformed_rect(ctx.hit_transform, ctx.bounds),
                env,
            );
            renderer.pop_accessibility_owner();
            scope
        });
        let axis = self.transition.as_ref().and_then(|runtime| runtime.axis);
        for (entry, rect) in self.entries.iter().zip(self.placed.iter()) {
            let factor = entry.factor;
            if factor <= f32::EPSILON {
                continue;
            }
            let child_ctx = ctx.child(
                kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y()))),
                kurbo::Rect::new(0.0, 0.0, f64::from(rect.width()), f64::from(rect.height())),
            );
            if entry.phase.suppresses_accessibility() {
                renderer.with_suppressed_accessibility(|renderer| {
                    Self::flush_entry(renderer, entry, child_ctx, &self.env, factor, axis);
                });
            } else {
                Self::flush_entry(renderer, entry, child_ctx, &self.env, factor, axis);
            }
        }
        #[cfg(feature = "accessibility")]
        if let Some(container_scope) = container_scope {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            renderer.end_accessibility_container(container_scope);
            renderer.pop_accessibility_owner();
        }
    }

    /// Emits every stable entry's accessibility nodes for the semantic walk —
    /// the same container scope and per-phase suppression the rendered flush
    /// applies, with no bounds and no clip layers.
    #[cfg(feature = "accessibility")]
    pub(super) fn emit_accessibility(&self, renderer: &mut SemanticCore) {
        let container_scope = self.accessibility_container_env.as_ref().map(|env| {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            let scope = renderer.begin_accessibility_container_semantic(env);
            renderer.pop_accessibility_owner();
            scope
        });
        for entry in &self.entries {
            if entry.factor <= f32::EPSILON {
                continue;
            }
            if entry.phase.suppresses_accessibility() {
                renderer.with_suppressed_accessibility(|renderer| {
                    entry.node.emit_accessibility(renderer, &self.env);
                });
            } else {
                entry.node.emit_accessibility(renderer, &self.env);
            }
        }
        if let Some(container_scope) = container_scope {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            renderer.end_accessibility_container(container_scope);
            renderer.pop_accessibility_owner();
        }
    }

    /// Flushes one entry, wrapping it in an opacity+clip layer while it is
    /// transitioning (`factor < 1`) and flushing it directly otherwise. The clip
    /// trims the entry to its factor-scaled extent along the stack axis (the
    /// visual collapse); a fade-only transition clips nothing and only fades.
    fn flush_entry(
        renderer: &mut HydrolysisRenderer,
        entry: &CollectionEntry,
        child_ctx: RenderContext,
        env: &Environment,
        factor: f32,
        axis: Option<TransitionAxis>,
    ) {
        if factor >= 1.0 {
            entry.node.flush(renderer, child_ctx, env);
            return;
        }
        let bounds = child_ctx.bounds;
        let clip = match axis {
            Some(TransitionAxis { vertical: true, .. }) => kurbo::Rect::new(
                0.0,
                0.0,
                bounds.width(),
                bounds.height() * f64::from(factor),
            ),
            Some(TransitionAxis {
                vertical: false, ..
            }) => kurbo::Rect::new(
                0.0,
                0.0,
                bounds.width() * f64::from(factor),
                bounds.height(),
            ),
            None => bounds,
        };
        renderer.with_clip_rect_scope(
            factor,
            LayerTransforms {
                paint: child_ctx.transform,
                hit: child_ctx.hit_transform,
            },
            clip,
            |renderer| {
                let previous_opacity = renderer.hit_test.hit_test_opacity;
                renderer.hit_test.hit_test_opacity = previous_opacity * factor;
                entry.node.flush(renderer, child_ctx, env);
                renderer.hit_test.hit_test_opacity = previous_opacity;
            },
        );
    }

    /// Apply a membership change: keep each surviving id's node (and its
    /// in-flight state), build newly-present ids, and drop departed ones — in
    /// the new order. Entries the watcher flagged as replaced keep their id
    /// and phase but their node is rebuilt — a same-id content change re-
    /// materializes exactly that row. With a transition, departed entries
    /// instead begin their exit — kept in display order, anchored after the
    /// live id they followed — and new ids animate in (the initial membership
    /// was built at rest by [`RenderNode::build_collection`]; only later
    /// changes reach here).
    pub(super) fn reconcile(&mut self, renderer: &mut SemanticCore) {
        renderer.state.counters.structural_patches += 1;
        let env = self.env.clone();
        let len = self.views.len().snapshot();
        let now = renderer.frame_instant;
        let animated = self.transition.is_some();

        let ids: Vec<CollectionItemId> = (0..len)
            .map(|index| {
                self.views
                    .get_id(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} has no id"))
            })
            .collect();
        let live: std::collections::HashSet<CollectionItemId> = ids.iter().copied().collect();

        // Partition the previous display order into entries still live
        // (reusable, keyed by id) and departed ones. With a transition the
        // latter keep collapsing out, anchored to the live id they followed;
        // without one they are dropped here (releasing their retained nodes).
        let mut reuse_by_id: std::collections::BTreeMap<CollectionItemId, CollectionEntry> =
            std::collections::BTreeMap::new();
        let mut head_dead: Vec<CollectionEntry> = Vec::new();
        let mut dead_after: std::collections::BTreeMap<CollectionItemId, Vec<CollectionEntry>> =
            std::collections::BTreeMap::new();
        {
            let mut last_live: Option<CollectionItemId> = None;
            for entry in self.entries.drain(..) {
                if live.contains(&entry.id) {
                    last_live = Some(entry.id);
                    reuse_by_id.insert(entry.id, entry);
                } else if animated {
                    match last_live {
                        Some(anchor) => dead_after.entry(anchor).or_default().push(entry),
                        None => head_dead.push(entry),
                    }
                }
            }
        }

        let begin_exit = |mut entry: CollectionEntry| -> CollectionEntry {
            if !matches!(entry.phase, EntryPhase::Exiting(_)) {
                entry.phase = EntryPhase::Exiting(now);
            }
            entry
        };

        let mut next = Vec::with_capacity(len + head_dead.len());
        next.extend(head_dead.into_iter().map(begin_exit));
        let replaced = core::mem::take(&mut *self.replaced_ids.borrow_mut());
        for (index, id) in ids.into_iter().enumerate() {
            let entry = match reuse_by_id.remove(&id) {
                Some(mut previous) => {
                    previous.phase = next_live_phase(self.transition.as_ref(), previous.phase, now);
                    if replaced.contains(&id) {
                        // Same id, changed content: re-materialize this row's
                        // node from the current item. The entry keeps its
                        // identity and phase; only the node is rebuilt.
                        let view = self.views.get_view(index).unwrap_or_else(|| {
                            panic!("hydrolysis collection: item {index} missing")
                        });
                        previous.node =
                            RenderNode::build(normalize_layout_view(view, &env), &env, renderer);
                    }
                    previous
                }
                None => {
                    let view = self
                        .views
                        .get_view(index)
                        .unwrap_or_else(|| panic!("hydrolysis collection: item {index} missing"));
                    CollectionEntry {
                        id,
                        node: RenderNode::build(normalize_layout_view(view, &env), &env, renderer),
                        phase: if animated {
                            EntryPhase::Entering(now)
                        } else {
                            EntryPhase::Stable
                        },
                        factor: if animated { 0.0 } else { 1.0 },
                    }
                }
            };
            next.push(entry);
            if let Some(dead) = dead_after.remove(&id) {
                next.extend(dead.into_iter().map(begin_exit));
            }
        }
        // Entries whose anchor departed in the same update continue their exit
        // animation after the remaining live entries.
        next.extend(dead_after.into_values().flatten().map(begin_exit));
        self.entries = next;
    }

    /// Settle finished transitions and resolve this frame's presence factors
    /// from the frame clock: finished exits drop their entries (a structural
    /// change, so the caller runs the prune cycle), finished enters become
    /// `Stable`, and while anything is still mid-flight a refresh is requested
    /// so frames keep coming until the collection settles. Returns whether the
    /// tree changed shape or is still animating (both need a fresh layout).
    pub(super) fn advance_transitions(&mut self, renderer: &mut SemanticCore) -> bool {
        let Some(runtime) = &self.transition else {
            return false;
        };
        let animation = runtime.animation.clone();
        let now = renderer.frame_instant;
        let before = self.entries.len();
        self.entries
            .retain(|entry| !entry.phase.is_finished_exit(now, &animation));
        let dropped = self.entries.len() != before;
        let mut transitioning = false;
        for entry in &mut self.entries {
            if let EntryPhase::Entering(start) = entry.phase
                && animation.is_complete(now.saturating_duration_since(start))
            {
                entry.phase = EntryPhase::Stable;
            }
            entry.factor = entry.phase.factor(now, &animation).clamp(0.0, 1.0);
            transitioning |= entry.phase.is_transitioning(now, &animation);
        }
        if transitioning {
            renderer.signals.request_refresh();
        }
        dropped || transitioning
    }
}

/// Moves `rect` to `main_position` along the transition axis, keeping its
/// cross-axis placement and full extent.
fn place_on_axis(rect: Rect, axis: TransitionAxis, main_position: f32) -> Rect {
    let origin = if axis.vertical {
        Point::new(rect.x(), main_position)
    } else {
        Point::new(main_position, rect.y())
    };
    Rect::new(origin, *rect.size())
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
        let count = self.views.len().snapshot();
        if count > 0
            && let Some((visible_start, visible_end)) = self.visible_span.get()
        {
            let estimate = self.estimate.get();
            let spacing = self.spacing();
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
            let views = &self.views;
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

    fn spacing(&self) -> f64 {
        match &self.axis {
            LazyStackAxisConfig::Vertical { spacing, .. }
            | LazyStackAxisConfig::Horizontal { spacing, .. } => f64::from(spacing.snapshot()),
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

    /// Measures item `index` under the given proposal, returning its
    /// measured size and stretch axis (both needed to place it).
    fn measure_item(
        &self,
        state: &mut HydroState,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        index: usize,
        proposal: ProposalSize,
    ) -> (Size, StretchAxis) {
        let id = self
            .views
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

        let view = self
            .views
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
        state: &mut HydroState,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        cross: Option<f32>,
    ) -> f64 {
        if let Some((previous, floor)) = self.floor_sample.get()
            && previous == cross
        {
            return floor;
        }
        let (size, _) = self.measure_item(state, theme, 0, self.item_min_proposal(cross));
        let floor = self.main_extent(size);
        self.floor_sample.set(Some((cross, floor)));
        floor
    }

    /// Keeps estimates scoped to the cross-axis query and collection lifetime.
    fn ensure_estimate(
        &self,
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
        let (size, _) = self.measure_item(state, theme, 0, self.item_proposal(cross));
        let extent = self.main_extent(size);
        self.estimate.set(extent.max(1.0));
        self.estimate_sample.set(Some((cross, size)));
        self.floor_sample.set(None);
        self.dirty.set(true);
        self.prepare_extent_index(self.views.len().snapshot());
        self.extent_index.borrow_mut().set_measured(0, extent);
        size
    }

    fn prepare_extent_index(&self, count: usize) {
        let estimate = self.estimate.get();
        if count == 0 || estimate <= 0.0 {
            return;
        }
        let spacing = self.spacing();
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
        state: &mut HydroState,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        count: usize,
        cross: Option<f32>,
    ) {
        let visible = self.visible_range.borrow().clone();
        let cache = self.item_cache.borrow();
        for index in visible.start.min(count)..visible.end.min(count) {
            let id = self
                .views
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
        let count = self.views.len().snapshot();
        if count == 0 {
            return ViewDimensions::new(Size::zero());
        }
        let (cross, offered) = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => (proposal.width, proposal.height),
            LazyStackAxisConfig::Horizontal { .. } => (proposal.height, proposal.width),
        };
        let sample = self.ensure_estimate(state, theme, cross);
        self.prepare_extent_index(count);
        self.refresh_visible_extents(state, theme, count, cross);
        // The extent index holds the items' intrinsic main extents, so the
        // unclamped total is the stack's ideal, not its minimum: a lazy
        // stack virtualizes — it fits a finite main-axis offer below its
        // extent by showing fewer items — while an open axis reads the full
        // extent. It never reports below the items' summed minima: content
        // that cannot shrink keeps its extent, as the eager stack's
        // `minima_overflow` answer does.
        let extent = self.extent_index.borrow().total_extent();
        let floor = self.ensure_floor(state, theme, cross) * count as f64
            + self.spacing() * (count - 1) as f64;
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
    pub(super) fn flush(
        &self,
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        _env: &Environment,
    ) {
        let count = self.views.len().snapshot();
        if count == 0 {
            return;
        }
        #[cfg(feature = "accessibility")]
        let container_scope = self.accessibility_container_env.as_ref().map(|env| {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            let scope = renderer.begin_accessibility_container(
                transformed_rect(ctx.hit_transform, ctx.bounds),
                env,
            );
            renderer.pop_accessibility_owner();
            scope
        });
        let cross = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => Some(ctx.bounds.width() as f32),
            LazyStackAxisConfig::Horizontal { .. } => Some(ctx.bounds.height() as f32),
        };
        let theme = renderer.theme();
        self.ensure_estimate(&mut renderer.state, &theme, cross);
        self.prepare_extent_index(count);
        let total_extent_before = self.extent_index.borrow().total_extent();
        self.item_cache.borrow_mut().begin_frame();
        let visible = renderer
            .lazy
            .lazy_viewport_stack
            .last()
            .map(|viewport| {
                (ctx.transform.inverse() * viewport.transform).transform_rect_bbox(viewport.bounds)
            })
            .unwrap_or(ctx.bounds);
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
        let spacing = self.spacing();
        let window = self
            .extent_index
            .borrow()
            .visible_window(visible_start, visible_end);
        *self.visible_range.borrow_mut() = window.start..window.end;
        let mut cursor = window.leading_offset;
        for index in window.start..window.end {
            let id = self
                .views
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
            let proposal = self.item_proposal(cross);
            let (size, stretch) = {
                let env = &self.env;
                let views = &self.views;
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
                let views = &self.views;
                let mut cache = self.item_cache.borrow_mut();
                let subview = cache.entry(id, || {
                    let view = views.get_view(index).unwrap_or_else(|| {
                        panic!("hydrolysis LazyStack failed to materialize item {index}")
                    });
                    normalize_layout_view(view, env)
                });
                subview.flush_in_rect(renderer, ctx, env, proposal, child_rect);
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
            renderer.request_refresh();
        }
    }

    /// Emits every item's accessibility nodes for the semantic walk. There is
    /// no viewport to bound emission against — the semantic tree contains the
    /// whole collection, so every item materializes its node rather than only
    /// the visible window.
    #[cfg(feature = "accessibility")]
    pub(super) fn emit_accessibility(&self, renderer: &mut SemanticCore) {
        let count = self.views.len().snapshot();
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
        for index in 0..count {
            let id = self
                .views
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
            let env = &self.env;
            let views = &self.views;
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
