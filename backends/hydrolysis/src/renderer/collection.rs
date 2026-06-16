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
use crate::renderer::lazy::{LazyStackAxisConfig, lazy_stack_axis_config};
use nami::watcher::BoxWatcherGuard;
use crate::time::Instant;
use waterui_core::animation::Animation;
use waterui_core::id::{Id as RawId, SelfId};
use waterui_core::views::{AnyViews, SharedAnyViews, Views};
use waterui_layout::collection_transition::CollectionTransition;

/// The type-erased item identity used across every retained collection.
type CollectionId = SelfId<RawId>;

/// The membership-transition phase of a retained collection item. An item with a
/// transition fades and (along the stack axis) collapses in while `Entering` and
/// out while `Exiting`; `Stable` items render at full size and opacity. Without a
/// transition every item is `Stable`.
#[derive(Clone, Copy)]
enum EntryPhase {
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
            Self::Exiting(start) => {
                1.0 - animation.progress(now.saturating_duration_since(start))
            }
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
}

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
    /// Membership-transition phase. Always `Stable` when the collection has no
    /// transition configured.
    phase: EntryPhase,
}

/// The resolved transition timing for a collection: the animation curve and, for
/// a stack layout, the axis and spacing used to interpolate item placement while
/// items enter and exit.
#[derive(Clone)]
struct CollectionTransitionRuntime {
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

/// Retained state for one reactive collection, keyed by its controller slot
/// `cache_key` in [`HydrolysisRenderer::collection_caches`].
pub(crate) struct CollectionCache {
    /// The source collection. Kept across rebuilds so its id mapping is stable.
    views: SharedAnyViews<AnyView>,
    /// The collection's layout, used to place items on capture and reconcile.
    layout: Box<dyn Layout>,
    /// Items in display order, each holding its retained subtree + placement.
    /// With a transition this also holds still-`Exiting` items, anchored at their
    /// former position, until their fade-out completes.
    entries: Vec<CollectionItem>,
    /// Resolved membership transition, or `None` when the collection pops.
    transition: Option<CollectionTransitionRuntime>,
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

impl CollectionCache {
    /// Whether any item is mid enter/exit at `now` (so frames must keep coming).
    fn has_active_transition(&self, now: Instant) -> bool {
        self.transition.as_ref().is_some_and(|runtime| {
            self.entries
                .iter()
                .any(|entry| entry.phase.is_transitioning(now, &runtime.animation))
        })
    }
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

        // Resolve the opt-in membership transition (if any) for this collection.
        let transition = collection_transition_runtime(env, layout.as_ref());

