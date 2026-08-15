use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::gesture::GestureTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, VisibleSubviewCache,
    WidgetRenderContext, local_interaction_state, materialize_list_item, measure_list_intrinsic,
    measure_list_item_row_height, measure_view_intrinsic, transformed_rect,
};
use crate::scroll::ScrollHandle;
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use waterui::component::list::{ListConfig, Move};
use waterui::gesture::{DragEvent, DragGesture, Gesture, GesturePhase};
use waterui_core::handler::{BoxedAction, boxed_action};
use waterui_core::id::{Id as RawId, SelfId};
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::views::Views;
use waterui_core::{Environment, Native};
use waterui_layout::scroll::Axis as ScrollAxis;

use crate::renderer::lazy::VirtualExtentIndex;
use crate::widgets::{draw_scroll_indicators, widget_theme};
use nami::watcher::BoxWatcherGuard;

/// The stable per-row id used to key the retained content sub-view cache, matching
/// the id `ListConfig::contents` (a `SharedAnyViews<ListItem>`) yields per index.
type ListItemId = SelfId<RawId>;

#[derive(Clone, Copy)]
struct ListViewportAnchor {
    id: ListItemId,
    index: usize,
    offset_within_row: f64,
}

/// Fraction of the row's width a swipe must cross to dismiss on release.
/// Compose's `SwipeToDismissBoxDefaults.positionalThreshold` is
/// `{ distance -> distance * 0.5f }`.
const SWIPE_DISMISS_POSITIONAL_THRESHOLD: f64 = 0.5;

/// Time constant (seconds) of the exponential spring-back when a swipe is
/// released short of the threshold, and of the settle after a committed
/// dismiss. Matches the feel of the smooth-scroll approach used elsewhere.
const SWIPE_SETTLE_TAU: f64 = 0.09;

/// Offset below which a settling swipe snaps to rest and stops requesting
/// frames.
const SWIPE_SETTLE_EPSILON: f64 = 0.5;

/// Elevation handed to the theme while a row is lifted for reordering. Material
/// raises a dragged list item to level 3.
const REORDER_LIFT_ELEVATION: f64 = 3.0;

/// How much of a programmatic jump is animated, in viewports. Anything further
/// is closed instantly first, so the glide stays legible and a jump across a
/// huge dataset cannot materialize every viewport it would otherwise cross.
const MAX_ANIMATED_JUMP_VIEWPORTS: f64 = 1.0;

/// Distance the pointer must travel before a row drag is recognized, matching
/// Android's `ViewConfiguration` touch slop. Without it a row could not be
/// tapped at all: every press would immediately read as a swipe.
const ROW_DRAG_SLOP: f32 = 8.0;

/// This frame's identity and geometry for one row, shared with that row's
/// retained gesture recognizers.
#[derive(Clone, Copy)]
struct RowBinding {
    /// The row's current position in the collection.
    index: usize,
    /// Row width, against which the swipe-dismiss threshold is measured.
    width: f64,
    /// Row height, the slot size a reorder drag steps by.
    height: f64,
    /// Collection length, clamping where a reorder can land.
    total_rows: usize,
}

impl RowBinding {
    /// Stand-in stored when a row's binding cell is created; every registration
    /// path overwrites it with real geometry in the same frame.
    const PLACEHOLDER: Self = Self {
        index: 0,
        width: 0.0,
        height: 0.0,
        total_rows: 0,
    };
}

/// A row being swiped horizontally, or springing back after release.
#[derive(Clone, Copy)]
struct RowSwipe {
    id: ListItemId,
    /// Current horizontal displacement of the row content, in logical pixels.
    offset: f64,
    /// Set once the pointer is released: the offset is then eased to `target`
    /// instead of tracking the pointer.
    settling: bool,
    /// Where a settling swipe is heading — `0.0` to spring back, or off-screen
    /// once the dismiss is committed.
    target: f64,
}

/// A row lifted by its move handle and dragged to a new position.
#[derive(Clone, Copy)]
struct RowReorder {
    id: ListItemId,
    /// Index the row occupied when the drag began.
    from: usize,
    /// Index the row would land on if released now.
    to: usize,
    /// Vertical displacement from the row's resting position.
    dy: f64,
}

