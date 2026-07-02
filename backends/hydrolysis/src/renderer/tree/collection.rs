//! Retained collection nodes: [`CollectionNode`] (reactive, non-virtualized,
//! reconciled by id) and [`LazyStackNode`] (viewport-virtualized lazy stack).

use super::*;

pub(crate) struct CollectionNode {
    /// The container layout (e.g. `AbsoluteLayout`, `ZStackLayout`).
    pub(super) layout: Box<dyn Layout>,
    /// The reactive item collection (`len`/`get_view`/`get_id`, watched).
    pub(super) views: AnyViews<AnyView>,
    /// Environment captured at build, used to materialize items.
    pub(super) env: Environment,
    /// Current items in order, keyed by id so a membership change keeps unchanged
    /// items' nodes (and their in-flight state) and only builds/drops the delta.
    pub(super) items: Vec<(CollectionItemId, RenderNode)>,
    /// Child frames cached by [`RenderNode::layout`], reused by `flush`.
    pub(super) placed: Vec<Rect>,
    /// Set by the membership watcher; consumed by `patch` to trigger a reconcile.
    pub(super) dirty: Rc<Cell<bool>>,
    /// Stable allocation whose address is this collection's patch dirty-key.
    #[allow(
        dead_code,
        reason = "pins the dirty-key address for the node's lifetime"
    )]
    pub(super) dirty_key: Rc<()>,
    /// Membership-change watcher; a change sets `dirty` and schedules a refresh.
    #[allow(dead_code, reason = "keeps the membership watcher subscription alive")]
    pub(super) guard: BoxWatcherGuard,
}

pub(crate) struct LazyStackNode {
    /// Stack axis + spacing + cross-axis alignment.
    pub(super) axis: LazyStackAxisConfig,
    /// The reactive item collection (`len`/`get_view`, watched for membership).
    pub(super) views: AnyViews<AnyView>,
    /// Environment captured at build, used to materialize and style items.
    pub(super) env: Environment,
    /// Per-index main-axis extent cache, persisting across flushes so a steady
    /// scroll only measures items entering the visible window.
    pub(super) item_extents: RefCell<Vec<Option<f64>>>,
    /// Retained node sub-views for the items currently in the visible window, keyed
    /// by stable id so a steady scroll reuses each visible item's node (keeping its
    /// reactive content live) and only builds items entering the window.
    pub(super) item_cache: RefCell<VisibleSubviewCache<CollectionItemId>>,
    /// Estimated extent for not-yet-measured items, seeded from the first measure;
    /// used to size the scroll content without measuring the whole collection.
    pub(super) estimate: Cell<f64>,
    /// Stable allocation whose address is this collection's patch dirty-key,
    /// owned so the key cannot be reused by another allocation while it lives.
    #[allow(
        dead_code,
        reason = "pins the dirty-key address for the node's lifetime"
    )]
    pub(super) dirty_key: Rc<()>,
    /// Membership-change watcher: a change schedules a window refresh, which
    /// re-resolves the visible window (the collection `len`/items are re-read).
    #[allow(dead_code, reason = "keeps the membership watcher subscription alive")]
    pub(super) guard: BoxWatcherGuard,
}

impl CollectionNode {
    /// Build the `NodeSubView` proxies for this collection's items, sharing one
    /// state cell across the level (mirrors [`ContainerNode`] measurement).
    pub(super) fn measure(&self, state: &mut HydroState, proposal: ProposalSize) -> ViewDimensions {
        let cell = RefCell::new(state);
        let subs: Vec<NodeSubView> = self
            .items
            .iter()
            .map(|(_, node)| NodeSubView::new(node, &cell, &self.env))
            .collect();
        let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();
        ViewDimensions::new(self.layout.size_that_fits(proposal, &refs))
    }

    pub(super) fn layout(&mut self, renderer: &mut HydrolysisRenderer, size: Size) {
        let env = self.env.clone();
        let rects = {
            let cell = RefCell::new(&mut renderer.state);
            let subs: Vec<NodeSubView> = self
                .items
                .iter()
                .map(|(_, node)| NodeSubView::new(node, &cell, &env))
                .collect();
            let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();
            self.layout.place(Rect::from_size(size), &refs)
        };
        for ((_, child), rect) in self.items.iter_mut().zip(rects.iter()) {
            child.layout(renderer, &env, *rect.size());
        }
        self.placed = rects;
    }

