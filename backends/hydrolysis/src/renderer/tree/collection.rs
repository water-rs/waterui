//! Retained collection nodes: [`CollectionNode`] (reactive, non-virtualized,
//! reconciled by id) and [`LazyStackNode`] (viewport-virtualized lazy stack).

use super::*;

use waterui_core::animation::Animation;
use waterui_core::layout::Point;
use waterui_layout::collection_transition::CollectionTransition;

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
            spacing: f64::from(spacing.get()),
        },
        LazyStackAxisConfig::Horizontal { spacing, .. } => TransitionAxis {
            vertical: false,
            spacing: f64::from(spacing.get()),
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
    #[cfg(feature = "accessibility")]
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
    /// Stable allocation whose address is this collection's patch dirty-key.
    pub(super) _dirty_key: Rc<()>,
    /// Membership-change watcher; a change sets `dirty` and schedules a refresh.
    pub(super) _guard: BoxWatcherGuard,
}

pub(crate) struct LazyStackNode {
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
    #[cfg(feature = "accessibility")]
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
    /// Estimated extent for not-yet-measured items, seeded from the first measure;
    /// used to size the scroll content without measuring the whole collection.
    pub(super) estimate: Cell<f64>,
    /// Membership changes reset index-based measurements, including moves that
    /// preserve the collection length.
    pub(super) dirty: Rc<Cell<bool>>,
    /// Stable allocation whose address is this collection's patch dirty-key,
    /// owned so the key cannot be reused by another allocation while it lives.
    pub(super) _dirty_key: Rc<()>,
    /// Membership-change watcher: a change schedules a window refresh, which
    /// re-resolves the visible window (the collection `len`/items are re-read).
    pub(super) _guard: BoxWatcherGuard,
    /// Direction-change watcher: locale changes immediately mirror placement.
    pub(super) _direction_guard: BoxWatcherGuard,
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
    pub(super) fn measure(&self, state: &mut HydroState, proposal: ProposalSize) -> ViewDimensions {
        if self.has_active_transition()
            && let Some(runtime) = &self.transition
            && let Some(axis) = runtime.axis
        {
            return ViewDimensions::new(self.measure_transitioning_stack(state, proposal, axis));
        }
        // At rest — and for fade-only (non-stack) transitions, whose exiting
        // entries keep their place while fading — the layout measures every
        // entry at full extent.
        let cell = RefCell::new(state);
        let subs: Vec<NodeSubView> = self
            .entries
            .iter()
            .map(|entry| NodeSubView::new(&entry.node, &cell, &self.env))
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
        proposal: ProposalSize,
        axis: TransitionAxis,
    ) -> LayoutSize {
        let cell = RefCell::new(state);
        let item_proposal = if axis.vertical {
            ProposalSize::new(proposal.width, None)
        } else {
            ProposalSize::new(None, proposal.height)
        };
        let mut main = 0.0_f64;
        let mut cross = 0.0_f64;
        let mut first_visible = true;
        for entry in &self.entries {
            let factor = f64::from(entry.factor);
            if factor <= f64::EPSILON {
                continue;
            }
            let sub = NodeSubView::new(&entry.node, &cell, &self.env);
            let size = sub.measure(item_proposal).size;
            let (item_main, item_cross) = if axis.vertical {
                (f64::from(size.height), f64::from(size.width))
            } else {
                (f64::from(size.width), f64::from(size.height))
            };
            if !first_visible {
                main += axis.spacing * factor;
            }
            first_visible = false;
            main += item_main * factor;
            cross = cross.max(item_cross);
        }
        #[allow(clippy::cast_possible_truncation)]
        if axis.vertical {
            LayoutSize::new(cross as f32, main as f32)
        } else {
            LayoutSize::new(main as f32, cross as f32)
        }
    }