/// Retained state for a list `Widget` node: the consumed config plus a per-widget
/// cache of the visible rows' content sub-views, keyed by stable row id. Only the
/// rows in the current visible window are built and retained (evicted once they
/// scroll out), so the list stays virtualized — cost is bounded by visible rows.
pub(crate) struct ListRenderState {
    pub(crate) config: ListConfig,
    /// Estimated/measured row extents belong to this list, not to its render
    /// position in a backend-global slot array.
    extent_index: RefCell<VirtualExtentIndex>,
    /// The scroll offset belongs to this semantic list node.
    scroll: RefCell<Option<ScrollHandle>>,
    /// Content sub-views for the rows currently in view, keyed by stable row id so a
    /// steady scroll reuses each visible row's node (keeping its reactive content
    /// live) and only builds rows entering the window.
    item_cache: RefCell<VisibleSubviewCache<ListItemId>>,
    /// A membership change invalidates index-based extents, including reorder
    /// operations whose collection length stays unchanged.
    rows_dirty: Rc<Cell<bool>>,
    /// Last programmatic scroll generation applied to this semantic list.
    applied_scroll_generation: Cell<i32>,
    /// A requested index stays pending until its measured row intersects the
    /// concrete viewport. This is required when estimated and measured row
    /// heights differ, especially for a jump to the final row.
    pending_scroll: Cell<Option<(i32, usize)>>,
    /// Stable top-row identity and intra-row offset from the previous frame.
    /// Membership changes use this anchor so delete/move operations do not
    /// visibly jump the viewport.
    viewport_anchor: Cell<Option<ListViewportAnchor>>,
    /// Concrete offset resolved from `viewport_anchor` after an extent reset.
    pending_membership_offset: Cell<Option<f64>>,
    /// A backend move action keeps the viewport's index fixed for the next
    /// membership reconcile so the moved row visibly changes position.
    preserve_anchor_index_once: Cell<bool>,
    /// Gesture recognizers for the visible rows, keyed by stable row id.
    ///
    /// The engine clears its target list every frame, so a recognizer that was
    /// only ever built by `register_target` would forget an in-flight drag on
    /// the very next frame. Retaining the target here and re-registering it at
    /// the row's fresh bounds keeps the state machine alive across frames,
    /// which is what makes a swipe or a reorder drag survive scrolling.
    row_gestures: RefCell<std::collections::HashMap<ListItemId, RowGestures>>,
    /// The row currently being swiped, or springing back after release.
    swipe: Rc<Cell<Option<RowSwipe>>>,
    /// The row currently lifted for reordering.
    reorder: Rc<Cell<Option<RowReorder>>>,
    /// Last frame time a settling swipe was advanced at.
    swipe_last_tick: Cell<Option<crate::time::Instant>>,
    /// Collection membership watcher.
    _guard: BoxWatcherGuard,
}

/// Retained gesture targets for one row, plus the live binding they read.
struct RowGestures {
    /// This frame's index and geometry for the row.
    binding: Rc<Cell<RowBinding>>,
    /// Horizontal swipe-to-dismiss over the whole row.
    swipe: Option<GestureTarget>,
    /// Vertical reorder drag on the row's move handle.
    reorder: Option<GestureTarget>,
}

impl ListRenderState {
    pub(crate) fn from_config(config: ListConfig, renderer: &HydrolysisRenderer) -> Self {
        let rows_dirty = Rc::new(Cell::new(true));
        let rows_dirty_for_watch = Rc::clone(&rows_dirty);
        let signals = renderer.frame_signals();
        let guard = config.contents.watch(.., move |_change| {
            rows_dirty_for_watch.set(true);
            signals.request_refresh();
        });
        Self {
            config,
            extent_index: RefCell::new(VirtualExtentIndex::default()),
            scroll: RefCell::new(None),
            item_cache: RefCell::new(VisibleSubviewCache::new()),
            rows_dirty,
            applied_scroll_generation: Cell::new(0),
            pending_scroll: Cell::new(None),
            viewport_anchor: Cell::new(None),
            pending_membership_offset: Cell::new(None),
            preserve_anchor_index_once: Cell::new(false),
            row_gestures: RefCell::new(std::collections::HashMap::new()),
            swipe: Rc::new(Cell::new(None)),
            reorder: Rc::new(Cell::new(None)),
            swipe_last_tick: Cell::new(None),
            _guard: guard,
        }
    }

    /// Eases a released swipe toward its resting or dismissed position and
    /// reports whether more frames are needed. A swipe still tracking the
    /// pointer is left alone — only a `settling` one is driven here.
    fn advance_swipe_settle(&self, now: crate::time::Instant) -> bool {
        let Some(mut swipe) = self.swipe.get() else {
            self.swipe_last_tick.set(None);
            return false;
        };
        if !swipe.settling {
            // The pointer owns the offset; restart the clock so the first
            // settle step after release measures from release, not from press.
            self.swipe_last_tick.set(None);
            return false;
        }
        let dt = self
            .swipe_last_tick
            .get()
            .map_or(0.0, |last| now.duration_since(last).as_secs_f64());
        self.swipe_last_tick.set(Some(now));
        let blend = 1.0 - (-dt / SWIPE_SETTLE_TAU).exp();
        swipe.offset = (swipe.target - swipe.offset).mul_add(blend, swipe.offset);
        if (swipe.target - swipe.offset).abs() < SWIPE_SETTLE_EPSILON {
            self.swipe.set(None);
            self.swipe_last_tick.set(None);
            return false;
        }
        self.swipe.set(Some(swipe));
        true
    }

    /// Horizontal displacement to draw row `id` at, if it is the swiped row.
    fn swipe_offset_for(&self, id: ListItemId) -> f64 {
        self.swipe
            .get()
            .filter(|swipe| swipe.id == id)
            .map_or(0.0, |swipe| swipe.offset)
    }

    /// Vertical displacement to draw the row at `index` at, given any lifted
    /// row. The lifted row follows the pointer; every row between its origin
    /// and its prospective destination slides one slot to make room.
    fn reorder_offset_for(&self, index: usize, id: ListItemId, row_height: f64) -> f64 {
        let Some(reorder) = self.reorder.get() else {
            return 0.0;
        };
        if reorder.id == id {
            return reorder.dy;
        }
        if reorder.to > reorder.from && index > reorder.from && index <= reorder.to {
            -row_height
        } else if reorder.to < reorder.from && index >= reorder.to && index < reorder.from {
            row_height
        } else {
            0.0
        }
    }

