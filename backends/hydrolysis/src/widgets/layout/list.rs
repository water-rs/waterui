use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
    /// Per-row section chrome, resolved once per membership change for a list
    /// whose rows carry section markers.
    sections: RefCell<Vec<RowSectionChrome>>,
    /// Row count the resolved chrome was built for, so a list that renders
    /// before its rows exist re-resolves once they do.
    sections_resolved_for: Cell<Option<usize>>,
    /// Collection membership watcher.
    _guard: BoxWatcherGuard,
}

/// The section chrome one row is responsible for drawing.
///
/// A marker opens a section on the row that carries it, so that row owns the
/// header. The footer closes the section on a *different* row — the last one
/// before the next marker — which the draw loop cannot discover on its own
/// while only part of the list is realized. Resolving both onto rows up front
/// keeps every row's height a local question again.
#[derive(Clone, Default)]
struct RowSectionChrome {
    header: Option<waterui_core::Str>,
    footer: Option<waterui_core::Str>,
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
            sections: RefCell::new(Vec::new()),
            sections_resolved_for: Cell::new(None),
            _guard: guard,
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
        let mut open_section: Option<(usize, Option<waterui_core::Str>)> = None;
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

    fn prepare_rows(&self, len: usize, estimate: f64) {
        let dirty = self.rows_dirty.replace(false);
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
        let offset = self.extent_index.borrow().offset_of(index);
        let _ = handle.scroll_to(0.0, offset);
        let extent_index = self.extent_index.borrow();
        let Some(extent) = extent_index.measured(index) else {
            return;
        };
        let metrics = handle.metrics();
        let row_start = extent_index.offset_of(index);
        let row_end = row_start + extent;
        let viewport_end = metrics.offset_y + metrics.viewport_height;
        if row_end > metrics.offset_y && row_start < viewport_end {
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
    if state.borrow().resolve_sections(row_count, env) {
        // Row extents measured before the chrome was known are short by its
        // height, so drop them rather than drawing rows into a stale slot.
        state
            .borrow()
            .extent_index
            .borrow_mut()
            .reset(row_count, list_metrics.one_line_row_height, 0.0);
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
    // Begin a fresh frame for the per-row content sub-view cache: only rows touched
    // in the visible loop below survive `end_frame`, preserving virtualization.
    state.borrow().item_cache.borrow_mut().begin_frame();
    let mut y = viewport.y0 - metrics.offset_y + window.leading_offset;
    for index in window.start..window.end {
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
        let row_interaction_base = (i32::from(*row_id) as u32 as usize)
            .checked_mul(3)
            .expect("hydrolysis List interaction identity overflow");
        let chrome = state.borrow().section_chrome(index);
        // A row's extent covers the section chrome it owns, so scroll offsets,
        // hit testing, and the visible window all account for it.
        let row_height = {
            let cached_extent = state.borrow().extent_index.borrow().measured(index);
            if let Some(extent) = cached_extent {
                extent
            } else {
                let extent = measure_list_item_row_height(&item, ctx.state_mut(), &row_env)
                    + chrome.total_height(&list_metrics);
                state
                    .borrow()
                    .extent_index
                    .borrow_mut()
                    .set_measured(index, extent);
                extent
            }
        };
        let slot_rect = vello::kurbo::Rect::new(viewport.x0, y, viewport.x1, y + row_height);
        y += row_height;
        if slot_rect.y1 <= viewport.y0 || slot_rect.y0 >= viewport.y1 {
            continue;
        }
        let header_height = chrome.header_height(&list_metrics);
        let footer_height = chrome.footer_height(&list_metrics);
        let row_rect = vello::kurbo::Rect::new(
            slot_rect.x0,
            slot_rect.y0 + header_height,
            slot_rect.x1,
            slot_rect.y1 - footer_height,
        );
        {
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_list_row_background(&mut draw, row_rect, index % 2 == 1);
        }
        if let Some(header) = chrome.header.clone() {
            let header_rect = vello::kurbo::Rect::new(
                slot_rect.x0 + list_metrics.horizontal_inset,
                slot_rect.y0,
                slot_rect.x1 - list_metrics.horizontal_inset,
                slot_rect.y0 + header_height,
            );
            draw_section_label(ctx, header, header_rect, true, &row_env);
        }
        if let Some(footer) = chrome.footer.clone() {
            let footer_rect = vello::kurbo::Rect::new(
                slot_rect.x0 + list_metrics.horizontal_inset,
                slot_rect.y1 - footer_height,
                slot_rect.x1 - list_metrics.horizontal_inset,
                slot_rect.y1,
            );
            draw_section_label(ctx, footer, footer_rect, false, &row_env);
        }

        let deletable = ctx.renderer_mut().read_signal(&item.deletable);
        let content_size = measure_view_intrinsic(&item.content, ctx.state_mut(), &row_env);
        let mut content_rect = list_content_rect(row_rect, list_metrics, content_size);
        let mut trailing_x = row_rect.x1 - 8.0;

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

/// Draws a section header or footer.
///
/// The chrome reads the `MutedForeground` theme token rather than naming a
/// colour, so a section title matches the platform's own secondary text on
/// every theme.
fn draw_section_label(
    ctx: &mut WidgetRenderContext<'_>,
    label: waterui_core::Str,
    bounds: vello::kurbo::Rect,
    is_header: bool,
    env: &Environment,
) {
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return;
    }
    let text = waterui_text::Text::new(label).color(waterui_graphics::color::Color::new(
        waterui::theme::color::MutedForeground,
    ));
    let text = if is_header {
        text.font(waterui_text::font::Subheadline)
    } else {
        text.font(waterui_text::font::Caption)
    };
    let styled = ctx.renderer_mut().read_signal(&text.resolve(env).content);
    ctx.render_styled_text(
        styled,
        waterui_layout::stack::HorizontalAlignment::Leading,
        env,
        bounds,
    );
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