    pub(super) fn flush(&self, renderer: &mut HydrolysisRenderer, ctx: RenderContext) {
        for ((_, child), rect) in self.items.iter().zip(self.placed.iter()) {
            let child_ctx = ctx.child(
                vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y()))),
                vello::kurbo::Rect::new(
                    0.0,
                    0.0,
                    f64::from(rect.width()),
                    f64::from(rect.height()),
                ),
            );
            child.flush(renderer, child_ctx, &self.env);
        }
    }

    /// Apply a membership change: keep each surviving id's node (and its in-flight
    /// state), build newly-present ids, and drop departed ones — in the new order.
    pub(super) fn reconcile(&mut self, renderer: &mut HydrolysisRenderer) {
        let env = self.env.clone();
        let len = self.views.len().get();
        let mut surviving: std::collections::BTreeMap<CollectionItemId, RenderNode> =
            self.items.drain(..).collect();
        let mut next = Vec::with_capacity(len);
        for index in 0..len {
            let id = self
                .views
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis collection: item {index} has no id"));
            let node = surviving.remove(&id).unwrap_or_else(|| {
                let view = self
                    .views
                    .get_view(index)
                    .unwrap_or_else(|| panic!("hydrolysis collection: item {index} missing"));
                RenderNode::build(normalize_layout_view(view, &env), &env, renderer)
            });
            next.push((id, node));
        }
        self.items = next;
    }
}

impl LazyStackNode {
    fn spacing(&self) -> f64 {
        match self.axis {
            LazyStackAxisConfig::Vertical { spacing, .. }
            | LazyStackAxisConfig::Horizontal { spacing, .. } => spacing,
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
        let view = self
            .views
            .get_view(index)
            .unwrap_or_else(|| panic!("hydrolysis LazyStack failed to materialize item {index}"));
        let view = normalize_layout_view(view, &self.env);
        let bound = RefCell::new(&mut *state);
        let subview = HydroSubview::from_view(&view, &bound, &self.env);
        #[allow(clippy::cast_possible_truncation)]
        let proposal = match self.axis {
            LazyStackAxisConfig::Vertical { .. } => ProposalSize::new(Some(cross as f32), None),
            LazyStackAxisConfig::Horizontal { .. } => ProposalSize::new(None, Some(cross as f32)),
        };
        (subview.measure(proposal).size, subview.stretch_axis())
    }

    fn main_extent(&self, size: Size) -> f64 {
        match self.axis {
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
        self.item_extents.borrow_mut()[0] = Some(extent);
    }

    /// Total content extent along the main axis: cached extents where known,
    /// estimated elsewhere, plus inter-item spacing.
    fn content_main_extent(&self, count: usize) -> f64 {
        let extents = self.item_extents.borrow();
        let sum = sum_cached_or_estimated(&extents, self.estimate.get());
        sum + self.spacing() * count.saturating_sub(1) as f64
    }

    #[allow(clippy::cast_possible_truncation)]
    pub(super) fn measure(&self, state: &mut HydroState, proposal: ProposalSize) -> ViewDimensions {
        let count = self.views.len().get();
        self.item_extents.borrow_mut().resize(count, None);
        if count == 0 {
            return ViewDimensions::new(Size::zero());
        }
        let cross = match self.axis {
            LazyStackAxisConfig::Vertical { .. } => proposal.width.unwrap_or(0.0),
            LazyStackAxisConfig::Horizontal { .. } => proposal.height.unwrap_or(0.0),
        };
        self.ensure_estimate(state, f64::from(cross));
        let main = self.content_main_extent(count) as f32;
        let size = match self.axis {
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
        self.item_extents.borrow_mut().resize(count, None);
        if count == 0 {
            return;
        }
        let cross = match self.axis {
            LazyStackAxisConfig::Vertical { .. } => ctx.bounds.width(),
            LazyStackAxisConfig::Horizontal { .. } => ctx.bounds.height(),
        };
        self.ensure_estimate(&mut renderer.state, cross);
        self.item_cache.borrow_mut().begin_frame();
        let visible = renderer
            .lazy
            .lazy_viewport_stack
            .last()
            .copied()
            .unwrap_or(ctx.bounds);
        let (visible_start, visible_end) = match self.axis {
            LazyStackAxisConfig::Vertical { .. } => (visible.y0, visible.y1),
            LazyStackAxisConfig::Horizontal { .. } => (visible.x0, visible.x1),
        };
        let spacing = self.spacing();
        let window = resolve_visible_index_window(count, visible_start, visible_end, |index| {
            let cached = self.item_extents.borrow()[index];
            let extent = cached.unwrap_or_else(|| {
                let (size, _) = self.measure_item(&mut renderer.state, index, cross);
                let extent = self.main_extent(size);
                self.item_extents.borrow_mut()[index] = Some(extent);
                extent
            });
            if index + 1 < count {
                extent + spacing
            } else {
                extent
            }
        });
        let mut cursor = window.leading_offset;
        for index in window.start..window.end {
            let (size, stretch) = self.measure_item(&mut renderer.state, index, cross);
            let child_rect = place_lazy_stack_item(self.axis, stretch, size, ctx.bounds, cursor);
            let extent = match self.axis {
                LazyStackAxisConfig::Vertical { .. } => child_rect.height(),
                LazyStackAxisConfig::Horizontal { .. } => child_rect.width(),
            };
            self.item_extents.borrow_mut()[index] = Some(extent);
            let id = self
                .views
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis LazyStack item {index} has no id"));
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
    }
}