    fn prepare_rows(&self, len: usize, estimate: f64) {
        let dirty = self.rows_dirty.replace(false);
        if dirty || !self.extent_index.borrow().matches(len, estimate, 0.0) {
            self.extent_index.borrow_mut().reset(len, estimate, 0.0);
            let preserve_anchor_index = self.preserve_anchor_index_once.replace(false);
            let membership_offset = self.viewport_anchor.get().and_then(|anchor| {
                if len == 0 {
                    return None;
                }
                let index = if preserve_anchor_index {
                    anchor.index.min(len - 1)
                } else if self.config.contents.get_id(anchor.index) == Some(anchor.id) {
                    anchor.index
                } else {
                    (0..len)
                        .find(|index| self.config.contents.get_id(*index) == Some(anchor.id))
                        .unwrap_or_else(|| anchor.index.min(len - 1))
                };
                Some(
                    self.extent_index.borrow().offset_of(index)
                        + anchor.offset_within_row.min(estimate),
                )
            });
            self.pending_membership_offset.set(membership_offset);
        }
    }

    fn bind_scroll(
        &self,
        viewport_width: f64,
        viewport_height: f64,
        content_height: f64,
    ) -> ScrollHandle {
        let mut scroll = self.scroll.borrow_mut();
        if let Some(handle) = scroll.as_mut() {
            handle.rebind(
                ScrollAxis::Vertical,
                viewport_width,
                viewport_height,
                viewport_width,
                content_height,
            )
        } else {
            let handle = ScrollHandle::new(
                ScrollAxis::Vertical,
                viewport_width,
                viewport_height,
                viewport_width,
                content_height,
            );
            *scroll = Some(handle.clone());
            handle
        }
    }

    fn apply_scroll_request(
        &self,
        renderer: &mut HydrolysisRenderer,
        handle: &ScrollHandle,
        row_count: usize,
    ) {
        let Some(controller) = &self.config.scroll_controller else {
            return;
        };
        let generation = renderer.read_signal(&controller.generation());
        if generation != self.applied_scroll_generation.get()
            && self
                .pending_scroll
                .get()
                .is_none_or(|(pending_generation, _)| pending_generation != generation)
        {
            let index = renderer.read_signal(&controller.target());
            assert!(
                index < row_count,
                "List scroll target {index} exceeds collection length {row_count}"
            );
            self.pending_scroll.set(Some((generation, index)));
        }
        let Some((pending_generation, index)) = self.pending_scroll.get() else {
            return;
        };
        assert!(
            index < row_count,
            "List scroll target {index} exceeds collection length {row_count}"
        );
        // Ease toward the row rather than teleporting. Re-issuing the target
        // every frame is what keeps a virtualized jump accurate: rows measured
        // while the glide passes over them move `offset_of(index)`, so the
        // destination is refined until the animation actually settles.
        let offset = self.extent_index.borrow().offset_of(index);
        let metrics = handle.metrics();
        let reachable = metrics.viewport_height * MAX_ANIMATED_JUMP_VIEWPORTS;
        let distance = offset - metrics.offset_y;
        if distance.abs() > reachable {
            // A jump across a 100k-row dataset must not be animated the whole
            // way: gliding over it would materialize every viewport in between
            // and read as a blur anyway. Close the gap instantly to just short
            // of the target, then animate only the final approach — the same
            // shape as `LinearSmoothScroller` on a distant target.
            let approach = offset - reachable.copysign(distance);
            let _ = handle.scroll_to(0.0, approach);
        }
        let _ = handle.scroll_to_animated(0.0, offset);
        let extent_index = self.extent_index.borrow();
        let Some(extent) = extent_index.measured(index) else {
            return;
        };
        let metrics = handle.metrics();
        let row_start = extent_index.offset_of(index);
        let row_end = row_start + extent;
        let viewport_end = metrics.offset_y + metrics.viewport_height;
        let row_visible = row_end > metrics.offset_y && row_start < viewport_end;
        if row_visible && !handle.is_smooth_scrolling() {
            self.applied_scroll_generation.set(pending_generation);
            self.pending_scroll.set(None);
        }
    }

    fn apply_membership_anchor(&self, handle: &ScrollHandle) {
        if let Some(offset) = self.pending_membership_offset.take() {
            let _ = handle.scroll_to(0.0, offset);
        }
    }

    fn record_viewport_anchor(&self, metrics: crate::scroll::ScrollMetrics, row_count: usize) {
        let window = self
            .extent_index
            .borrow()
            .visible_window(metrics.offset_y, metrics.offset_y + metrics.viewport_height);
        if window.start >= row_count {
            self.viewport_anchor.set(None);
            return;
        }
        let id = self
            .config
            .contents
            .get_id(window.start)
            .unwrap_or_else(|| panic!("hydrolysis List item {} has no stable id", window.start));
        self.viewport_anchor.set(Some(ListViewportAnchor {
            id,
            index: window.start,
            offset_within_row: (metrics.offset_y - window.leading_offset).max(0.0),
        }));
    }
}

impl HydroNativeView for Native<ListConfig> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_list_intrinsic(view.as_inner(), state, env)
    }
}