    pub(super) fn layout(&mut self, renderer: &mut HydrolysisRenderer, size: Size) {
        let env = self.env.clone();
        let mut rects = {
            let cell = RefCell::new(&mut renderer.state);
            let subs: Vec<NodeSubView> = self
                .entries
                .iter()
                .map(|entry| NodeSubView::new(&entry.node, &cell, &env))
                .collect();
            let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();
            self.layout.place(Rect::from_size(size), &refs)
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
            for (entry, rect) in self.entries.iter().zip(rects.iter_mut()) {
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
        for (entry, rect) in self.entries.iter_mut().zip(rects.iter()) {
            entry.node.layout(renderer, &env, *rect.size());
        }
        self.placed = rects;
    }

    pub(super) fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext) {
        #[cfg(feature = "accessibility")]
        let container_scope = self.accessibility_container_env.as_ref().map(|env| {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            let scope = renderer
                .begin_accessibility_container(transformed_rect(ctx.hit_transform, ctx.bounds), env);
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
                vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y()))),
                vello::kurbo::Rect::new(
                    0.0,
                    0.0,
                    f64::from(rect.width()),
                    f64::from(rect.height()),
                ),
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
            Some(TransitionAxis { vertical: true, .. }) => vello::kurbo::Rect::new(
                0.0,
                0.0,
                bounds.width(),
                bounds.height() * f64::from(factor),
            ),
            Some(TransitionAxis {
                vertical: false, ..
            }) => vello::kurbo::Rect::new(
                0.0,
                0.0,
                bounds.width() * f64::from(factor),
                bounds.height(),
            ),
            None => bounds,
        };
        renderer.push_layer_rect(factor, child_ctx.transform, clip);
        let previous_opacity = renderer.hit_test.hit_test_opacity;
        renderer.hit_test.hit_test_opacity = previous_opacity * factor;
        entry.node.flush(renderer, child_ctx, env);
        renderer.hit_test.hit_test_opacity = previous_opacity;
        renderer.pop_layer();
    }

    /// Apply a membership change: keep each surviving id's node (and its
    /// in-flight state), build newly-present ids, and drop departed ones — in
    /// the new order. With a transition, departed entries instead begin their
    /// exit — kept in display order, anchored after the live id they followed —
    /// and new ids animate in (the initial membership was built at rest by
    /// [`RenderNode::build_collection`]; only later changes reach here).
    pub(super) fn reconcile(&mut self, renderer: &mut HydrolysisRenderer) {
        let env = self.env.clone();
        let len = self.views.len().get();
        let now = renderer.frame_instant;
        let animated = self.transition.is_some();

        let ids: Vec<CollectionItemId> = (0..len)
            .map(|index| {
                self.views
                    .get_id(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} has no id"))
            })
            .collect();
        let live: std::collections::BTreeSet<CollectionItemId> = ids.iter().copied().collect();

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
        for (index, id) in ids.into_iter().enumerate() {
            let entry = match reuse_by_id.remove(&id) {
                Some(mut previous) => {
                    previous.phase = next_live_phase(self.transition.as_ref(), previous.phase, now);
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
    pub(super) fn advance_transitions(&mut self, renderer: &mut HydrolysisRenderer) -> bool {
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
    /// before the parent scroll view measures this stack.
    pub(super) fn patch_visible(&self, renderer: &mut HydrolysisRenderer) -> bool {
        self.item_cache.borrow_mut().patch_for_parent(renderer)
    }

    fn spacing(&self) -> f64 {
        match &self.axis {
            LazyStackAxisConfig::Vertical { spacing, .. }
            | LazyStackAxisConfig::Horizontal { spacing, .. } => f64::from(spacing.get()),
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn item_proposal(&self, cross: f64) -> ProposalSize {
        match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => ProposalSize::new(Some(cross as f32), None),
            LazyStackAxisConfig::Horizontal { .. } => ProposalSize::new(None, Some(cross as f32)),
        }
    }

    /// Measures item `index` under the given cross-axis extent, returning its
    /// measured size and stretch axis (both needed to place it).
    fn measure_item(
        &self,
        state: &mut HydroState,
        index: usize,
        cross: f64,
    ) -> (Size, StretchAxis) {
        let id = self
            .views
            .get_id(index)
            .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
        let proposal = self.item_proposal(cross);
        if let Some(item) = self.item_cache.borrow().get(&id) {
            return (
                item.measure_built_with_proposal(state, &self.env, proposal),
                item.stretch_axis(),
            );
        }

        let view = self
            .views
            .get_view(index)
            .unwrap_or_else(|| panic!("hydrolysis LazyStack failed to materialize item {index}"));
        let view = normalize_layout_view(view, &self.env);
        let bound = RefCell::new(&mut *state);
        let subview = HydroSubview::from_view(&view, &bound, &self.env);
        (subview.measure(proposal).size, subview.stretch_axis())
    }

    fn main_extent(&self, size: Size) -> f64 {
        match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => f64::from(size.height),
            LazyStackAxisConfig::Horizontal { .. } => f64::from(size.width),
        }
    }

    /// Seeds the estimated item extent from item 0 if not yet known.
    fn ensure_estimate(&self, state: &mut HydroState, cross: f64) {
        if self.estimate.get() > 0.0 {
            return;
        }
        let (size, _) = self.measure_item(state, 0, cross);
        let extent = self.main_extent(size);
        self.estimate.set(extent.max(1.0));
        self.prepare_extent_index(self.views.len().get());
        self.extent_index.borrow_mut().set_measured(0, extent);
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
    fn refresh_visible_extents(&self, state: &mut HydroState, count: usize, cross: f64) {
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
            let size = item.measure_built_with_proposal(state, &self.env, proposal);
            let extent = self.main_extent(size);
            self.extent_index.borrow_mut().set_measured(index, extent);
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    pub(super) fn measure(&self, state: &mut HydroState, proposal: ProposalSize) -> ViewDimensions {
        let count = self.views.len().get();
        if count == 0 {
            return ViewDimensions::new(Size::zero());
        }
        let cross = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => proposal.width.unwrap_or(0.0),
            LazyStackAxisConfig::Horizontal { .. } => proposal.height.unwrap_or(0.0),
        };
        self.ensure_estimate(state, f64::from(cross));
        self.prepare_extent_index(count);
        self.refresh_visible_extents(state, count, f64::from(cross));
        let main = self.extent_index.borrow().total_extent() as f32;
        let size = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => Size::new(cross, main),
            LazyStackAxisConfig::Horizontal { .. } => Size::new(main, cross),
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
        let count = self.views.len().get();
        if count == 0 {
            return;
        }
        #[cfg(feature = "accessibility")]
        let container_scope = self.accessibility_container_env.as_ref().map(|env| {
            renderer.push_accessibility_owner(&self.accessibility_identity);
            let scope = renderer
                .begin_accessibility_container(transformed_rect(ctx.hit_transform, ctx.bounds), env);
            renderer.pop_accessibility_owner();
            scope
        });
        let cross = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => ctx.bounds.width(),
            LazyStackAxisConfig::Horizontal { .. } => ctx.bounds.height(),
        };
        self.ensure_estimate(&mut renderer.state, cross);
        self.prepare_extent_index(count);
        let total_extent_before = self.extent_index.borrow().total_extent();
        self.item_cache.borrow_mut().begin_frame();
        let visible = renderer
            .lazy
            .lazy_viewport_stack
            .last()
            .copied()
            .unwrap_or(ctx.bounds);
        let (visible_start, visible_end) = match &self.axis {
            LazyStackAxisConfig::Vertical { .. } => (visible.y0, visible.y1),
            LazyStackAxisConfig::Horizontal { .. }
                if self.axis.direction().get().is_right_to_left() =>
            {
                (
                    ctx.bounds.x0 + ctx.bounds.x1 - visible.x1,
                    ctx.bounds.x0 + ctx.bounds.x1 - visible.x0,
                )
            }
            LazyStackAxisConfig::Horizontal { .. } => (visible.x0, visible.x1),
        };
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
                subview.flush_in_rect(renderer, ctx, env, child_rect);
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
}
