use crate::renderer::bounded_proposal;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::gesture::GestureTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::{
    AccessibilityActionTarget, accessibility_container_child_environment,
    hoist_accessibility_metadata,
};
use crate::renderer::{
    HydroNativeView, HydroState, RenderContext, VisibleSubviewCache, WidgetRenderContext,
    list_row_height_for_content, local_interaction_state, materialize_list_item,
    measure_list_intrinsic, measure_transient_view_intrinsic, transformed_rect,
};
use crate::scroll::ScrollHandle;
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, NodeId as AccessibilityNodeId,
    Role as AccessibilityNodeRole,
};
#[cfg(feature = "accessibility")]
use waterui::accessibility::{AccessibilityHidden, AccessibilityStateSignal};
use waterui::component::list::{ListConfig, ListItem, ListSelection, Move};
use waterui::gesture::{DragEvent, DragGesture, Gesture, GesturePhase};
use waterui_core::handler::{BoxedAction, boxed_action};
use waterui_core::id::{Id as RawId, SelfId};
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::views::{SharedAnyViews, Views};
use waterui_core::{Environment, Native};
use waterui_layout::scroll::Axis as ScrollAxis;
use waterui_text::Text;

use crate::platform::Modifiers;
use crate::renderer::lazy::VirtualExtentIndex;
use crate::widgets::draw_scroll_indicators;
use nami::watcher::BoxWatcherGuard;
use nami::{Computed, Signal, SignalExt as _};
use waterui::theme::color;
use waterui_core::resolve::Resolvable as _;
use waterui_graphics::cherenkov::Draw as _;

/// The stable per-row id used to key the retained content sub-view cache, matching
/// the id `ListConfig::contents` (a `SharedAnyViews<ListItem>`) yields per index.
pub(crate) type ListItemId = SelfId<RawId>;

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

/// Rows a programmatic jump animates over. A target further away than this is
/// closed instantly first and only the last stretch is animated, so the glide
/// stays legible and cannot drag the list through a whole dataset. This is
/// Compose's `NumberOfItemsToTeleport`, the same bound `animateScrollToItem`
/// applies.
const ROWS_BEFORE_JUMP_TELEPORT: usize = 100;

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

/// The list's row-selection state, shared by every input path — pointer,
/// keyboard and accessibility — so all of them write the same erased
/// `ListSelection` binding under the same rules: a plain click selects, the
/// toggle modifier toggles the clicked row in multi mode, and Shift extends a
/// range from the anchor the last non-Shift write set (water-rs/waterui#1226).
pub(crate) struct ListRowSelection {
    /// The erased selection `ListConfig` carries — keyed by the same row ids
    /// `contents.get_id` reports.
    selection: ListSelection<ListItemId>,
    /// Row ids by index, resolved for Shift-range writes.
    contents: SharedAnyViews<ListItem>,
    /// The row a Shift range extends from — the last row written without
    /// Shift, or the list's first row when nothing has been written yet.
    anchor: Cell<Option<ListItemId>>,
}

impl ListRowSelection {
    /// Shares the config's selection when the list is selectable; `None` on
    /// `ListSelection::None`, so a non-selectable list keeps its old input
    /// behaviour — no row press target, no selected state.
    fn new(
        selection: &ListSelection<ListItemId>,
        contents: SharedAnyViews<ListItem>,
    ) -> Option<Rc<Self>> {
        match selection {
            ListSelection::None => None,
            _ => Some(Rc::new(Self {
                selection: selection.clone(),
                contents,
                anchor: Cell::new(None),
            })),
        }
    }

    /// Whether the row is selected, as a signal the flush reads like any other
    /// row state — a binding write repaints the row's chrome.
    pub(crate) fn is_selected(&self, id: ListItemId) -> Computed<bool> {
        match &self.selection {
            ListSelection::None => nami::constant(false).computed(),
            ListSelection::Single(selection) => selection
                .clone()
                .map(move |current| current == Some(id))
                .computed(),
            ListSelection::Multiple(selection) => selection
                .clone()
                .map(move |current| current.contains(&id))
                .computed(),
        }
    }

    /// The index `id` currently occupies — Shift ranges are measured in
    /// indices, so a range write resolves the anchor's position the same way
    /// the row loop does.
    fn index_of(&self, id: ListItemId) -> Option<usize> {
        (0..nami::Signal::snapshot(&self.contents.len()))
            .find(|index| self.contents.get_id(*index) == Some(id))
    }

    /// Writes a row interaction into the selection binding: plain selects the
    /// row (and anchors the next range), the toggle modifier toggles the row
    /// in multi mode (also anchoring), and Shift writes the whole range
    /// between the anchor and this row without moving the anchor.
    pub(crate) fn write(&self, index: usize, id: ListItemId, modifiers: Modifiers) {
        match &self.selection {
            ListSelection::None => {}
            ListSelection::Single(selection) => {
                selection.set(Some(id));
            }
            ListSelection::Multiple(selection) if modifiers.shift => {
                let anchor = self
                    .anchor
                    .get()
                    .and_then(|anchor| self.index_of(anchor))
                    .unwrap_or(0);
                let (start, end) = if anchor <= index {
                    (anchor, index)
                } else {
                    (index, anchor)
                };
                selection.set(
                    (start..=end)
                        .filter_map(|row| self.contents.get_id(row))
                        .collect(),
                );
            }
            ListSelection::Multiple(selection) => {
                if modifiers.control || modifiers.super_key {
                    selection.with_mut(|selected| {
                        if !selected.insert(id) {
                            selected.remove(&id);
                        }
                    });
                } else {
                    selection.set(std::collections::BTreeSet::from([id]));
                }
                self.anchor.set(Some(id));
            }
        }
    }
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
    /// position in a backend-global slot array. Shared with each row's
    /// `ListRow` accessibility target so a `ScrollIntoView` request resolves
    /// the row's span against the same measured extents the draw pass uses.
    extent_index: Rc<RefCell<VirtualExtentIndex>>,
    /// The scroll offset belongs to this semantic list node.
    scroll: RefCell<Option<ScrollHandle>>,
    /// Content sub-views for the rows currently in view, keyed by stable row id so a
    /// steady scroll reuses each visible row's node (keeping its reactive content
    /// live) and only builds rows entering the window.
    item_cache: RefCell<VisibleSubviewCache<ListItemId>>,
    /// A membership change invalidates index-based extents, including reorder
    /// operations whose collection length stays unchanged.
    rows_dirty: Rc<Cell<bool>>,
    /// Ids the collection watcher reported as replaced since the last
    /// `prepare_rows` consume — same-id items whose content may differ, so
    /// exactly those rows are dropped from `item_cache` and re-materialized
    /// while every other row keeps its retained node (focus, gestures,
    /// in-flight scroll anchoring all live inside it).
    replaced_row_ids: Rc<RefCell<std::collections::HashSet<ListItemId>>>,
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
    /// Per-row section chrome, resolved once per membership change for a list
    /// whose rows carry section markers.
    sections: RefCell<Vec<RowSectionChrome>>,
    /// Row count the resolved chrome was built for, so a list that renders
    /// before its rows exist re-resolves once they do.
    sections_resolved_for: Cell<Option<usize>>,
    /// The list's row-selection state shared by pointer, keyboard and
    /// accessibility input; `None` when the list is not selectable.
    row_selection: Option<Rc<ListRowSelection>>,
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

/// The section chrome one row is responsible for drawing.
///
/// A marker opens a section on the row that carries it, so that row owns the
/// header. The footer closes the section on a *different* row — the last one
/// before the next marker — which the draw loop cannot discover on its own
/// while only part of the list is realized. Resolving both onto rows up front
/// keeps every row's height a local question again.
///
/// The header and footer are semantic text, not strings resolved once: an app
/// can drive a section title from a signal, and the retained chrome is only
/// rebuilt when membership changes. Holding the signal here is what lets a
/// title change repaint without the list being rebuilt.
#[derive(Clone, Default)]
struct RowSectionChrome {
    header: Option<Text>,
    footer: Option<Text>,
}

impl RowSectionChrome {
    fn header_height(&self, metrics: &waterui_backend_core::widget::ListMetrics) -> f64 {
        if self.header.is_some() {
            metrics.section_header_height
        } else {
            0.0
        }
    }