/// Emits a list's accessibility tree from its node-owned retained state.
pub(crate) fn list_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    state: &Rc<RefCell<ListRenderState>>,
    env: &Environment,
) {
    let state = state.borrow();
    let list = &state.config;
    let row_count_signal = list.contents.len();
    let row_count = renderer.read_signal(&row_count_signal);
    let list_metrics = crate::widgets::widget_theme(env).list_metrics();
    state.prepare_rows(row_count, list_metrics.one_line_row_height);
    let viewport = ctx.bounds;
    let content_height = state
        .extent_index
        .borrow()
        .total_extent()
        .max(viewport.height());
    let handle = state.bind_scroll(viewport.width(), viewport.height(), content_height);
    state.apply_membership_anchor(&handle);
    state.apply_scroll_request(renderer, &handle, row_count);
    #[cfg(feature = "accessibility")]
    {
        let metrics = handle.metrics();
        let window = state
            .extent_index
            .borrow()
            .visible_window(metrics.offset_y, metrics.offset_y + viewport.height());
        let mut list_node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::List),
        );
        let list_label = renderer.resolve_accessibility_label(env, None);
        if let Some(label) = list_label {
            list_node.set_label(label);
        }
        list_node.set_scroll_y(metrics.offset_y);
        list_node.set_scroll_y_min(0.0);
        list_node.set_scroll_y_max(metrics.max_y);
        list_node.add_action(AccessibilityAction::ScrollUp);
        list_node.add_action(AccessibilityAction::ScrollDown);
        let mut y = viewport.y0 - metrics.offset_y + window.leading_offset;
        for index in window.start..window.end {
            let row_env = env.clone();
            let item = materialize_list_item(&list.contents, index, &row_env);
            let row_height = {
                let cached_extent = state.extent_index.borrow().measured(index);
                if let Some(extent) = cached_extent {
                    extent
                } else {
                    let extent =
                        measure_list_item_row_height(&item, renderer.state_mut(), &row_env);
                    state.extent_index.borrow_mut().set_measured(index, extent);
                    extent
                }
            };
            let row_rect = vello::kurbo::Rect::new(viewport.x0, y, viewport.x1, y + row_height);
            y += row_height;
            if row_rect.y1 <= viewport.y0 || row_rect.y0 >= viewport.y1 {
                continue;
            }
            let mut row_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ListItem),
            );
            let default_label = renderer.accessibility_label_from_view(&item.content, &row_env);
            let label = renderer.resolve_accessibility_label(&row_env, default_label);
            if let Some(label) = label {
                row_node.set_label(label);
            }
            row_node.add_action(AccessibilityAction::Focus);
            let row_id = list
                .contents
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis list row {index} has no stable identity"));
            if let Some(row_node_id) = renderer.register_accessibility_child_node_with_key(
                i64::from(i32::from(*row_id)),
                row_node,
                transformed_rect(ctx.hit_transform, row_rect),
                &row_env,
                None,
            ) {
                list_node.push_child(row_node_id);
            }
        }
        let _ = renderer.register_accessibility_node(
            list_node,
            transformed_rect(ctx.hit_transform, viewport),
            env,
            Some(AccessibilityActionTarget::Scroll {
                handle: handle.clone(),
                axis: ScrollAxis::Vertical,
            }),
        );
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = handle;
    }
}

/// Measures a list leaf from its config (intrinsic-sized; proposal-independent).
pub(crate) fn measure_list_node(
    list: &ListConfig,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    ViewDimensions::new(measure_list_intrinsic(list, state, env))
}

/// Renders a retained list leaf every flush.
pub(crate) fn render_list_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ListRenderState>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().push_accessibility_suppression();
    }
    {
        let render_ctx = ctx.render_context();
        list_accessibility(ctx.renderer_mut(), render_ctx, state, env);
    }
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().pop_accessibility_suppression();
    }
    render_list_parts(ctx, state, env);
}