        // Reuse the existing slot cache (keeping its stable id mapping + watch),
        // or register a fresh one on first encounter.
        let (views, watch_guard, render_generation, existing_items) =
            match renderer.collection_caches.remove(&cache_key) {
                Some(cache) => (
                    cache.views,
                    cache._watch_guard,
                    cache.render_generation,
                    cache.entries,
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

        let (entries, container_bounds) = renderer.build_collection_items(
            &views,
            layout.as_ref(),
            ctx,
            existing_items,
            transition.as_ref(),
            env,
        );

        renderer.collection_caches.insert(
            cache_key,
            CollectionCache {
                views,
                layout,
                entries,
                transition,
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

    /// Reconciles the current membership of `views` against `existing` entries:
    /// measures every live item, places them with `layout`, reuses the cached
    /// subtree of any unchanged-and-same-size id, dispatches new or resized
    /// items, and either drops removed ones (no transition) or keeps them in the
    /// display order as `Exiting` (with a transition) so they collapse out in
    /// place. Returns the reconciled entries (in display order) and the
    /// container's settled local bounds.
    fn build_collection_items(
        &mut self,
        views: &SharedAnyViews<AnyView>,
        layout: &dyn Layout,
        ctx: RenderContext,
        existing: Vec<CollectionItem>,
        transition: Option<&CollectionTransitionRuntime>,
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

        let now = self.frame_instant;
        let live_set: BTreeSet<CollectionId> = ids.iter().copied().collect();
        // The first materialization of a collection shows its initial membership
        // at rest — only items added by a *later* change animate in.
        let initial = existing.is_empty();

        // Partition the previous display order into items still live (reusable,
        // keyed by id) and items no longer live. With a transition the latter keep
        // collapsing out, anchored to the live id they followed; without one they
        // are dropped here (releasing their retained subtrees).
        let mut reuse_by_id: BTreeMap<CollectionId, CollectionItem> = BTreeMap::new();
        let mut head_dead: Vec<CollectionItem> = Vec::new();
        let mut dead_after: BTreeMap<CollectionId, Vec<CollectionItem>> = BTreeMap::new();
        {
            let mut last_live: Option<CollectionId> = None;
            for item in existing {
                if live_set.contains(&item.id) {
                    last_live = Some(item.id);
                    reuse_by_id.insert(item.id, item);
                } else if transition.is_some() {
                    match last_live {
                        Some(anchor) => dead_after.entry(anchor).or_default().push(item),
                        None => head_dead.push(item),
                    }
                }
            }
        }

        let begin_exit = |mut item: CollectionItem| -> CollectionItem {
            if !matches!(item.phase, EntryPhase::Exiting(_)) {
                item.phase = EntryPhase::Exiting(now);
            }
            item
        };

        let mut entries = Vec::with_capacity(count + head_dead.len());
        for item in head_dead {
            entries.push(begin_exit(item));
        }
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

            let entry = match reuse_by_id.remove(&id) {
                // Unchanged size: reuse the captured subtree; only the placement
                // (and, with a transition, the phase) may change.
                Some(mut prev) if size_near(prev.bounds, local_bounds) => {
                    // The reused subtree is not re-dispatched, so re-register the
                    // `Dynamic` node identities it references as active this frame;
                    // otherwise `finish_rebuild_frame` prunes them and the reused
                    // placements composite stale content.
                    self.mark_subtree_dynamic_identities_active(&prev.subtree);
                    prev.base_transform = base;
                    prev.base_hit_transform = base;
                    prev.bounds = local_bounds;
                    prev.phase = next_live_phase(transition, prev.phase, now);
                    prev
                }
                // New id, or an existing id whose size changed: dispatch fresh.
                other => {
                    let previous_phase = other.map(|item| item.phase);
                    let item_ctx = ctx.child(base, local_bounds);
                    let item_local_ctx = item_ctx.with_identity_transforms(local_bounds);
                    let subtree = Self::render_dynamic_subtree_with_local_interactions(
                        self,
                        item_ctx,
                        item_local_ctx,
                        env,
                        view,
                    );
                    let phase = match previous_phase {
                        // A resized existing item keeps its phase; a brand-new id
                        // enters (unless it is part of the initial membership).
                        Some(phase) => next_live_phase(transition, phase, now),
                        None if transition.is_some() && !initial => EntryPhase::Entering(now),
                        None => EntryPhase::Stable,
                    };
                    CollectionItem {
                        id,
                        subtree,
                        base_transform: base,
                        base_hit_transform: base,
                        bounds: local_bounds,
                        phase,
                    }
                }
            };
            entries.push(entry);
            if let Some(dead) = dead_after.remove(&id) {
                for item in dead {
                    entries.push(begin_exit(item));
                }
            }
        }
        // Any dead anchored to a live id that did not reappear (defensive): keep
        // them so their fade-out still completes rather than popping.
        for item in dead_after.into_values().flatten() {
            entries.push(begin_exit(item));
        }

        (entries, container_bounds)
    }

    /// Composites every entry of a collection from its retained subtree. Mirrors
    /// [`Self::replay_dynamic_node_placement`]: the cache is taken out, replayed,
    /// and reinserted, so a later membership change (which only touches the
    /// changed items' subtrees) is picked up without re-walking the rest of the
    /// window. While a transition is in flight every entry is re-placed and
    /// faded from its presence factor at the current frame instant; otherwise the
    /// cached placements are replayed directly.
    pub(super) fn replay_dynamic_collection(
        &mut self,
        parent_ctx: RenderContext,
        draw: &DynamicCollectionDraw,
    ) {
        let Some(mut cache) = self.collection_caches.remove(&draw.cache_key) else {
            return;
        };
        let container_transform = parent_ctx.transform * draw.base_transform;
        let container_hit = parent_ctx.hit_transform * draw.base_hit_transform;
        let now = self.frame_instant;

        if let Some(runtime) = cache.transition.clone() {
            // Settle finished enters and drop finished exits *before* choosing how
            // to render. The fast path must never composite a finished-but-still-
            // present exiting item at its stale placement — that overlaps the
            // reflowed live items (and a rebuild interrupting the transition can
            // strand such items there permanently). After this, any non-`Stable`
            // entry is genuinely mid-transition.
            let before = cache.entries.len();
            cache
                .entries
                .retain(|entry| !entry.phase.is_finished_exit(now, &runtime.animation));
            for entry in &mut cache.entries {
                if let EntryPhase::Entering(start) = entry.phase
                    && runtime.animation.is_complete(now.saturating_duration_since(start))
                {
                    entry.phase = EntryPhase::Stable;
                }
            }
            let dropped = before != cache.entries.len();
            let transitioning = cache
                .entries
                .iter()
                .any(|entry| !matches!(entry.phase, EntryPhase::Stable));
            if transitioning {
                self.replay_transitioning_collection(
                    &cache,
                    &runtime,
                    container_transform,
                    container_hit,
                    now,
                );
            } else {
                self.replay_collection_stable(&cache, container_transform, container_hit);
                if dropped {
                    // The transition just finished: reflow the surrounding layout
                    // to the collection's settled size.
                    self.request_rebuild();
                }
            }
        } else {
            self.replay_collection_stable(&cache, container_transform, container_hit);
        }
        self.collection_caches.insert(draw.cache_key, cache);
    }

    /// Composites every entry at its cached placement (no transition in flight).
    fn replay_collection_stable(
        &mut self,
        cache: &CollectionCache,
        container_transform: vello::kurbo::Affine,
        container_hit: vello::kurbo::Affine,
    ) {
        for entry in &cache.entries {
            let item_ctx = RenderContext::with_transforms(
                entry.bounds,
                container_transform * entry.base_transform,
                container_hit * entry.base_hit_transform,
            );
            self.replay_dynamic_subtree(item_ctx, &entry.subtree);
        }
    }

    /// Composites a collection mid-transition. For a stack layout each entry is
    /// re-stacked using its presence-scaled extent (so entering/exiting items
    /// grow/collapse along the axis and their neighbours slide), clipped to that
    /// extent and faded by the same factor. For a non-stack layout entries keep
    /// their captured placement and only cross-fade.
    fn replay_transitioning_collection(
        &mut self,
        cache: &CollectionCache,
        runtime: &CollectionTransitionRuntime,
        container_transform: vello::kurbo::Affine,
        container_hit: vello::kurbo::Affine,
        now: Instant,
    ) {
        let Some(axis) = runtime.axis else {
            for entry in &cache.entries {
                let alpha = entry.phase.factor(now, &runtime.animation).clamp(0.0, 1.0);
                if alpha <= f32::EPSILON {
                    continue;
                }
                let item_transform = container_transform * entry.base_transform;
                let item_hit = container_hit * entry.base_hit_transform;
                let item_ctx =
                    RenderContext::with_transforms(entry.bounds, item_transform, item_hit);
                let suppress = !matches!(entry.phase, EntryPhase::Stable);
                self.replay_collection_entry(
                    item_ctx,
                    &entry.subtree,
                    alpha,
                    entry.bounds,
                    suppress,
                );
            }
            return;
        };

        let mut main = 0.0_f64;
        for entry in &cache.entries {
            let factor = f64::from(entry.phase.factor(now, &runtime.animation).clamp(0.0, 1.0));
            if factor <= f64::EPSILON {
                continue;
            }
            let (cross_extent, main_extent) = if axis.vertical {
                (entry.bounds.width(), entry.bounds.height())
            } else {
                (entry.bounds.height(), entry.bounds.width())
            };
            let cross_offset = if axis.vertical {
                entry.base_transform.translation().x
            } else {
                entry.base_transform.translation().y
            };
            let effective_main = main_extent * factor;
            let position = if axis.vertical {
                vello::kurbo::Affine::translate((cross_offset, main))
            } else {
                vello::kurbo::Affine::translate((main, cross_offset))
            };
            let item_transform = container_transform * position;
            let item_hit = container_hit * position;
            let item_ctx = RenderContext::with_transforms(entry.bounds, item_transform, item_hit);
            #[allow(clippy::cast_possible_truncation)]
            let alpha = factor as f32;
            let clip = if axis.vertical {
                vello::kurbo::Rect::new(0.0, 0.0, cross_extent, effective_main)
            } else {
                vello::kurbo::Rect::new(0.0, 0.0, effective_main, cross_extent)
            };
            let suppress = !matches!(entry.phase, EntryPhase::Stable);
            self.replay_collection_entry(item_ctx, &entry.subtree, alpha, clip, suppress);
            main += effective_main + axis.spacing * factor;
        }
    }

    /// Replays one collection entry, wrapping it in an opacity+clip layer while it
    /// is transitioning (`alpha < 1`) and replaying it directly otherwise. A
    /// transitioning entry is kept out of the accessibility tree
    /// (`suppress_accessibility`): the tree reflects the collection's settled
    /// (logical) membership, while the fade/collapse is purely visual.
    fn replay_collection_entry(
        &mut self,
        item_ctx: RenderContext,
        subtree: &DynamicSubtree,
        alpha: f32,
        clip: vello::kurbo::Rect,
        suppress_accessibility: bool,
    ) {
        #[cfg(feature = "accessibility")]
        if suppress_accessibility {
            self.push_accessibility_suppression();
        }
        if alpha >= 1.0 {
            self.replay_dynamic_subtree(item_ctx, subtree);
        } else {
            self.push_layer_rect(alpha, item_ctx.transform, clip);
            let previous_opacity = self.hit_test.hit_test_opacity;
            self.hit_test.hit_test_opacity = previous_opacity * alpha;
            self.replay_dynamic_subtree(item_ctx, subtree);
            self.hit_test.hit_test_opacity = previous_opacity;
            self.pop_layer();
        }
        #[cfg(feature = "accessibility")]
        if suppress_accessibility {
            self.pop_accessibility_suppression();
        }
        #[cfg(not(feature = "accessibility"))]
        let _ = suppress_accessibility;
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
            let transition = cache.transition.clone();
            let previous_container = cache.container_bounds;
            let existing = core::mem::take(&mut cache.entries);
            // Vacate the boxed layout so it can be borrowed while `self` is
            // mutably re-borrowed by the reconcile, then restore it.
            let layout = core::mem::replace(&mut cache.layout, Box::new(NullLayout));

            // Re-dispatch new items under a retained capture so nested dynamic
            // draws inside them are captured, not baked (mirrors dynamic-node patch).
            self.enter_dynamic_capture_depths();
            let (entries, container_bounds) = self.build_collection_items(
                &views,
                layout.as_ref(),
                ctx,
                existing,
                transition.as_ref(),
                &env,
            );
            self.exit_dynamic_capture_depths();

            // A still-running enter/exit keeps the visual size changing each frame;
            // only escalate to a structural reflow once the membership has settled.
            let settled_size_changed = !size_near(previous_container, container_bounds)
                && !entries.iter().any(|entry| {
                    transition.as_ref().is_some_and(|runtime| {
                        entry.phase.is_transitioning(self.frame_instant, &runtime.animation)
                    })
                });
            if let Some(cache) = self.collection_caches.get_mut(&cache_key) {
                cache.layout = layout;
                cache.entries = entries;
                cache.container_bounds = container_bounds;
            }
            let container_changed = settled_size_changed;
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

    /// Collects every `Dynamic` node identity reachable through the items of the
    /// collections in `subtree` (recursing into each item's subtree). The
    /// collection counterpart of [`Self::collect_subtree_dynamic_identities`],
    /// kept here because [`CollectionCache`]'s items are private to this module.
    pub(super) fn collect_collection_dynamic_identities(
        &self,
        subtree: &DynamicSubtree,
        out: &mut Vec<usize>,
    ) {
        for draw in subtree.collection_draws() {
            if let Some(cache) = self.collection_caches.get(&draw.cache_key) {
                for entry in &cache.entries {
                    self.collect_subtree_dynamic_identities(&entry.subtree, out);
                }
            }
        }
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
                for entry in &cache.entries {
                    self.collect_subtree_active_scalar_keys(&entry.subtree, keys);
                }
            }
        }
    }

    /// Whether every scroll view reachable through the collections in `subtree`
    /// can still be re-composited from its cache (recursing into items).
    pub(super) fn collection_scroll_draws_reusable(&self, subtree: &DynamicSubtree) -> bool {
        for draw in subtree.collection_draws() {
            if let Some(cache) = self.collection_caches.get(&draw.cache_key) {
                for entry in &cache.entries {
                    if !self.subtree_scroll_draws_reusable(&entry.subtree) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Whether any reactive collection has an in-flight enter/exit transition at
    /// `now`. Drives the runner to keep issuing animation frames (and the retained
    /// window frame to refresh, re-sampling each item's presence factor) until
    /// every membership transition settles.
    pub(crate) fn collection_transitions_active(&self, now: Instant) -> bool {
        self.collection_caches
            .values()
            .any(|cache| cache.has_active_transition(now))
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

/// The phase a reused/resized live item should carry after a reconcile: `Stable`
/// without a transition, a fresh enter when it was leaving, otherwise its current
/// phase (so an in-flight enter keeps running).
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

/// Resolves the opt-in [`CollectionTransition`] for a collection from the
/// environment, pairing it with the stack axis/spacing when the layout is a
/// vertical or horizontal stack (so items collapse along the axis; other layouts
/// cross-fade in place).
fn collection_transition_runtime(
    env: &Environment,
    layout: &dyn Layout,
) -> Option<CollectionTransitionRuntime> {
    let transition = env.get::<CollectionTransition>()?;
    let axis = lazy_stack_axis_config(layout).map(|config| match config {
        LazyStackAxisConfig::Vertical { spacing, .. } => TransitionAxis {
            vertical: true,
            spacing,
        },
        LazyStackAxisConfig::Horizontal { spacing, .. } => TransitionAxis {
            vertical: false,
            spacing,
        },
    });
    Some(CollectionTransitionRuntime {
        animation: transition.animation.clone(),
        axis,
    })
}
