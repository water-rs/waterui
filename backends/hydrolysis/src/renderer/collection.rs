//! Retained reactive collections.
//!
//! A `LazyContainer` whose layout is *not* a viewport-virtualized stack
//! (`AbsoluteLayout` / `ZStackLayout` overlays — see [`lazy_stack_axis_config`])
//! is rendered here as a retained per-item collection: every item's content is
//! captured once into its own [`DynamicSubtree`], keyed by the item's identity
//! in [`CollectionCache::items`]. A membership change reconciles only the
//! changed items — new ids are dispatched, removed ids are evicted, unchanged
//! ids keep their cached subtree (so in-flight animations, ripple state, and
//! `on_appear` one-shots survive) — and the window frame re-composites from the
//! unchanged placements of every other item. This is the collection counterpart
//! of the `Dynamic`-node reactive patch (see [`DynamicNodeDraw`]).
//!
//! The cache is keyed by a [`CollectionController`] slot whose address is stable
//! across structural rebuilds (cursor reuse, like the scroll controller), and it
//! keeps the *first* capture's type-erased [`SharedAnyViews`] — and thus its id
//! mapping — for the collection's lifetime, so item identities stay stable
//! across rebuilds (a window resize re-composites the same cached items instead
//! of re-dispatching every one).

use super::*;
use nami::watcher::BoxWatcherGuard;
use waterui_core::id::{Id as RawId, SelfId};
use waterui_core::views::{AnyViews, SharedAnyViews, Views};

/// The type-erased item identity used across every retained collection.
type CollectionId = SelfId<RawId>;

/// One retained item in a [`CollectionCache`], captured once and placed by the
/// collection's layout.
pub(crate) struct CollectionItem {
    /// Stable item identity from the source collection.
    id: CollectionId,
    /// The item's content, captured in local (identity) coordinates.
    subtree: DynamicSubtree,
    /// Placement within the container's local frame (origin offset).
    base_transform: vello::kurbo::Affine,
    base_hit_transform: vello::kurbo::Affine,
    /// The item's local bounds (origin `0,0`, size = its placed rect). The
    /// captured subtree is valid for exactly this size; a placement that resizes
    /// the item re-dispatches it.
    bounds: vello::kurbo::Rect,
}

/// Retained state for one reactive collection, keyed by its controller slot
/// `cache_key` in [`HydrolysisRenderer::collection_caches`].
pub(crate) struct CollectionCache {
    /// The source collection. Kept across rebuilds so its id mapping is stable.
    views: SharedAnyViews<AnyView>,
    /// The collection's layout, used to place items on capture and reconcile.
    layout: Box<dyn Layout>,
    /// Items in collection order, each holding its retained subtree + placement.
    items: Vec<CollectionItem>,
    /// The container's local bounds the items were placed within (origin `0,0`).
    container_bounds: vello::kurbo::Rect,
    /// The real-coordinate context the collection was dispatched with, used to
    /// re-dispatch new items in isolation during a reactive patch.
    dispatch_ctx: RenderContext,
    /// The environment the collection was dispatched with.
    dispatch_env: Environment,
    /// Rebuild generation captured at watch registration, gating dirty marks.
    render_generation: u64,
    /// `Views::watch` guard; dropping it (cache eviction) unregisters the watch.
    _watch_guard: BoxWatcherGuard,
}

/// A placement of a reactive collection within a captured subtree. Its items are
/// composited from [`CollectionCache::items`] at replay, each at its placed
/// transform; a membership change patches only the changed items.
pub(crate) struct DynamicCollectionDraw {
    cache_key: usize,
    base_transform: vello::kurbo::Affine,
    base_hit_transform: vello::kurbo::Affine,
}

fn size_near(left: vello::kurbo::Rect, right: vello::kurbo::Rect) -> bool {
    (left.width() - right.width()).abs() <= 0.5 && (left.height() - right.height()).abs() <= 0.5
}