pub(crate) fn render_list_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ListRenderState>>,
    env: &Environment,
) {
    let (editing, row_count_signal, contents) = {
        let list = &state.borrow().config;
        (
            list.editing.clone(),
            list.contents.len(),
            list.contents.clone(),
        )
    };
    let editing = ctx.renderer_mut().read_signal(&editing);
    let row_count = ctx.renderer_mut().read_signal(&row_count_signal);
    let list_metrics = widget_theme(env).list_metrics();
    state
        .borrow()
        .prepare_rows(row_count, list_metrics.one_line_row_height);

    let viewport = ctx.bounds;
    let content_height = state
        .borrow()
        .extent_index
        .borrow()
        .total_extent()
        .max(viewport.height());
    let handle = state
        .borrow()
        .bind_scroll(viewport.width(), viewport.height(), content_height);
    state.borrow().apply_membership_anchor(&handle);
    state
        .borrow()
        .apply_scroll_request(ctx.renderer_mut(), &handle, row_count);
    let mut metrics = handle.metrics();
    let needs_viewport_clip = metrics.max_y > 0.0;
    if needs_viewport_clip {
        ctx.push_layer_rect(1.0, viewport);
    }

    let window = state
        .borrow()
        .extent_index
        .borrow()
        .visible_window(metrics.offset_y, metrics.offset_y + viewport.height());
    // The delete/move handlers (`Option<Box<dyn Fn>>`) stay owned by the retained
    // config; per-row tap targets invoke them through the shared cell so they are
    // reused every flush instead of being consumed.
    let has_delete = state.borrow().config.on_delete.is_some();
    let has_move = state.borrow().config.on_move.is_some();
    let total_rows = row_count;
    // A released swipe eases home (or off-screen) on the redraw cadence; keep
    // frames coming while it is still travelling.
    let now = ctx.renderer_mut().frame_instant();
    if state.borrow().advance_swipe_settle(now) {
        ctx.renderer_mut().request_refresh();
    }
    // One hit-test group for every row gesture in this list, so a swipe on one
    // row and a reorder drag on another can never both claim the same pointer.
    let gesture_group = ctx.renderer_mut().allocate_gesture_group_id();
    // Begin a fresh frame for the per-row content sub-view cache: only rows touched
    // in the visible loop below survive `end_frame`, preserving virtualization.
    state.borrow().item_cache.borrow_mut().begin_frame();
    // Resting geometry for the whole visible window is resolved before anything
    // is painted. A lifted row has to be drawn last so it floats over the rows
    // it travels past, so draw order can no longer be the order the vertical
    // cursor advances in.
    let mut rows = Vec::with_capacity(window.end.saturating_sub(window.start));
    let mut y = viewport.y0 - metrics.offset_y + window.leading_offset;
    for index in window.start..window.end {
        let row_env = env.clone();
        let item = materialize_list_item(&contents, index, &row_env);
        let row_id = contents
            .get_id(index)
            .unwrap_or_else(|| panic!("hydrolysis List item {index} has no stable id"));
        let row_height = {
            let cached_extent = state.borrow().extent_index.borrow().measured(index);
            if let Some(extent) = cached_extent {
                extent
            } else {
                let extent = measure_list_item_row_height(&item, ctx.state_mut(), &row_env);
                state
                    .borrow()
                    .extent_index
                    .borrow_mut()
                    .set_measured(index, extent);
                extent
            }
        };
        rows.push((index, row_id, item, y, row_height));
        y += row_height;
    }
    let lifted_id = state.borrow().reorder.get().map(|reorder| reorder.id);
    if let Some(lifted_id) = lifted_id
        && let Some(position) = rows.iter().position(|(_, id, ..)| *id == lifted_id)
    {
        let lifted_row = rows.remove(position);
        rows.push(lifted_row);
    }
    let visible_ids: Vec<ListItemId> = rows.iter().map(|(_, id, ..)| *id).collect();

    for (index, row_id, item, resting_y, row_height) in rows {
        let row_env = env.clone();
        let row_interaction_base = (i32::from(*row_id) as u32 as usize)
            .checked_mul(3)
            .expect("hydrolysis List interaction identity overflow");
        let reorder_dy = state.borrow().reorder_offset_for(index, row_id, row_height);
        let swipe_dx = state.borrow().swipe_offset_for(row_id);
        // Where the row's slot is (the gap it occupies in the list), before the
        // row itself is displaced sideways by a swipe.
        let slot_rect = vello::kurbo::Rect::new(
            viewport.x0,
            resting_y + reorder_dy,
            viewport.x1,
            resting_y + reorder_dy + row_height,
        );
        if slot_rect.y1 <= viewport.y0 || slot_rect.y0 >= viewport.y1 {
            continue;
        }
        {
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            if swipe_dx != 0.0 {
                let threshold =
                    (slot_rect.width() * SWIPE_DISMISS_POSITIONAL_THRESHOLD).max(f64::EPSILON);
                let progress = (swipe_dx.abs() / threshold).clamp(0.0, 1.0);
                theme.draw_list_swipe_dismiss_background(
                    &mut draw,
                    slot_rect,
                    progress,
                    swipe_dx < 0.0,
                );
            }
        }
        // Everything the row draws — its background, controls and content —
        // rides the swipe displacement; only the revealed dismiss background
        // stays anchored to the slot.
        let row_rect = slot_rect + vello::kurbo::Vec2::new(swipe_dx, 0.0);
        {
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_list_row_background(&mut draw, row_rect, index % 2 == 1);
            if lifted_id == Some(row_id) {
                theme.draw_list_row_lifted(&mut draw, row_rect, REORDER_LIFT_ELEVATION);
            }
        }

        let deletable = ctx.renderer_mut().read_signal(&item.deletable);
        let content_size = measure_view_intrinsic(&item.content, ctx.state_mut(), &row_env);
        let mut content_rect = list_content_rect(row_rect, list_metrics, content_size);
        let mut trailing_x = row_rect.x1 - 8.0;

        // Refreshed before either recognizer runs this frame, so an in-flight
        // drag always sees the row's current index and size.
        let row_binding = refresh_row_binding(
            state,
            row_id,
            RowBinding {
                index,
                width: slot_rect.width(),
                height: row_height,
                total_rows,
            },
        );

        // Swipe-to-dismiss covers the whole row and is available whenever the
        // list can delete, matching Material's `SwipeToDismissBox` rather than
        // being gated behind edit mode.
        if has_delete && deletable {
            register_row_swipe_gesture(
                ctx,
                state,
                gesture_group,
                row_id,
                Rc::clone(&row_binding),
                slot_rect,
                &row_env,
            );
        }

        if editing && has_move {
            let control_width = list_metrics.move_control_width;
            let vertical_inset = list_metrics.trailing_control_vertical_inset;
            let control_height = (row_height - vertical_inset * 2.0).max(vertical_inset * 2.0);
            let control_rect = vello::kurbo::Rect::new(
                trailing_x - control_width,
                row_rect.y0 + vertical_inset,
                trailing_x,
                row_rect.y0 + vertical_inset + control_height,
            );
            trailing_x -= control_width + list_metrics.trailing_control_spacing;
            // The handle is also the reorder grip: dragging it lifts the row.
            // The tap targets below stay, so the same control still offers
            // discrete one-step moves for pointer and keyboard users.
            register_row_reorder_gesture(
                ctx,
                state,
                gesture_group,
                row_id,
                Rc::clone(&row_binding),
                control_rect,
                &row_env,
            );
            let up_rect = vello::kurbo::Rect::new(
                control_rect.x0,
                control_rect.y0,
                control_rect.x1,
                control_rect.y0 + control_rect.height() / 2.0,
            );
            let down_rect = vello::kurbo::Rect::new(
                control_rect.x0,
                control_rect.y0 + control_rect.height() / 2.0,
                control_rect.x1,
                control_rect.y1,
            );
            let up_interaction = (index > 0).then(|| {
                let hit_bounds = transformed_rect(ctx.hit_transform, up_rect);
                let key = crate::renderer::InteractionKey::for_rc(state, row_interaction_base);
                let (state, slot, _) = ctx
                    .renderer_mut()
                    .bind_interaction_target(key, hit_bounds, &row_env);
                (hit_bounds, state, slot)
            });
            let down_interaction = (index + 1 < total_rows).then(|| {
                let hit_bounds = transformed_rect(ctx.hit_transform, down_rect);
                let key = crate::renderer::InteractionKey::for_rc(state, row_interaction_base + 1);
                let (state, slot, _) = ctx
                    .renderer_mut()
                    .bind_interaction_target(key, hit_bounds, &row_env);
                (hit_bounds, state, slot)
            });
            {
                let up_state = up_interaction
                    .as_ref()
                    .map(|(_, state, _)| local_interaction_state(*state, ctx.hit_transform));
                let down_state = down_interaction
                    .as_ref()
                    .map(|(_, state, _)| local_interaction_state(*state, ctx.hit_transform));
                let theme = widget_theme(env);
                let mut draw = ctx.draw_context();
                theme.draw_list_move_control(&mut draw, control_rect);
                if let Some(state) = up_state {
                    theme.draw_list_move_control_state_layer(&mut draw, up_rect, state);
                }
                if let Some(state) = down_state {
                    theme.draw_list_move_control_state_layer(&mut draw, down_rect, state);
                }
            }
            if let Some((hit_bounds, _, press_slot)) = up_interaction {
                let state = Rc::clone(state);
                let action_env = row_env.clone();
                ctx.renderer_mut().register_interactive_pointer_target(
                    hit_bounds,
                    press_slot,
                    move |_renderer, _point, _env| {
                        state.borrow().preserve_anchor_index_once.set(true);
                        if let Some(action) = state.borrow().config.on_move.as_ref() {
                            (action)(&action_env, Move::new(index, index - 1));
                        }
                        if !state.borrow().rows_dirty.get() {
                            state.borrow().preserve_anchor_index_once.set(false);
                        }
                        true
                    },
                );
            }
            if let Some((hit_bounds, _, press_slot)) = down_interaction {
                let state = Rc::clone(state);
                let action_env = row_env.clone();
                ctx.renderer_mut().register_interactive_pointer_target(
                    hit_bounds,
                    press_slot,
                    move |_renderer, _point, _env| {
                        state.borrow().preserve_anchor_index_once.set(true);
                        if let Some(action) = state.borrow().config.on_move.as_ref() {
                            (action)(&action_env, Move::new(index, index + 1));
                        }
                        if !state.borrow().rows_dirty.get() {
                            state.borrow().preserve_anchor_index_once.set(false);
                        }
                        true
                    },
                );
            }
        }

        if editing && deletable && has_delete {
            let delete_rect = vello::kurbo::Rect::new(
                trailing_x - list_metrics.delete_control_width,
                row_rect.y0 + list_metrics.trailing_control_vertical_inset,
                trailing_x,
                row_rect.y1 - list_metrics.trailing_control_vertical_inset,
            );
            trailing_x = delete_rect.x0 - list_metrics.trailing_control_spacing;
            let delete_hit_bounds = transformed_rect(ctx.hit_transform, delete_rect);
            let delete_key =
                crate::renderer::InteractionKey::for_rc(state, row_interaction_base + 2);
            let (delete_interaction, delete_press_slot, _) = ctx
                .renderer_mut()
                .bind_interaction_target(delete_key, delete_hit_bounds, &row_env);
            {
                let delete_interaction =
                    local_interaction_state(delete_interaction, ctx.hit_transform);
                let theme = widget_theme(env);
                let mut draw = ctx.draw_context();
                theme.draw_list_delete_control(&mut draw, delete_rect);
                theme.draw_list_delete_control_state_layer(
                    &mut draw,
                    delete_rect,
                    delete_interaction,
                );
            }
            let state = Rc::clone(state);
            let action_env = row_env.clone();
            ctx.renderer_mut().register_interactive_pointer_target(
                delete_hit_bounds,
                delete_press_slot,
                move |_renderer, _point, _env| {
                    if let Some(action) = state.borrow().config.on_delete.as_ref() {
                        (action)(&action_env, index);
                    }
                    true
                },
            );
        }

        content_rect.x1 = content_rect.x1.min(trailing_x);
        if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
            // Render the row content through a persistent node held in the per-widget
            // cache, keyed by stable row id, instead of re-dispatching it each frame.
            // The cache keeps a row's node only while it stays visible (built on first
            // appearance, evicted by `end_frame` once it scrolls out), so reactive row
            // content stays live across frames while virtualization is preserved. Row
            // a11y is emitted by `list_accessibility`, so suppress the sub-view's own
            // a11y (matching the old `dispatch_in_rect_without_accessibility`).
            let id = contents
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis list row {index} has no id"));
            let content = item.content;
            #[cfg(feature = "accessibility")]
            ctx.renderer_mut().push_accessibility_suppression();
            let render_ctx = ctx.render_context();
            {
                let state_ref = state.borrow();
                let mut cache = state_ref.item_cache.borrow_mut();
                let subview = cache.entry(id, move || content);
                subview.flush_in_rect(ctx.renderer_mut(), render_ctx, &row_env, content_rect);
            }
            #[cfg(feature = "accessibility")]
            ctx.renderer_mut().pop_accessibility_suppression();
        }

        {
            let separator = vello::kurbo::Rect::new(
                row_rect.x0 + list_metrics.divider_leading_inset,
                row_rect.y1 - 1.0,
                row_rect.x1 - list_metrics.divider_trailing_inset,
                row_rect.y1,
            );
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_list_separator(&mut draw, separator);
        }
    }
    // Evict content sub-views for rows no longer in the visible window.
    state.borrow().item_cache.borrow_mut().end_frame();
    // Retained gesture recognizers follow the same virtualization rule: a row
    // that scrolled out has no in-flight gesture worth remembering, and keeping
    // one per row would defeat the point of a 100k-row lazy list.
    state
        .borrow()
        .row_gestures
        .borrow_mut()
        .retain(|id, _| visible_ids.contains(id));

    if state.borrow().pending_scroll.get().is_some() {
        let content_height = state
            .borrow()
            .extent_index
            .borrow()
            .total_extent()
            .max(viewport.height());
        let rebound =
            state
                .borrow()
                .bind_scroll(viewport.width(), viewport.height(), content_height);
        state
            .borrow()
            .apply_scroll_request(ctx.renderer_mut(), &rebound, row_count);
        metrics = rebound.metrics();
        ctx.renderer_mut().frame_signals().request_refresh();
    }
    state.borrow().record_viewport_anchor(metrics, row_count);

    if needs_viewport_clip {
        ctx.pop_layer();
    }

    let handle_for_input = handle.clone();
    let hit_transform = ctx.hit_transform;
    ctx.renderer_mut().register_scroll_target(
        transformed_rect(hit_transform, viewport),
        handle.clone(),
        move |dx, dy, is_line_delta| handle_for_input.apply_scroll_delta(dx, dy, is_line_delta),
    );
    draw_scroll_indicators(ctx, env, viewport, metrics, ScrollAxis::Vertical, &handle);
}