    fn footer_height(&self, metrics: &waterui_backend_core::widget::ListMetrics) -> f64 {
        if self.footer.is_some() {
            metrics.section_footer_height
        } else {
            0.0
        }
    }

    fn total_height(&self, metrics: &waterui_backend_core::widget::ListMetrics) -> f64 {
        self.header_height(metrics) + self.footer_height(metrics)
    }
}

impl ListRenderState {
    pub(crate) fn from_config(
        config: ListConfig,
        renderer: &crate::renderer::SemanticCore,
    ) -> Self {
        let rows_dirty = Rc::new(Cell::new(true));
        let rows_dirty_for_watch = Rc::clone(&rows_dirty);
        let replaced_row_ids = Rc::new(RefCell::new(std::collections::HashSet::new()));
        let replaced_for_watch = Rc::clone(&replaced_row_ids);
        let signals = renderer.frame_signals();
        let guard = config.contents.watch(.., move |ctx, change| {
            rows_dirty_for_watch.set(true);
            crate::renderer::collect_replaced_ids(
                ctx.value(),
                &change,
                &mut replaced_for_watch.borrow_mut(),
            );
            signals.request_refresh();
        });
        let row_selection = ListRowSelection::new(&config.selection, config.contents.clone());
        Self {
            config,
            row_selection,
            extent_index: Rc::new(RefCell::new(VirtualExtentIndex::default())),
            scroll: RefCell::new(None),
            item_cache: RefCell::new(VisibleSubviewCache::new()),
            rows_dirty,
            replaced_row_ids,
            applied_scroll_generation: Cell::new(0),
            pending_scroll: Cell::new(None),
            viewport_anchor: Cell::new(None),
            pending_membership_offset: Cell::new(None),
            preserve_anchor_index_once: Cell::new(false),
            row_gestures: RefCell::new(std::collections::HashMap::new()),
            swipe: Rc::new(Cell::new(None)),
            reorder: Rc::new(Cell::new(None)),
            swipe_last_tick: Cell::new(None),
            sections: RefCell::new(Vec::new()),
            sections_resolved_for: Cell::new(None),
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

    /// Resolves each row's section chrome from the markers the rows carry.
    ///
    /// Only a list built from static section content is walked: `uses_sections`
    /// is false for `List::for_each`, whose rows are virtualized and must never
    /// all be materialized at once.
    /// Returns whether the chrome changed, which invalidates row extents: a
    /// row's height includes the chrome it owns.
    fn resolve_sections(&self, len: usize, env: &Environment) -> bool {
        if !self.config.uses_sections {
            return false;
        }
        if self.sections_resolved_for.get() == Some(len) {
            return false;
        }

        let mut chrome = vec![RowSectionChrome::default(); len];
        // The footer of the section a row opens closes on the row before the
        // next marker, so each marker settles the *previous* section's footer.
        let mut open_section: Option<(usize, Option<Text>)> = None;
        for index in 0..len {
            let item = materialize_list_item(&self.config.contents, index, env);
            let Some(section) = item.section else {
                continue;
            };
            if let Some((_, footer)) = open_section.take()
                && index > 0
            {
                chrome[index - 1].footer = footer;
            }
            chrome[index].header = section.label;
            open_section = Some((index, section.footer));
        }
        if let Some((_, footer)) = open_section
            && len > 0
        {
            chrome[len - 1].footer = footer;
        }

        *self.sections.borrow_mut() = chrome;
        self.sections_resolved_for.set(Some(len));
        true
    }

    fn section_chrome(&self, index: usize) -> RowSectionChrome {
        self.sections
            .borrow()
            .get(index)
            .cloned()
            .unwrap_or_default()
    }

    /// Drop the retained sub-views of the rows the collection watcher
    /// reported as replaced since the last consume — a same-id content change
    /// re-materializes exactly those rows on their next `entry`, while every
    /// untouched row keeps its node (focus, gestures, in-flight scroll
    /// anchoring all live inside it).
    fn consume_replaced_rows(&self) {
        let replaced = core::mem::take(&mut *self.replaced_row_ids.borrow_mut());
        if !replaced.is_empty() {
            self.item_cache.borrow_mut().invalidate_ids(&replaced);
        }
    }

    fn prepare_rows(&self, len: usize, estimate: f64) {
        let dirty = self.rows_dirty.replace(false);
        self.consume_replaced_rows();
        if dirty || !self.extent_index.borrow().matches(len, estimate, 0.0) {
            self.extent_index.borrow_mut().reset(len, estimate, 0.0);
            self.sections_resolved_for.set(None);
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
                None,
            );
            *scroll = Some(handle.clone());
            handle
        }
    }

    /// Applies the pending scroll request, if there is one. `animate` selects
    /// between the rendered glide and the semantic jump: nothing ticks the
    /// list's smooth scroll on the semantic runtime, so a request there lands
    /// in place instead.
    fn apply_scroll_request(
        &self,
        renderer: &mut crate::renderer::SemanticCore,
        handle: &ScrollHandle,
        row_count: usize,
        animate: bool,
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
            self.pending_scroll.set(Some((generation, index)));
        }
        let Some((pending_generation, index)) = self.pending_scroll.get() else {
            return;
        };
        if index >= row_count {
            // A scroll request names a row the contents may not have yet: a
            // list materializing mid-flush can be shorter than its pending
            // target, and a signal-driven collection can shrink below it. The
            // request stays pending until the collection reaches the index;
            // a newer generation supersedes it.
            return;
        }
        // Ease toward the row rather than teleporting. Re-issuing the target
        // every frame is what keeps a virtualized jump accurate: rows measured
        // while the glide passes over them move `offset_of(index)`, so the
        // destination is refined until the animation actually settles.
        let offset = self.extent_index.borrow().offset_of(index);
        if animate {
            let metrics = handle.metrics();
            let current = self
                .extent_index
                .borrow()
                .visible_window(metrics.offset_y, metrics.offset_y + metrics.viewport_height)
                .start;
            if index.abs_diff(current) > ROWS_BEFORE_JUMP_TELEPORT {
                // Animating the whole way across a 100k-row dataset would drag the
                // list through every viewport between here and there, and read as a
                // blur regardless. Compose solves this the same way: its
                // `animateScrollToItem` snaps to within `NumberOfItemsToTeleport`
                // items of the target and animates only that final stretch.
                let approach_index = if index > current {
                    index - ROWS_BEFORE_JUMP_TELEPORT
                } else {
                    index + ROWS_BEFORE_JUMP_TELEPORT
                };
                let approach = self.extent_index.borrow().offset_of(approach_index);
                let _ = handle.scroll_to(0.0, approach);
            }
            // The pump ticks smooth scrolls before rendering and the present already
            // wakes the loop, so arming here is enough — the next tick advances the
            // glide and keeps requesting frames until it settles.
            let _ = handle.scroll_to_animated(0.0, offset);
        } else {
            // The semantic runtime's pump never registers the list's handle in
            // its scroll targets, so a glide armed here would never tick; the
            // request lands in place.
            let _ = handle.scroll_to(0.0, offset);
        }
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
            // Re-anchoring after a delete or move keeps the viewport where the
            // user left it; that is a correction, not a journey, so it lands
            // immediately rather than gliding.
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
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_list_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// A row contributes up to six accessibility nodes — the section header it
/// opens, the row itself, the delete control and the two reorder halves edit
/// mode draws on it, and the section footer it closes — so each row's id owns
/// a slot of six keys rather than one.
#[cfg(feature = "accessibility")]
const A11Y_KEYS_PER_ROW: i64 = 6;
#[cfg(feature = "accessibility")]
const A11Y_KEY_ROW: i64 = 0;
#[cfg(feature = "accessibility")]
const A11Y_KEY_HEADER: i64 = 1;
#[cfg(feature = "accessibility")]
const A11Y_KEY_FOOTER: i64 = 2;
#[cfg(feature = "accessibility")]
const A11Y_KEY_DELETE: i64 = 3;
#[cfg(feature = "accessibility")]
const A11Y_KEY_MOVE_UP: i64 = 4;
#[cfg(feature = "accessibility")]
const A11Y_KEY_MOVE_DOWN: i64 = 5;

#[cfg(feature = "accessibility")]
fn row_a11y_key_base(row_id: ListItemId) -> i64 {
    i64::from(i32::from(*row_id)) * A11Y_KEYS_PER_ROW
}

/// Registers one section header or footer as its own accessibility node.
///
/// Section chrome is not part of any row: a screen reader announces "Activity"
/// as a heading and then the rows under it, rather than folding the title into
/// the first row's label. The label is read through the renderer so a title
/// driven by a signal re-flushes the tree when it changes.
#[cfg(feature = "accessibility")]
fn register_section_chrome_node(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    semantic_key: i64,
    label: Text,
    is_header: bool,
    bounds: kurbo::Rect,
    env: &Environment,
) -> Option<AccessibilityNodeId> {
    let role = if is_header {
        AccessibilityNodeRole::Header
    } else {
        AccessibilityNodeRole::Footer
    };
    let styled = renderer.read_resolved_text_styled(&label, env);
    let mut node = AccessibilityNode::new(renderer.resolve_accessibility_role(env, role));
    node.set_label(styled.to_string());
    match ctx {
        Some(ctx) => renderer.register_accessibility_child_node_with_key(
            semantic_key,
            node,
            transformed_rect(ctx.hit_transform, bounds),
            env,
            None,
        ),
        None => renderer.register_accessibility_child_node_with_key_semantic(
            semantic_key,
            node,
            env,
            None,
        ),
    }
}

/// Emits a list's accessibility tree from its node-owned retained state.
///
/// The rendered flush passes its [`RenderContext`] and theme: row extents are
/// real, and the emitted window is the scrolled viewport's. The semantic walk
/// passes `None` for both and emits every row — the semantic tree has no
/// viewport, and a row not emitted does not exist to assistive technology.
/// Scroll offsets then read as row indices: the scroll domain is measured in
/// rows, since the semantic path has no pixels to measure in.
pub(crate) fn list_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn crate::engine::WidgetTheme>>,
    state: &Rc<RefCell<ListRenderState>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    let owner = state;
    let state = state.borrow();
    let list = &state.config;
    let row_count_signal = list.contents.len();
    let row_count = renderer.read_signal(&row_count_signal);
    let list_metrics = theme.map(|theme| theme.list_metrics());
    if let Some(list_metrics) = list_metrics {
        // `min_row_height` is the floor every row is estimated and measured
        // against; unset, the theme's one-line height keeps the same values.
        let row_floor = list
            .min_row_height
            .map_or(list_metrics.one_line_row_height, f64::from);
        // A 0 floor still needs a positive seed estimate — measured extents
        // replace it row by row anyway.
        let row_estimate = row_floor.max(1.0);
        state.prepare_rows(row_count, row_estimate);
        // The chrome decides how tall each row's slot is, so it has to be
        // resolved before extents are measured here — exactly as the draw pass
        // does.
        if state.resolve_sections(row_count, env) {
            state
                .extent_index
                .borrow_mut()
                .reset(row_count, row_estimate, 0.0);
        }
    } else {
        // The semantic path keeps section chrome current but measures rows in
        // units — one row is one extent unit, so scroll offsets read as row
        // indices. It still consumes replaced-row invalidations: the semantic
        // emit reads the same retained rows.
        state.consume_replaced_rows();
        let _ = state.resolve_sections(row_count, env);
        let mut extent_index = state.extent_index.borrow_mut();
        if !extent_index.matches(row_count, 1.0, 0.0) {
            extent_index.reset(row_count, 1.0, 0.0);
        }
    }
    let _rendered = ctx.is_some();
    let viewport = ctx.map_or(kurbo::Rect::ZERO, |ctx| ctx.bounds);
    // The rendered scroll domain is the measured extent; the semantic one is
    // the row count — with a zero viewport every row is scrollable to.
    let content_height = state
        .extent_index
        .borrow()
        .total_extent()
        .max(viewport.height());
    let handle = state.bind_scroll(viewport.width(), viewport.height(), content_height);
    state.apply_membership_anchor(&handle);
    state.apply_scroll_request(renderer, &handle, row_count, _rendered);
    #[cfg(feature = "accessibility")]
    {
        let metrics = handle.metrics();
        let (emit_range, leading_offset) = if _rendered {
            let window = state
                .extent_index
                .borrow()
                .visible_window(metrics.offset_y, metrics.offset_y + viewport.height());
            (window.start..window.end, window.leading_offset)
        } else {
            (0..row_count, 0.0)
        };
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
        let editing = renderer.read_signal(&list.editing);
        let has_delete = list.on_delete.is_some();
        let has_move = list.on_move.is_some();
        if ctx.is_none() {
            // The semantic walk emits every row's content through the shared
            // sub-view cache — the same frame bookkeeping the rendered flush
            // runs keeps a row's retained node alive across emissions and
            // evicts the ones no row touched this pass.
            state.item_cache.borrow_mut().begin_frame();
        }
        let mut y = viewport.y0 - metrics.offset_y + leading_offset;
        for index in emit_range {
            let row_env = env.clone();
            let item = materialize_list_item(&list.contents, index, &row_env);
            let chrome = state.section_chrome(index);
            // Semantic rows have no layout extent — the slot is only measured
            // when the rendered path needs it to place the row.
            let slot_height = if _rendered {
                let cached_extent = state.extent_index.borrow().measured(index);
                if let Some(extent) = cached_extent {
                    extent
                } else {
                    let theme =
                        theme.expect("hydrolysis rendered list measurement requires a theme");
                    let list_metrics = list_metrics
                        .expect("hydrolysis rendered list measurement requires list metrics");
                    let content_size = measure_transient_view_intrinsic(
                        &item.content,
                        renderer.state_mut(),
                        &row_env,
                        theme,
                    );
                    let extent = list_row_height_for_content(
                        f64::from(content_size.height),
                        item.insets.as_ref(),
                        list.min_row_height,
                        list_metrics,
                    ) + chrome.total_height(&list_metrics);
                    state.extent_index.borrow_mut().set_measured(index, extent);
                    extent
                }
            } else {
                0.0
            };
            let slot_rect = kurbo::Rect::new(viewport.x0, y, viewport.x1, y + slot_height);
            y += slot_height;
            if _rendered && (slot_rect.y1 <= viewport.y0 || slot_rect.y0 >= viewport.y1) {
                continue;
            }
            let header_height = list_metrics.map_or(0.0, |m| chrome.header_height(&m));
            let footer_height = list_metrics.map_or(0.0, |m| chrome.footer_height(&m));
            // The chrome a row owns is not part of the row: a section title is
            // its own node, and the row's bounds are the band left between the
            // header and the footer — the same split the draw pass makes.
            let row_rect = kurbo::Rect::new(
                slot_rect.x0,
                slot_rect.y0 + header_height,
                slot_rect.x1,
                slot_rect.y1 - footer_height,
            );
            let row_id = list
                .contents
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis list row {index} has no stable identity"));
            let key_base = row_a11y_key_base(row_id);
            if let Some(header) = chrome.header.clone() {
                let header_rect = kurbo::Rect::new(
                    slot_rect.x0 + list_metrics.map_or(0.0, |m| m.horizontal_inset),
                    slot_rect.y0,
                    slot_rect.x1 - list_metrics.map_or(0.0, |m| m.horizontal_inset),
                    slot_rect.y0 + header_height,
                );
                if let Some(node_id) = register_section_chrome_node(
                    renderer,
                    ctx,
                    key_base + A11Y_KEY_HEADER,
                    header,
                    true,
                    header_rect,
                    &row_env,
                ) {
                    list_node.push_child(node_id);
                }
            }
            // Hoist the row content's accessibility metadata onto a scoped row
            // env, exactly as the retained build would: the row's `ListItem`
            // node then claims the content's explicit label, role or
            // identifier — not the first leaf inside it — and the subtree emits
            // under the container-child env that strips that naming, so the
            // row's name is announced once and children keep their own.
            let mut item = item;
            let (content, row_a11y_env) = hoist_accessibility_metadata(item.content, &row_env);
            item.content = content;
            // A hidden row vanishes whole — node and content — matching the
            // naming container's treatment of `accessibilityHidden`.
            let row_hidden = row_a11y_env
                .get::<AccessibilityHidden>()
                .is_some_and(AccessibilityHidden::is_hidden)
                || row_a11y_env
                    .get::<AccessibilityStateSignal>()
                    .is_some_and(|signal| renderer.read_signal(signal.state()).is_hidden());
            let mut row_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(&row_a11y_env, AccessibilityNodeRole::ListItem),
            );
            let default_label =
                renderer.accessibility_label_from_view(&item.content, &row_a11y_env);
            let label = renderer.resolve_accessibility_label(&row_a11y_env, default_label);
            if let Some(label) = label {
                row_node.set_label(label);
            }
            row_node.add_action(AccessibilityAction::Focus);
            // The selected state belongs to the list's selection, not the
            // item: a row exposes it only when the list is selectable.
            if let Some(selection) = state.row_selection.as_ref() {
                row_node.set_selected(renderer.read_signal(&selection.is_selected(row_id)));
            }
            let row_node_id = if row_hidden {
                None
            } else {
                // Arrow-key navigation moves through the row target:
                // `ScrollIntoView` reveals the row's span in the list's scroll
                // domain, and `Click` resolves the activation a pointer click
                // on the row's centre would run — Enter/Space fire it.
                row_node.add_action(AccessibilityAction::ScrollIntoView);
                row_node.add_action(AccessibilityAction::Click);
                let row_target = Some(AccessibilityActionTarget::ListRow {
                    index,
                    handle: handle.clone(),
                    extents: Rc::clone(&state.extent_index),
                    id: row_id,
                    selection: state.row_selection.clone(),
                });
                match ctx {
                    Some(ctx) => renderer.register_accessibility_child_node_with_key(
                        key_base + A11Y_KEY_ROW,
                        row_node,
                        transformed_rect(ctx.hit_transform, row_rect),
                        &row_a11y_env,
                        row_target,
                    ),
                    None => renderer.register_accessibility_child_node_with_key_semantic(
                        key_base + A11Y_KEY_ROW,
                        row_node,
                        &row_a11y_env,
                        row_target,
                    ),
                }
            };
            if let Some(row_node_id) = row_node_id {
                list_node.push_child(row_node_id);
                // Interaction slots per row: 0 and 1 are the reorder handle's
                // up/down press slots, 2 the delete control's, 3 the row's own
                // selection press, 4 the row's anchor — the key the draw pass
                // resolves the row node through to parent its content under.
                // Each press slot links to the node that control emits below,
                // so a pointer press lands keyboard focus on it.
                let row_interaction_base = (i32::from(*row_id) as u32 as usize)
                    .checked_mul(5)
                    .expect("hydrolysis List interaction identity overflow");
                renderer.register_accessibility_focus_link(
                    &crate::renderer::InteractionKey::for_rc(owner, row_interaction_base + 4),
                    row_node_id,
                );
                // The row's own press slot — the selection target the draw
                // pass registers — resolves focus to the same node.
                renderer.register_accessibility_focus_link(
                    &crate::renderer::InteractionKey::for_rc(owner, row_interaction_base + 3),
                    row_node_id,
                );
                let deletable = editing && renderer.read_signal(&item.deletable);
                let subtree_env = accessibility_container_child_environment(&row_a11y_env)
                    .unwrap_or_else(|| row_a11y_env.clone());
                if ctx.is_none() {
                    // Emit the row content's own semantics under the row's node:
                    // every text, control and image in the row becomes a child
                    // of its `ListItem`, so row content reaches the semantic
                    // tree. The rendered path parents the same subtree in
                    // `render_list_parts`.
                    renderer.push_accessibility_parent(row_node_id);
                    let content = item.content;
                    {
                        let mut cache = state.item_cache.borrow_mut();
                        let subview = cache.entry(row_id, move || content);
                        subview.emit_accessibility(renderer, &subtree_env);
                    }
                    renderer.pop_accessibility_parent();
                }
                // Edit mode's delete and reorder controls are pointer-only hit
                // regions in the draw pass — emit their nodes too, or the tree
                // is identical to a non-editing list and nothing can delete or
                // reorder a row through assistive technology
                // (water-rs/hydrolysis#52).
                //
                // The nodes take the same rects the draw pass paints and
                // hit-tests from `row_edit_controls`; the semantic walk has no
                // metrics, so its nodes carry no bounds.
                let controls = list_metrics.map(|metrics| {
                    row_edit_controls(
                        &metrics,
                        row_rect,
                        slot_rect.height(),
                        index,
                        row_count,
                        has_move,
                        deletable && has_delete,
                    )
                });
                if editing && deletable && has_delete {
                    let state = Rc::clone(owner);
                    let action_env = row_env.clone();
                    let node_id = register_edit_control_node(
                        renderer,
                        ctx,
                        key_base + A11Y_KEY_DELETE,
                        crate::localization::text(&subtree_env, "delete"),
                        controls.as_ref().and_then(|controls| controls.delete),
                        &subtree_env,
                        Rc::new(RefCell::new(
                            move |_renderer: &mut crate::renderer::SemanticCore,
                                  _env: &Environment| {
                                run_row_delete(&state, &action_env, index)
                            },
                        )),
                    );
                    if let Some(node_id) = node_id {
                        list_node.push_child(node_id);
                        renderer.register_accessibility_focus_link(
                            &crate::renderer::InteractionKey::for_rc(
                                owner,
                                row_interaction_base + 2,
                            ),
                            node_id,
                        );
                    }
                }
                if editing && has_move {
                    // The handle's halves are the two directions the draw pass
                    // presses on: a row at a boundary advertises only the
                    // direction it can move.
                    for (label_key, up, key, slot) in [
                        ("move_up", true, A11Y_KEY_MOVE_UP, 0usize),
                        ("move_down", false, A11Y_KEY_MOVE_DOWN, 1usize),
                    ] {
                        let enabled = if up { index > 0 } else { index + 1 < row_count };
                        if !enabled {
                            continue;
                        }
                        let state = Rc::clone(owner);
                        let action_env = row_env.clone();
                        let to = if up { index - 1 } else { index + 1 };
                        let node_id = register_edit_control_node(
                            renderer,
                            ctx,
                            key_base + key,
                            crate::localization::text(&subtree_env, label_key),
                            controls.as_ref().and_then(|controls| {
                                if up {
                                    controls.reorder_up
                                } else {
                                    controls.reorder_down
                                }
                            }),
                            &subtree_env,
                            Rc::new(RefCell::new(
                                move |_renderer: &mut crate::renderer::SemanticCore,
                                      _env: &Environment| {
                                    run_row_move(&state, &action_env, index, to)
                                },
                            )),
                        );
                        if let Some(node_id) = node_id {
                            list_node.push_child(node_id);
                            renderer.register_accessibility_focus_link(
                                &crate::renderer::InteractionKey::for_rc(
                                    owner,
                                    row_interaction_base + slot,
                                ),
                                node_id,
                            );
                        }
                    }
                }
            }
            if let Some(footer) = chrome.footer.clone() {
                let footer_rect = kurbo::Rect::new(
                    slot_rect.x0 + list_metrics.map_or(0.0, |m| m.horizontal_inset),
                    slot_rect.y1 - footer_height,
                    slot_rect.x1 - list_metrics.map_or(0.0, |m| m.horizontal_inset),
                    slot_rect.y1,
                );
                if let Some(node_id) = register_section_chrome_node(
                    renderer,
                    ctx,
                    key_base + A11Y_KEY_FOOTER,
                    footer,
                    false,
                    footer_rect,
                    &row_env,
                ) {
                    list_node.push_child(node_id);
                }
            }
        }
        if ctx.is_none() {
            state.item_cache.borrow_mut().end_frame();
        }
        let _ = renderer.register_accessibility_leaf(
            ctx,
            list_node,
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

/// Emits one list edit-control node — a row's delete button or one half of its
/// reorder handle — as a sibling of the row under the list node, at the bounds
/// the draw pass paints the control into (water-rs/hydrolysis#52).
///
/// The controls sit beside their row rather than inside it: under the row they
/// would be its innermost `Click` descendants, and the semantic runtime's row
/// activation resolves to exactly that child — turning "activate row" into
/// "delete row". The draw pass registers its pointer targets on the same
/// rects, so the node's `Click` and a physical tap reach the same handler.
///
/// `bounds` is `Some` only on the rendered walk — the semantic walk carries
/// no geometry — and the rendered walk must have it: emitting a control node
/// without the rect the draw pass painted would silently desynchronize the
/// two trees.
#[cfg(feature = "accessibility")]
fn register_edit_control_node(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    semantic_key: i64,
    label: String,
    bounds: Option<kurbo::Rect>,
    env: &Environment,
    action: crate::renderer::AccessibilityActivation,
) -> Option<AccessibilityNodeId> {
    let mut node = AccessibilityNode::new(
        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
    );
    node.set_label(label);
    node.add_action(AccessibilityAction::Focus);
    node.add_action(AccessibilityAction::Click);
    let target = Some(AccessibilityActionTarget::Activate { action });
    match ctx {
        Some(ctx) => renderer.register_accessibility_child_node_with_key(
            semantic_key,
            node,
            transformed_rect(
                ctx.hit_transform,
                bounds.expect(
                    "hydrolysis list edit-control node: the rendered walk always has list metrics",
                ),
            ),
            env,
            target,
        ),
        None => renderer.register_accessibility_child_node_with_key_semantic(
            semantic_key,
            node,
            env,
            target,
        ),
    }
}

/// Measures the scrollable viewport, using content size only for ideal queries.
pub(crate) fn measure_list_node(
    list: &ListConfig,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let intrinsic = measure_list_intrinsic(list, state, env, theme);
    ViewDimensions::new(LayoutSize::new(
        proposal.width.unwrap_or(intrinsic.width),
        proposal.height.unwrap_or(intrinsic.height),
    ))
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
        let theme = ctx.theme();
        list_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            Some(&theme),
            state,
            env,
        );
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
    let list_metrics = ctx.theme().list_metrics();
    // `min_row_height` is the floor every row is estimated and measured
    // against; unset, the theme's one-line height keeps the same values.
    let row_floor = state
        .borrow()
        .config
        .min_row_height
        .map_or(list_metrics.one_line_row_height, f64::from);
    // A 0 floor still needs a positive seed estimate — measured extents
    // replace it row by row anyway.
    let row_estimate = row_floor.max(1.0);
    state.borrow().prepare_rows(row_count, row_estimate);
    if state.borrow().resolve_sections(row_count, env) {
        // Row extents measured before the chrome was known are short by its
        // height, so drop them rather than drawing rows into a stale slot.
        state
            .borrow()
            .extent_index
            .borrow_mut()
            .reset(row_count, row_estimate, 0.0);
    }

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
        .apply_scroll_request(ctx.renderer_mut(), &handle, row_count, true);
    let mut metrics = handle.metrics();
    let needs_viewport_clip = metrics.max_y > 0.0;
    // Register before the rows flush: scroll-target dispatch walks the frame's
    // targets newest-first, so a scroll region inside a row wins the delta
    // until it hits its own edge, where it falls through to the list.
    let hit_transform = ctx.hit_transform;
    crate::widgets::scroll::register_scroll_wheel_target(
        ctx.renderer_mut(),
        hit_transform,
        viewport,
        &handle,
    );
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
    // A row's extent is derived from its content's measured size every frame —
    // the same transient measure `list_content_rect` consumes below — so a row
    // whose content re-measures differently is re-measured here and only here:
    // `set_measured` writes the identical extent back for unchanged rows and
    // rows outside the window are never touched (water-rs/hydrolysis#199).
    let mut extents_changed = false;
    let theme = ctx.theme();
    let min_row_height = state.borrow().config.min_row_height;
    let mut index = window.start;
    let mut end = window.end;
    loop {
        while index < end {
            // A list row is its own chrome: a button inside one is a row, not a
            // filled container floating on a screen. Buttons that picked a style
            // explicitly keep it.
            let mut row_env = env.clone();
            row_env.insert(waterui_controls::button::ButtonStyle::Plain);
            row_env.insert(crate::widgets::controls::button::ListRowChrome);
            let item = materialize_list_item(&contents, index, &row_env);
            let row_id = contents
                .get_id(index)
                .unwrap_or_else(|| panic!("hydrolysis List item {index} has no stable id"));
            let chrome = state.borrow().section_chrome(index);
            // A row's extent covers the section chrome it owns, so scroll offsets,
            // hit testing, and the visible window all account for it.
            let content_size =
                measure_transient_view_intrinsic(&item.content, ctx.state_mut(), &row_env, &theme);
            let row_height = list_row_height_for_content(
                f64::from(content_size.height),
                item.insets.as_ref(),
                min_row_height,
                list_metrics,
            ) + chrome.total_height(&list_metrics);
            {
                let state_ref = state.borrow();
                let mut extent_index = state_ref.extent_index.borrow_mut();
                extents_changed |= extent_index
                    .measured(index)
                    .is_none_or(|old| old.to_bits() != row_height.to_bits());
                extent_index.set_measured(index, row_height);
            }
            rows.push((index, row_id, item, y, row_height, content_size));
            y += row_height;
            index += 1;
        }
        // A re-measured row can pull the window's end either way: shrinkage
        // reveals rows the stale extents hid, and those rows must be resolved
        // into this frame's stack rather than leaving the viewport's tail
        // unpainted until the next refresh.
        let refreshed_end = state
            .borrow()
            .extent_index
            .borrow()
            .visible_window(metrics.offset_y, metrics.offset_y + viewport.height())
            .end;
        if refreshed_end <= end {
            break;
        }
        end = refreshed_end;
    }
    if extents_changed {
        // Everything downstream of this pass that reads extents — the
        // accessibility emit that ran before it, the scroll domain, the
        // indicators — was resolved against the stale values, so pull one
        // more frame rather than leaving them stale until an unrelated
        // refresh happens to arrive.
        ctx.renderer_mut().request_refresh();
    }
    let lifted_id = state.borrow().reorder.get().map(|reorder| reorder.id);
    if let Some(lifted_id) = lifted_id
        && let Some(position) = rows.iter().position(|(_, id, ..)| *id == lifted_id)
    {
        let lifted_row = rows.remove(position);
        rows.push(lifted_row);
    }
    let visible_ids: Vec<ListItemId> = rows.iter().map(|(_, id, ..)| *id).collect();

    for (index, row_id, item, resting_y, row_height, content_size) in rows {
        let row_env = env.clone();
        // The row's `ListItem` node claims whatever naming scope the content
        // carries — hoist the metadata off the view the same way
        // `list_accessibility` does, or the first leaf inside the subtree
        // would claim the row's explicit label for itself. The subtree then
        // flushes under the container-child env that strips that naming.
        #[cfg(feature = "accessibility")]
        let (item, row_env) = {
            let mut item = item;
            let (content, scoped) = hoist_accessibility_metadata(item.content, &row_env);
            item.content = content;
            (item, scoped)
        };
        #[cfg(feature = "accessibility")]
        let subtree_env =
            accessibility_container_child_environment(&row_env).unwrap_or_else(|| row_env.clone());
        #[cfg(not(feature = "accessibility"))]
        let subtree_env = row_env.clone();
        // Interaction slots per row: 0 and 1 are the reorder handle's up/down
        // press slots, 2 the delete control's, 3 the row's own selection
        // press, 4 the row's anchor — the key `list_accessibility` links the
        // row node to and this pass resolves it through below.
        let row_interaction_base = (i32::from(*row_id) as u32 as usize)
            .checked_mul(5)
            .expect("hydrolysis List interaction identity overflow");
        let chrome = state.borrow().section_chrome(index);
        let reorder_dy = state.borrow().reorder_offset_for(index, row_id, row_height);
        let swipe_dx = state.borrow().swipe_offset_for(row_id);
        // Where the row's slot is (the gap it occupies in the list), before the
        // row itself is displaced sideways by a swipe.
        let slot_rect = kurbo::Rect::new(
            viewport.x0,
            resting_y + reorder_dy,
            viewport.x1,
            resting_y + reorder_dy + row_height,
        );
        if slot_rect.y1 <= viewport.y0 || slot_rect.y0 >= viewport.y1 {
            continue;
        }
        let header_height = chrome.header_height(&list_metrics);
        let footer_height = chrome.footer_height(&list_metrics);
        // The slot covers the section chrome this row owns; the row itself is
        // the band left between that header and footer, and only that band
        // swipes — a section title is not part of the row that carries it.
        let row_slot = kurbo::Rect::new(
            slot_rect.x0,
            slot_rect.y0 + header_height,
            slot_rect.x1,
            slot_rect.y1 - footer_height,
        );
        {
            let theme = ctx.theme();
            let mut draw = ctx.draw_context();
            if swipe_dx != 0.0 {
                let threshold =
                    (row_slot.width() * SWIPE_DISMISS_POSITIONAL_THRESHOLD).max(f64::EPSILON);
                let progress = (swipe_dx.abs() / threshold).clamp(0.0, 1.0);
                theme.draw_list_swipe_dismiss_background(
                    &mut draw,
                    row_slot,
                    progress,
                    swipe_dx < 0.0,
                );
            }
        }
        // Everything the row draws — its background, controls and content —
        // rides the swipe displacement; only the revealed dismiss background
        // stays anchored to the slot.
        let row_rect = row_slot + kurbo::Vec2::new(swipe_dx, 0.0);
        let selected = state
            .borrow()
            .row_selection
            .as_ref()
            .map(|selection| {
                ctx.renderer_mut()
                    .read_signal(&selection.is_selected(row_id))
            })
            .unwrap_or(false);
        // The fill is the theme's own `SelectionContainer` token rather than a
        // `WidgetTheme` entry: the row's content already flips to
        // `SelectionForeground` against it (see `selection_themed` in the list
        // component), so the pair has to come from the same place.
        let selection_fill = selected.then(|| {
            ctx.renderer_mut()
                .read_signal(&color::SelectionContainer.resolve(&row_env).computed())
        });
        {
            let theme = ctx.theme();
            let mut draw = ctx.draw_context();
            theme.draw_list_row_background(&mut draw, row_rect, index % 2 == 1);
            if let Some(fill) = selection_fill {
                draw.fill(row_rect, fill);
            }
            if lifted_id == Some(row_id) {
                theme.draw_list_row_lifted(&mut draw, row_rect, REORDER_LIFT_ELEVATION);
            }
        }
        if let Some(header) = chrome.header.clone() {
            let header_rect = kurbo::Rect::new(
                slot_rect.x0 + list_metrics.horizontal_inset,
                slot_rect.y0,
                slot_rect.x1 - list_metrics.horizontal_inset,
                slot_rect.y0 + header_height,
            );
            draw_section_label(ctx, header, header_rect, true, &row_env);
        }
        if let Some(footer) = chrome.footer.clone() {
            let footer_rect = kurbo::Rect::new(
                slot_rect.x0 + list_metrics.horizontal_inset,
                slot_rect.y1 - footer_height,
                slot_rect.x1 - list_metrics.horizontal_inset,
                slot_rect.y1,
            );
            draw_section_label(ctx, footer, footer_rect, false, &row_env);
        }

        let deletable = ctx.renderer_mut().read_signal(&item.deletable);
        // The content's measured size was resolved in the resting-geometry pass
        // above — the same measure that wrote the row's extent this frame.
        let mut content_rect = list_content_rect(
            row_rect,
            list_metrics,
            item.insets.as_ref(),
            content_size,
            &row_env,
        );
        // Edit mode's trailing controls — the same computation the
        // accessibility emit uses to place their nodes.
        let controls = row_edit_controls(
            &list_metrics,
            row_rect,
            row_height,
            index,
            total_rows,
            editing && has_move,
            editing && deletable && has_delete,
        );

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

        // A selectable row owns the whole row rect as a press target: a
        // plain click selects it, the toggle modifier toggles it and Shift
        // extends the anchored range — while the controls and the content
        // flushed after it still take their own presses first. The press
        // belongs to the row's own view — the content sub-view's root — so the
        // ancestry check tells a gesture or tap registered inside the row (a
        // nested `on_tap`) from one attached to that same root: a descendant's
        // claims the press, the row's own handler coexists with it
        // (water-rs/hydrolysis#175).
        let mut content = Some(item.content);
        if let Some(selection) = state.borrow().row_selection.clone() {
            let hit_bounds = transformed_rect(ctx.hit_transform, row_rect);
            let key = crate::renderer::InteractionKey::for_rc(state, row_interaction_base + 3);
            let (_, press_slot, _) = ctx
                .renderer_mut()
                .bind_interaction_target(key, hit_bounds, &row_env);
            let row_owner = {
                let state_ref = state.borrow();
                let mut cache = state_ref.item_cache.borrow_mut();
                let subview = cache.entry(row_id, || {
                    content
                        .take()
                        .expect("hydrolysis list row sub-view missing")
                });
                subview.ensure_built(ctx.renderer_mut(), &subtree_env);
                subview.root_accessibility_identity()
            };
            if let Some(owner) = row_owner.as_ref() {
                ctx.renderer_mut().push_input_owner(owner);
            }
            ctx.renderer_mut().register_interactive_pointer_target(
                hit_bounds,
                press_slot,
                move |renderer: &mut crate::renderer::SemanticCore, _point, _env| {
                    selection.write(index, row_id, renderer.modifiers());
                    true
                },
            );
            if row_owner.is_some() {
                ctx.renderer_mut().pop_input_owner();
            }
        }

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

        if let Some(control_rect) = controls.reorder {
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
            let up_interaction = controls.reorder_up.map(|rect| {
                let hit_bounds = transformed_rect(ctx.hit_transform, rect);
                let key = crate::renderer::InteractionKey::for_rc(state, row_interaction_base);
                let (interaction, slot, _) = ctx
                    .renderer_mut()
                    .bind_interaction_target(key, hit_bounds, &row_env);
                (rect, hit_bounds, interaction, slot)
            });
            let down_interaction = controls.reorder_down.map(|rect| {
                let hit_bounds = transformed_rect(ctx.hit_transform, rect);
                let key = crate::renderer::InteractionKey::for_rc(state, row_interaction_base + 1);
                let (interaction, slot, _) = ctx
                    .renderer_mut()
                    .bind_interaction_target(key, hit_bounds, &row_env);
                (rect, hit_bounds, interaction, slot)
            });
            {
                let up_state = up_interaction
                    .as_ref()
                    .map(|(rect, _, interaction_state, _)| {
                        (
                            *rect,
                            local_interaction_state(*interaction_state, ctx.hit_transform),
                        )
                    });
                let down_state =
                    down_interaction
                        .as_ref()
                        .map(|(rect, _, interaction_state, _)| {
                            (
                                *rect,
                                local_interaction_state(*interaction_state, ctx.hit_transform),
                            )
                        });
                let theme = ctx.theme();
                let mut draw = ctx.draw_context();
                theme.draw_list_move_control(&mut draw, control_rect);
                if let Some((rect, state)) = up_state {
                    theme.draw_list_move_control_state_layer(&mut draw, rect, state);
                }
                if let Some((rect, state)) = down_state {
                    theme.draw_list_move_control_state_layer(&mut draw, rect, state);
                }
            }
            if let Some((_, hit_bounds, _, press_slot)) = up_interaction {
                let state = Rc::clone(state);
                let action_env = row_env.clone();
                ctx.renderer_mut().register_interactive_pointer_target(
                    hit_bounds,
                    press_slot,
                    move |_renderer, _point, _env| {
                        run_row_move(&state, &action_env, index, index - 1)
                    },
                );
            }
            if let Some((_, hit_bounds, _, press_slot)) = down_interaction {
                let state = Rc::clone(state);
                let action_env = row_env.clone();
                ctx.renderer_mut().register_interactive_pointer_target(
                    hit_bounds,
                    press_slot,
                    move |_renderer, _point, _env| {
                        run_row_move(&state, &action_env, index, index + 1)
                    },
                );
            }
        }

        if let Some(delete_rect) = controls.delete {
            let delete_hit_bounds = transformed_rect(ctx.hit_transform, delete_rect);
            let delete_key =
                crate::renderer::InteractionKey::for_rc(state, row_interaction_base + 2);
            let (delete_interaction, delete_press_slot, _) = ctx
                .renderer_mut()
                .bind_interaction_target(delete_key, delete_hit_bounds, &row_env);
            {
                let delete_interaction =
                    local_interaction_state(delete_interaction, ctx.hit_transform);
                let theme = ctx.theme();
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
                move |_renderer, _point, _env| run_row_delete(&state, &action_env, index),
            );
        }

        content_rect.x1 = content_rect.x1.min(controls.trailing_x);
        if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
            // Render the row content through a persistent node held in the per-widget
            // cache, keyed by stable row id, instead of re-dispatching it each frame.
            // The cache keeps a row's node only while it stays visible (built on first
            // appearance, evicted by `end_frame` once it scrolls out), so reactive row
            // content stays live across frames while virtualization is preserved. The
            // sub-view's own a11y emits *inside* the row: `list_accessibility` emitted
            // the row's `ListItem` node this frame and linked it to this row's
            // interaction key — parenting the flush under that node keeps every text,
            // control and image in the row a descendant of its `ListItem`. When the
            // row emitted no node (a hidden or otherwise suppressed row), the subtree
            // suppresses the same way the old flush-wide suppression did.
            #[cfg(feature = "accessibility")]
            let row_node_id =
                ctx.renderer_mut()
                    .focus_node_for_key(&crate::renderer::InteractionKey::for_rc(
                        state,
                        row_interaction_base + 4,
                    ));
            #[cfg(feature = "accessibility")]
            let row_parented = {
                if let Some(row_node_id) = row_node_id {
                    ctx.renderer_mut().push_accessibility_parent(row_node_id);
                    true
                } else {
                    ctx.renderer_mut().push_accessibility_suppression();
                    false
                }
            };
            let render_ctx = ctx.render_context();
            {
                let state_ref = state.borrow();
                let mut cache = state_ref.item_cache.borrow_mut();
                let subview = cache.entry(row_id, || {
                    content
                        .take()
                        .expect("hydrolysis list row sub-view missing")
                });
                subview.flush_in_rect(
                    ctx.renderer_mut(),
                    render_ctx,
                    &subtree_env,
                    bounded_proposal(content_rect),
                    content_rect,
                );
            }
            #[cfg(feature = "accessibility")]
            if row_parented {
                ctx.renderer_mut().pop_accessibility_parent();
            } else {
                ctx.renderer_mut().pop_accessibility_suppression();
            }
        }

        {
            let separator = kurbo::Rect::new(
                row_rect.x0 + list_metrics.divider_leading_inset,
                row_rect.y1 - 1.0,
                row_rect.x1 - list_metrics.divider_trailing_inset,
                row_rect.y1,
            );
            let theme = ctx.theme();
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

    if state
        .borrow()
        .pending_scroll
        .get()
        .is_some_and(|(_, index)| index < row_count)
    {
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
            .apply_scroll_request(ctx.renderer_mut(), &rebound, row_count, true);
        metrics = rebound.metrics();
        ctx.renderer_mut().frame_signals().request_refresh();
    }
    state.borrow().record_viewport_anchor(metrics, row_count);

    if needs_viewport_clip {
        ctx.pop_layer();
    }

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
    bounds: kurbo::Rect,
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

/// One row's edit-mode controls as the draw pass paints them: the reorder
/// handle, the up and down halves it presses through (each only toward a
/// direction the row can actually move), and the delete control left of them.
/// `trailing_x` is where the row's content resumes. `list_accessibility`
/// registers the same controls' accessibility nodes on these bounds, so a
/// `Click` and a pointer tap land identically (water-rs/hydrolysis#52).
struct RowEditControls {
    reorder: Option<kurbo::Rect>,
    reorder_up: Option<kurbo::Rect>,
    reorder_down: Option<kurbo::Rect>,
    delete: Option<kurbo::Rect>,
    trailing_x: f64,
}

fn row_edit_controls(
    metrics: &waterui_backend_core::widget::ListMetrics,
    row_rect: kurbo::Rect,
    slot_height: f64,
    index: usize,
    total_rows: usize,
    move_enabled: bool,
    delete_enabled: bool,
) -> RowEditControls {
    let mut controls = RowEditControls {
        reorder: None,
        reorder_up: None,
        reorder_down: None,
        delete: None,
        trailing_x: row_rect.x1 - 8.0,
    };
    let mut trailing_x = controls.trailing_x;
    if move_enabled {
        let control_width = metrics.move_control_width;
        let vertical_inset = metrics.trailing_control_vertical_inset;
        let control_height = (slot_height - vertical_inset * 2.0).max(vertical_inset * 2.0);
        let control_rect = kurbo::Rect::new(
            trailing_x - control_width,
            row_rect.y0 + vertical_inset,
            trailing_x,
            row_rect.y0 + vertical_inset + control_height,
        );
        trailing_x -= control_width + metrics.trailing_control_spacing;
        let half_height = control_rect.height() / 2.0;
        controls.reorder = Some(control_rect);
        controls.reorder_up = (index > 0).then(|| {
            kurbo::Rect::new(
                control_rect.x0,
                control_rect.y0,
                control_rect.x1,
                control_rect.y0 + half_height,
            )
        });
        controls.reorder_down = (index + 1 < total_rows).then(|| {
            kurbo::Rect::new(
                control_rect.x0,
                control_rect.y0 + half_height,
                control_rect.x1,
                control_rect.y1,
            )
        });
    }
    if delete_enabled {
        let delete_rect = kurbo::Rect::new(
            trailing_x - metrics.delete_control_width,
            row_rect.y0 + metrics.trailing_control_vertical_inset,
            trailing_x,
            row_rect.y1 - metrics.trailing_control_vertical_inset,
        );
        trailing_x = delete_rect.x0 - metrics.trailing_control_spacing;
        controls.delete = Some(delete_rect);
    }
    controls.trailing_x = trailing_x;
    controls
}

/// The delete control's one action — shared by its pointer target and its
/// accessibility `Click` node so both delete the row identically.
fn run_row_delete(state: &RefCell<ListRenderState>, env: &Environment, index: usize) -> bool {
    if let Some(action) = state.borrow().config.on_delete.as_ref() {
        (action)(env, index);
    }
    true
}

/// One discrete reorder step — shared by the move halves' pointer targets and
/// their accessibility `Click` nodes. The preserve-anchor latch stays set only
/// when the move dirtied the rows: it holds the viewport's index over the
/// membership reconcile so the moved row visibly travels.
fn run_row_move(
    state: &RefCell<ListRenderState>,
    env: &Environment,
    from: usize,
    to: usize,
) -> bool {
    state.borrow().preserve_anchor_index_once.set(true);
    if let Some(action) = state.borrow().config.on_move.as_ref() {
        (action)(env, Move::new(from, to));
    }
    if !state.borrow().rows_dirty.get() {
        state.borrow().preserve_anchor_index_once.set(false);
    }
    true
}

/// Swipe-to-dismiss across a whole row, committing `on_delete` once the row
/// travels past Material's positional threshold.
fn register_row_swipe_gesture(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ListRenderState>>,
    group: usize,
    row_id: ListItemId,
    binding: Rc<Cell<RowBinding>>,
    bounds: kurbo::Rect,
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
    bounds: kurbo::Rect,
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

/// Draws a section header or footer.
///
/// The chrome reads the `MutedForeground` theme token rather than naming a
/// colour, so a section title matches the platform's own secondary text on
/// every theme.
fn draw_section_label(
    ctx: &mut WidgetRenderContext<'_>,
    label: Text,
    bounds: kurbo::Rect,
    is_header: bool,
    env: &Environment,
) {
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return;
    }
    let styled = ctx
        .renderer_mut()
        .read_signal(&section_chrome_text(label, is_header).resolve(env).content);
    ctx.render_styled_text(
        styled,
        waterui_layout::stack::HorizontalAlignment::Leading,
        env,
        bounds,
    );
}

/// Applies the section chrome's own typography to a header or footer.
///
/// Section chrome is the platform's, not the app's: the title is styled by the
/// list, the way `UITableView` and Material section headers style theirs.
/// Only the text content comes from the app, and it stays reactive.
fn section_chrome_text(label: Text, is_header: bool) -> Text {
    let text = label.color(waterui_graphics::color::Color::new(
        waterui::theme::color::MutedForeground,
    ));
    if is_header {
        text.font(waterui_text::font::Subheadline)
    } else {
        text.font(waterui_text::font::Caption)
    }
}

fn list_content_rect(
    row_rect: kurbo::Rect,
    metrics: waterui_backend_core::widget::ListMetrics,
    insets: Option<&waterui_layout::padding::EdgeInsets>,
    content_size: waterui_core::layout::Size,
    env: &Environment,
) -> kurbo::Rect {
    // Rows propose their full inset width to the content; horizontal
    // alignment belongs to the content itself (composite items cannot be
    // statically classified as stretching, and interactive rows must keep a
    // full-width hit target). The row's `insets` replace the theme's row
    // insets edge for edge; unset, the theme's symmetric inset keeps the
    // same rect.
    let (leading_inset, trailing_inset) = insets.map_or(
        (metrics.horizontal_inset, metrics.horizontal_inset),
        |insets| (f64::from(insets.leading()), f64::from(insets.trailing())),
    );
    let vertical_insets = insets.map_or(metrics.vertical_inset * 2.0, |insets| {
        f64::from(insets.top() + insets.bottom())
    });
    // `EdgeInsets` is logical: leading opens the content on the side the
    // layout direction starts from.
    let (left_inset, right_inset) = if waterui_core::layout::layout_direction(env)
        .snapshot()
        .is_right_to_left()
    {
        (trailing_inset, leading_inset)
    } else {
        (leading_inset, trailing_inset)
    };
    let x0 = row_rect.x0 + left_inset;
    let x1 = row_rect.x1 - right_inset;
    let available_height = (row_rect.height() - vertical_insets).max(0.0);
    let height = f64::from(content_size.height).min(available_height);
    let y0 = row_rect.y0 + (row_rect.height() - height) * 0.5;
    kurbo::Rect::new(x0, y0, x1, y0 + height)
}

/// Emits a retained list's accessibility tree for the semantic walk — the same
/// nodes `list_accessibility` registers, with no bounds and every row present.
#[cfg(feature = "accessibility")]
pub(crate) fn emit_list_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<ListRenderState>>,
    env: &Environment,
) {
    list_accessibility(renderer, None, None, state, env);
}