impl HydrolysisRenderer {
    /// Renders a non-virtualized `LazyContainer` (`AbsoluteLayout`/`ZStackLayout`
    /// overlay) as a retained per-item collection. Reuses the existing cache for
    /// this slot (so item identities and unchanged subtrees survive across
    /// rebuilds), reconciles the current membership, and either defers a
    /// [`DynamicDrawOp::Collection`] (inside a retained capture) or replays the
    /// items inline (outside one).
    pub(crate) fn capture_collection(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        layout: Box<dyn Layout>,
        children: AnyViews<AnyView>,
        env: &Environment,
    ) {
        let cache_key = renderer.lazy.collection_controller.bind();

        // Reuse the existing slot cache (keeping its stable id mapping + watch),
        // or register a fresh one on first encounter.
        let (views, watch_guard, render_generation, existing_items) =
            match renderer.collection_caches.remove(&cache_key) {
                Some(cache) => (
                    cache.views,
                    cache._watch_guard,
                    cache.render_generation,
                    cache.items,
                ),
                None => {
                    let views: SharedAnyViews<AnyView> = children.into();
                    let render_generation = renderer.signals.rebuild_generation();
                    let guard = register_collection_watch(
                        &views,
                        cache_key,
                        render_generation,
                        &renderer.signals,
                    );
                    (views, guard, render_generation, Vec::new())
                }
            };

        let (items, container_bounds) =
            renderer.build_collection_items(&views, layout.as_ref(), ctx, existing_items, env);

        renderer.collection_caches.insert(
            cache_key,
            CollectionCache {
                views,
                layout,
                items,
                container_bounds,
                dispatch_ctx: ctx,
                dispatch_env: env.clone(),
                render_generation,
                _watch_guard: watch_guard,
            },
        );

        // Inside a retained capture, record a placement instead of baking the
        // items into the parent scene, so a later membership change patches in
        // isolation. Outside a capture, replay the items immediately.
        let draw = DynamicCollectionDraw {
            cache_key,
            base_transform: ctx.transform,
            base_hit_transform: ctx.hit_transform,
        };
        if renderer.dynamic_transform_capture_depth > 0 {
            renderer.flush_static_segment();
            renderer.draw_ops.push(DynamicDrawOp::Collection(draw));
        } else {
            let replay_ctx = ctx.with_identity_transforms(ctx.bounds);
            renderer.replay_dynamic_collection(replay_ctx, &draw);
        }
    }