/// Ensures row `row_id` has a retained gesture binding and refreshes it with
/// this frame's geometry.
///
/// The binding is what lets a recognizer outlive the frame that built it: the
/// closures read the row's *current* index and size through this cell, so a
/// drag still in flight after a reorder or a deletion acts on where the row is
/// now rather than on the index captured when the recognizer was created.
fn refresh_row_binding(
    state: &Rc<RefCell<ListRenderState>>,
    row_id: ListItemId,
    binding: RowBinding,
) -> Rc<Cell<RowBinding>> {
    let state_ref = state.borrow();
    let mut gestures = state_ref.row_gestures.borrow_mut();
    let row = gestures.entry(row_id).or_insert_with(RowGestures::empty);
    row.binding.set(binding);
    Rc::clone(&row.binding)
}

/// Registers `gesture` for one row, reusing the recognizer retained from an
/// earlier frame when there is one.
///
/// Building a fresh recognizer every frame would reset the state machine mid-
/// drag, so a swipe would stall the moment the list re-rendered. `slot` names
/// which of the row's two gestures this is.
fn register_row_gesture(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ListRenderState>>,
    group: usize,
    row_id: ListItemId,
    bounds: vello::kurbo::Rect,
    slot: RowGestureSlot,
    build: impl FnOnce() -> (Gesture, BoxedAction<()>),
) {
    let hit_bounds = transformed_rect(ctx.hit_transform, bounds);
    let retained = {
        let state_ref = state.borrow();
        let gestures = state_ref.row_gestures.borrow();
        gestures.get(&row_id).and_then(|row| slot.get(row).cloned())
    };
    if let Some(target) = retained {
        ctx.renderer_mut()
            .register_retained_gesture_target(&target, hit_bounds, group);
        return;
    }
    let (gesture, action) = build();
    let Some(target) = ctx
        .renderer_mut()
        .register_gesture_target(hit_bounds, group, gesture, action)
    else {
        return;
    };
    let state_ref = state.borrow();
    let mut gestures = state_ref.row_gestures.borrow_mut();
    let row = gestures.entry(row_id).or_insert_with(RowGestures::empty);
    slot.set(row, target);
}