    /// Reconciles the current membership of `views` against `existing` items:
    /// measures every live item, places them with `layout`, reuses the cached
    /// subtree of any unchanged-and-same-size id, dispatches new or resized
    /// items, and drops removed ones. Returns the reconciled items (in
    /// collection order) and the container's local bounds.
    fn build_collection_items(
        &mut self,
        views: &SharedAnyViews<AnyView>,
        layout: &dyn Layout,
        ctx: RenderContext,
        existing: Vec<CollectionItem>,
        env: &Environment,
    ) -> (Vec<CollectionItem>, vello::kurbo::Rect) {
        let count = views.len().get();

        // Materialize the live membership (ids + normalized views) in order.
        let mut ids = Vec::with_capacity(count);
        let mut live_views = Vec::with_capacity(count);
        for index in 0..count {
            ids.push(
                views
                    .get_id(index)
                    .unwrap_or_else(|| panic!("collection failed to provide id at index {index}")),
            );
            let view = views.get_view(index).unwrap_or_else(|| {
                panic!("collection failed to materialize view at index {index}")
            });
            live_views.push(normalize_layout_view(view, env));
        }

        // Measure and place every item with the container's layout. The measure
        // borrow of state ends before any item is dispatched.
        let offered = ctx.bounds;
        let (container_bounds, child_rects) = {
            let state = RefCell::new(&mut self.state);
            let mut subviews = Vec::with_capacity(count);
            for view in &live_views {
                subviews.push(HydroSubview::from_view(view, &state, env));
            }
            let refs: Vec<&dyn SubView> =
                subviews.iter().map(|view| view as &dyn SubView).collect();
            let proposal =
                ProposalSize::new(Some(offered.width() as f32), Some(offered.height() as f32));
            let layout_size = layout.size_that_fits(proposal, &refs);
            let stretch_axis = layout.stretch_axis();
            let width = if matches!(stretch_axis, StretchAxis::Horizontal | StretchAxis::Both) {
                offered.width() as f32
            } else {
                layout_size.width.min(offered.width() as f32)
            };
            let height = if matches!(stretch_axis, StretchAxis::Vertical | StretchAxis::Both) {
                offered.height() as f32
            } else {
                layout_size.height.min(offered.height() as f32)
            };
            let container = LayoutRect::from_size(LayoutSize::new(width, height));
            let child_rects = layout.place(container, &refs);
            let container_bounds =
                vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
            (container_bounds, child_rects)
        };

        let mut existing_by_id: BTreeMap<CollectionId, CollectionItem> =
            existing.into_iter().map(|item| (item.id, item)).collect();

        let mut items = Vec::with_capacity(count);
        for (index, view) in live_views.into_iter().enumerate() {
            let id = ids[index];
            let rect = child_rects[index];
            let local_bounds = vello::kurbo::Rect::new(
                0.0,
                0.0,
                f64::from(rect.width()),
                f64::from(rect.height()),
            );
            let base = vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y())));

            // Reuse an unchanged item's captured subtree when its placed size is
            // unchanged; only its placement transform may differ.
            let reused = existing_by_id
                .remove(&id)
                .filter(|item| size_near(item.bounds, local_bounds))
                .map(|item| item.subtree);
            let subtree = reused.unwrap_or_else(|| {
                let item_ctx = ctx.child(base, local_bounds);
                let item_local_ctx = item_ctx.with_identity_transforms(local_bounds);
                Self::render_dynamic_subtree_with_local_interactions(
                    self,
                    item_ctx,
                    item_local_ctx,
                    env,
                    view,
                )
            });

            items.push(CollectionItem {
                id,
                subtree,
                base_transform: base,
                base_hit_transform: base,
                bounds: local_bounds,
            });
        }
        // `existing_by_id` now holds the removed items; dropping them releases
        // their retained subtrees (and the watchers those subtrees held).

        (items, container_bounds)
    }

    /// Composites every item of a collection from its retained subtree, each at
    /// its placed transform. Mirrors [`Self::replay_dynamic_node_placement`]: the
    /// cache is taken out, replayed, and reinserted, so a later membership change
    /// (which only touches the changed items' subtrees) is picked up without
    /// re-walking the rest of the window.
    pub(super) fn replay_dynamic_collection(
        &mut self,
        parent_ctx: RenderContext,
        draw: &DynamicCollectionDraw,
    ) {
        let Some(cache) = self.collection_caches.remove(&draw.cache_key) else {
            return;
        };
        let container_transform = parent_ctx.transform * draw.base_transform;
        let container_hit = parent_ctx.hit_transform * draw.base_hit_transform;
        for item in &cache.items {
            let item_ctx = RenderContext::with_transforms(
                item.bounds,
                container_transform * item.base_transform,
                container_hit * item.base_hit_transform,
            );
            self.replay_dynamic_subtree(item_ctx, &item.subtree);
        }
        self.collection_caches.insert(draw.cache_key, cache);
    }

    /// Re-dispatches every dirty collection in isolation, reconciling only its
    /// changed items. Returns `false` if a reconcile changed the container's
    /// size (escalating to a structural rebuild so the surrounding layout can
    /// reflow), in which case the caller must rebuild instead of compositing.
    pub(super) fn patch_dirty_collections(&mut self) -> bool {
        let dirty = self.signals.take_dirty_collections();
        for cache_key in dirty {
            let Some(cache) = self.collection_caches.get_mut(&cache_key) else {
                continue;
            };
            let views = cache.views.clone();
            let ctx = cache.dispatch_ctx;
            let env = cache.dispatch_env.clone();
            let previous_container = cache.container_bounds;
            let existing = core::mem::take(&mut cache.items);
            // Vacate the boxed layout so it can be borrowed while `self` is
            // mutably re-borrowed by the reconcile, then restore it.
            let layout = core::mem::replace(&mut cache.layout, Box::new(NullLayout));

            // Re-dispatch new items under a retained capture so nested dynamic
            // draws inside them are captured, not baked (mirrors dynamic-node patch).
            self.enter_dynamic_capture_depths();
            let (items, container_bounds) =
                self.build_collection_items(&views, layout.as_ref(), ctx, existing, &env);
            self.exit_dynamic_capture_depths();

            let container_changed = !size_near(previous_container, container_bounds);
            if let Some(cache) = self.collection_caches.get_mut(&cache_key) {
                cache.layout = layout;
                cache.items = items;
                cache.container_bounds = container_bounds;
            }
            if container_changed {
                // The collection's size changed: the surrounding layout must
                // reflow, which only a structural rebuild can do.
                self.request_rebuild();
            }
            if self.signals.has_rebuild_request() {
                return false;
            }
        }
        true
    }

    /// Bumps the transform/morph/scroll capture depths so a re-dispatch inside a
    /// reactive patch captures nested dynamic draws instead of baking them.
    fn enter_dynamic_capture_depths(&mut self) {
        self.dynamic_transform_capture_depth = self
            .dynamic_transform_capture_depth
            .checked_add(1)
            .expect("hydrolysis collection patch transform capture depth overflow");
        self.dynamic_morph_capture_depth = self
            .dynamic_morph_capture_depth
            .checked_add(1)
            .expect("hydrolysis collection patch morph capture depth overflow");
        self.scroll_content_capture_depth = self
            .scroll_content_capture_depth
            .checked_add(1)
            .expect("hydrolysis collection patch scroll capture depth overflow");
    }

    fn exit_dynamic_capture_depths(&mut self) {
        self.scroll_content_capture_depth = self
            .scroll_content_capture_depth
            .checked_sub(1)
            .expect("hydrolysis collection patch scroll capture depth underflow");
        self.dynamic_morph_capture_depth = self
            .dynamic_morph_capture_depth
            .checked_sub(1)
            .expect("hydrolysis collection patch morph capture depth underflow");
        self.dynamic_transform_capture_depth = self
            .dynamic_transform_capture_depth
            .checked_sub(1)
            .expect("hydrolysis collection patch transform capture depth underflow");
    }

    /// Collects the active animation scalar keys reachable through every item of
    /// the collections in `subtree` (recursing into each item's subtree).
    pub(super) fn collect_collection_active_scalar_keys(
        &self,
        subtree: &DynamicSubtree,
        keys: &mut BTreeSet<AnimationKey>,
    ) {
        for draw in subtree.collection_draws() {
            if let Some(cache) = self.collection_caches.get(&draw.cache_key) {
                for item in &cache.items {
                    self.collect_subtree_active_scalar_keys(&item.subtree, keys);
                }
            }
        }
    }

    /// Whether every scroll view reachable through the collections in `subtree`
    /// can still be re-composited from its cache (recursing into items).
    pub(super) fn collection_scroll_draws_reusable(&self, subtree: &DynamicSubtree) -> bool {
        for draw in subtree.collection_draws() {
            if let Some(cache) = self.collection_caches.get(&draw.cache_key) {
                for item in &cache.items {
                    if !self.subtree_scroll_draws_reusable(&item.subtree) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Registers the membership watcher for a collection. The first fire is the
/// initial snapshot delivered on registration (during the capturing rebuild) and
/// is ignored; later real changes mark the collection dirty for an isolated
/// reconcile.
fn register_collection_watch(
    views: &SharedAnyViews<AnyView>,
    cache_key: usize,
    render_generation: u64,
    signals: &FrameSignals,
) -> BoxWatcherGuard {
    let signals = signals.clone();
    let first = Cell::new(true);
    views.watch(.., move |_ctx| {
        if first.replace(false) {
            return;
        }
        signals.mark_collection_dirty(cache_key, render_generation);
        signals.request_redraw();
    })
}

/// A zero-child layout used to temporarily vacate a cache's boxed layout while it
/// is borrowed for a patch reconcile. Never used to place anything.
#[derive(Debug)]
struct NullLayout;

impl Layout for NullLayout {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
    fn size_that_fits(&self, _proposal: ProposalSize, _children: &[&dyn SubView]) -> LayoutSize {
        LayoutSize::zero()
    }
    fn place(&self, _bounds: LayoutRect, children: &[&dyn SubView]) -> Vec<LayoutRect> {
        children
            .iter()
            .map(|_| LayoutRect::from_size(LayoutSize::zero()))
            .collect()
    }
}