/// Swipe-to-dismiss across a whole row, committing `on_delete` once the row
/// travels past Material's positional threshold.
fn register_row_swipe_gesture(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ListRenderState>>,
    group: usize,
    row_id: ListItemId,
    binding: Rc<Cell<RowBinding>>,
    bounds: vello::kurbo::Rect,
    row_env: &Environment,
) {
    let owner = Rc::clone(state);
    let action_env = row_env.clone();
    register_row_gesture(
        ctx,
        state,
        group,
        row_id,
        bounds,
        RowGestureSlot::Swipe,
        || {
            let action: BoxedAction<()> = boxed_action(move |env: Environment| {
                let event = drag_event(&env);
                let offset = f64::from(event.translation.x);
                let row = binding.get();
                let list = owner.borrow();
                match event.phase {
                    GesturePhase::Started | GesturePhase::Updated => {
                        list.swipe.set(Some(RowSwipe {
                            id: row_id,
                            offset,
                            settling: false,
                            target: 0.0,
                        }));
                    }
                    GesturePhase::Ended => {
                        let threshold = row.width * SWIPE_DISMISS_POSITIONAL_THRESHOLD;
                        if offset.abs() >= threshold {
                            list.swipe.set(None);
                            if let Some(delete) = list.config.on_delete.as_ref() {
                                (delete)(&action_env, row.index);
                            }
                        } else {
                            // Short of the threshold the row springs back instead
                            // of committing, exactly as `SwipeToDismissBox` does.
                            list.swipe.set(Some(RowSwipe {
                                id: row_id,
                                offset,
                                settling: true,
                                target: 0.0,
                            }));
                        }
                    }
                    GesturePhase::Cancelled => {
                        list.swipe.set(Some(RowSwipe {
                            id: row_id,
                            offset,
                            settling: true,
                            target: 0.0,
                        }));
                    }
                }
            });
            (Gesture::Drag(DragGesture::new(ROW_DRAG_SLOP)), action)
        },
    );
}

/// Vertical reorder drag on a row's move handle, committing `on_move` on
/// release.
fn register_row_reorder_gesture(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ListRenderState>>,
    group: usize,
    row_id: ListItemId,
    binding: Rc<Cell<RowBinding>>,
    bounds: vello::kurbo::Rect,
    row_env: &Environment,
) {
    let owner = Rc::clone(state);
    let action_env = row_env.clone();
    register_row_gesture(
        ctx,
        state,
        group,
        row_id,
        bounds,
        RowGestureSlot::Reorder,
        || {
            let action: BoxedAction<()> = boxed_action(move |env: Environment| {
                let event = drag_event(&env);
                let dy = f64::from(event.translation.y);
                let row = binding.get();
                // How many whole slots the pointer has travelled. Rows are
                // measured individually, but a drag only ever crosses its
                // neighbours one at a time, so stepping by the dragged row's
                // own height tracks the pointer closely enough to feel direct.
                let step = if row.height > 0.0 {
                    (dy / row.height).round()
                } else {
                    0.0
                };
                let to = reorder_destination(row.index, step, row.total_rows);
                let list = owner.borrow();
                match event.phase {
                    GesturePhase::Started | GesturePhase::Updated => {
                        list.reorder.set(Some(RowReorder {
                            id: row_id,
                            from: row.index,
                            to,
                            dy,
                        }));
                    }
                    GesturePhase::Ended => {
                        list.reorder.set(None);
                        if to != row.index {
                            list.preserve_anchor_index_once.set(true);
                            if let Some(action) = list.config.on_move.as_ref() {
                                (action)(&action_env, Move::new(row.index, to));
                            }
                            if !list.rows_dirty.get() {
                                list.preserve_anchor_index_once.set(false);
                            }
                        }
                    }
                    GesturePhase::Cancelled => {
                        list.reorder.set(None);
                    }
                }
            });
            (Gesture::Drag(DragGesture::new(ROW_DRAG_SLOP)), action)
        },
    );
}

/// Clamps a slot step from `index` into the collection's index range.
fn reorder_destination(index: usize, step: f64, total_rows: usize) -> usize {
    if total_rows == 0 {
        return 0;
    }
    let last = total_rows - 1;
    if step >= 0.0 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "step is a whole non-negative slot count, clamped into range below"
        )]
        let forward = step as usize;
        index.saturating_add(forward).min(last)
    } else {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the negated step is a whole non-negative slot count"
        )]
        let backward = (-step) as usize;
        index.saturating_sub(backward)
    }
}

/// Reads the drag payload the gesture engine layered into the action's
/// environment.
fn drag_event(env: &Environment) -> DragEvent {
    env.get::<DragEvent>()
        .expect("hydrolysis list drag action is missing its DragEvent")
        .clone()
}

/// Which of a row's two retained recognizers is being addressed.
#[derive(Clone, Copy)]
enum RowGestureSlot {
    Swipe,
    Reorder,
}

impl RowGestureSlot {
    const fn get(self, row: &RowGestures) -> Option<&GestureTarget> {
        match self {
            Self::Swipe => row.swipe.as_ref(),
            Self::Reorder => row.reorder.as_ref(),
        }
    }

    fn set(self, row: &mut RowGestures, target: GestureTarget) {
        match self {
            Self::Swipe => row.swipe = Some(target),
            Self::Reorder => row.reorder = Some(target),
        }
    }
}

impl RowGestures {
    fn empty() -> Self {
        Self {
            binding: Rc::new(Cell::new(RowBinding::PLACEHOLDER)),
            swipe: None,
            reorder: None,
        }
    }
}

fn list_content_rect(
    row_rect: vello::kurbo::Rect,
    metrics: waterui_backend_core::widget::ListMetrics,
    content_size: waterui_core::layout::Size,
) -> vello::kurbo::Rect {
    // Rows propose their full inset width to the content; horizontal
    // alignment belongs to the content itself (composite items cannot be
    // statically classified as stretching, and interactive rows must keep a
    // full-width hit target).
    let x0 = row_rect.x0 + metrics.horizontal_inset;
    let x1 = row_rect.x1 - metrics.horizontal_inset;
    let available_height = (row_rect.height() - metrics.vertical_inset * 2.0).max(0.0);
    let height = f64::from(content_size.height).min(available_height);
    let y0 = row_rect.y0 + (row_rect.height() - height) * 0.5;
    vello::kurbo::Rect::new(x0, y0, x1, y0 + height)
}
