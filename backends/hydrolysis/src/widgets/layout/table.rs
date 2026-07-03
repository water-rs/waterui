use std::cell::RefCell;
use std::rc::Rc;

#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::lazy::{
    resolve_table_visible_rows, resolve_visible_column_window, table_metrics_from_slot,
};
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, VisibleSubviewCache,
    WidgetRenderContext, measure_table_metrics, refresh_table_slot_baseline, table_data_cell_rect,
    table_header_cell_rect, transformed_rect, update_table_slot_visible_cell_widths,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::Signal;
use waterui::component::table::{TableColumn, TableConfig};
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::views::Views;
use waterui_core::{AnyView, Environment, Native};
use waterui_layout::scroll::Axis as ScrollAxis;

use crate::widgets::{draw_scroll_indicators, inset_rect, widget_theme};

/// The cache key for a table's retained cell sub-views. The table re-reads its
/// `Vec<TableColumn>` each frame with no stable per-column id, so cells are keyed by
/// their position in the visible window (the same index model the table already uses
/// for its column-width / row-count slot caches). A column header and the data cells
/// in that column are distinct entries.
#[derive(Clone, PartialEq, Eq, Hash)]
enum TableCellKey {
    /// A column header cell, keyed by column index.
    Header(usize),
    /// A data cell, keyed by `(column_index, row_index)`.
    Cell(usize, usize),
}

/// Retained state for a table `Widget` node: the consumed config plus a per-widget
/// cache of the visible header/data cells' content sub-views, keyed by
/// [`TableCellKey`]. Only the cells in the current visible row/column window are
/// built and retained (evicted once they scroll out), so the table stays
/// virtualized — cost is bounded by visible cells.
pub(crate) struct TableRenderState {
    pub(crate) config: TableConfig,
    /// Content sub-views for the cells currently in view, keyed by [`TableCellKey`]
    /// so a steady scroll reuses each visible cell's node (keeping its reactive
    /// content live) and only builds cells entering the window.
    item_cache: RefCell<VisibleSubviewCache<TableCellKey>>,
}

impl TableRenderState {
    pub(crate) fn from_config(config: TableConfig) -> Self {
        Self {
            config,
            item_cache: RefCell::new(VisibleSubviewCache::new()),
        }
    }
}

impl HydroNativeView for Native<TableConfig> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_table_intrinsic(view.as_inner(), state, env)
    }
}

/// Measures a table's intrinsic size from its (reactive) columns.
fn measure_table_intrinsic(table: &TableConfig, state: &mut HydroState, env: &Environment) -> LayoutSize {
    let columns = table.columns.get();
    if columns.is_empty() {
        return LayoutSize::zero();
    }
    let metrics = measure_table_metrics(&columns, state, env);
    LayoutSize::new(metrics.table_width as f32, metrics.table_height as f32)
}

/// Emits a table's accessibility tree from its config — and crucially binds the
/// table's scroll handle (`bind_scroll_handle` pushes it pending), which the
/// subsequent `render_table_parts` consumes via `take_pending_scroll_handle`. The
/// dispatch wrapper calls this before `render`, and the retained `Widget`-node
/// path calls it from `render_table_node` for the same ordering, so the handle is
/// always bound before the draw whether or not accessibility nodes are emitted.
pub(crate) fn table_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    table: &TableConfig,
    env: &Environment,
) {
    let columns = renderer.read_signal(&table.columns);
    if columns.is_empty() {
        return;
    }
    let slot_index = renderer.lazy.lazy_table_controller.bind();
    {
        let (slot, state) = renderer.table_slot_and_state_mut(slot_index);
        refresh_table_slot_baseline(&columns, slot, state, env);
    }
    let viewport = ctx.bounds;
    let layout_metrics = widget_theme(env).table_metrics();
    let table_metrics = {
        let slot = &renderer.lazy.lazy_table_controller.slots[slot_index];
        table_metrics_from_slot(slot, layout_metrics)
    };
    let handle = renderer.bind_scroll_handle(
        ScrollAxis::All,
        viewport.width(),
        viewport.height(),
        table_metrics.table_width.max(viewport.width()),
        table_metrics.table_height.max(viewport.height()),
    );
    #[cfg(feature = "accessibility")]
    {
        let scroll_metrics = handle.metrics();
        let row_window = {
            let slot = &renderer.lazy.lazy_table_controller.slots[slot_index];
            resolve_table_visible_rows(
                scroll_metrics.offset_y,
                viewport.height(),
                slot.max_rows,
                layout_metrics,
            )
        };
        let mut column_window = {
            let slot = &renderer.lazy.lazy_table_controller.slots[slot_index];
            resolve_visible_column_window(
                &slot.column_widths,
                scroll_metrics.offset_x,
                scroll_metrics.offset_x + viewport.width(),
            )
        };
        {
            {
                let (slot, state) = renderer.table_slot_and_state_mut(slot_index);
                update_table_slot_visible_cell_widths(
                    &columns,
                    slot,
                    row_window,
                    column_window,
                    state,
                    env,
                );
            }
            let slot = &renderer.lazy.lazy_table_controller.slots[slot_index];
            column_window = resolve_visible_column_window(
                &slot.column_widths,
                scroll_metrics.offset_x,
                scroll_metrics.offset_x + viewport.width(),
            );
        }
        let mut table_node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Table),
        );
        let table_label = renderer.resolve_accessibility_label(env, None);
        if let Some(label) = table_label {
            table_node.set_label(label);
        }
        table_node.set_scroll_x(scroll_metrics.offset_x);
        table_node.set_scroll_x_min(0.0);
        table_node.set_scroll_x_max(scroll_metrics.max_x);
        table_node.set_scroll_y(scroll_metrics.offset_y);
        table_node.set_scroll_y_min(0.0);
        table_node.set_scroll_y_max(scroll_metrics.max_y);
        table_node.add_action(AccessibilityAction::ScrollLeft);
        table_node.add_action(AccessibilityAction::ScrollRight);
        table_node.add_action(AccessibilityAction::ScrollUp);
        table_node.add_action(AccessibilityAction::ScrollDown);

        let origin_x = viewport.x0 - scroll_metrics.offset_x;
        let origin_y = viewport.y0 - scroll_metrics.offset_y;
        let mut x_offset = column_window.leading_offset;
        for (column_index, column) in columns
            .iter()
            .enumerate()
            .take(column_window.end)
            .skip(column_window.start)
        {
            let width =
                renderer.lazy.lazy_table_controller.slots[slot_index].column_widths[column_index];
            let header_cell =
                table_header_cell_rect(origin_x, origin_y, x_offset, width, layout_metrics);
            let header_view = AnyView::new(column.label());
            let mut header_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ColumnHeader),
            );
            let default_label = renderer.accessibility_label_from_view(&header_view, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                header_node.set_label(label);
            }
            header_node.add_action(AccessibilityAction::Focus);
            if let Some(header_node_id) = renderer.register_accessibility_child_node(
                header_node,
                transformed_rect(ctx.hit_transform, header_cell),
                env,
                None,
            ) {
                table_node.push_child(header_node_id);
            }
            let rows = column.rows();
            for row_index in row_window.start..row_window.end {
                let cell_rect = table_data_cell_rect(
                    origin_x,
                    origin_y,
                    x_offset,
                    width,
                    row_index,
                    layout_metrics,
                );
                if let Some(cell) = rows.get_view(row_index) {
                    let cell_view = AnyView::new(cell);
                    let mut cell_node = AccessibilityNode::new(
                        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Cell),
                    );
                    let default_label = renderer.accessibility_label_from_view(&cell_view, env);
                    let label = renderer.resolve_accessibility_label(env, default_label);
                    if let Some(label) = label {
                        cell_node.set_label(label);
                    }
                    cell_node.add_action(AccessibilityAction::Focus);
                    if let Some(cell_node_id) = renderer.register_accessibility_child_node(
                        cell_node,
                        transformed_rect(ctx.hit_transform, cell_rect),
                        env,
                        None,
                    ) {
                        table_node.push_child(cell_node_id);
                    }
                }
            }
            x_offset += width;
        }

        let _ = renderer.register_accessibility_node(
            table_node,
            transformed_rect(ctx.hit_transform, viewport),
            env,
            Some(AccessibilityActionTarget::Scroll {
                handle: handle.clone(),
                axis: ScrollAxis::All,
            }),
        );
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = handle;
    }
}

/// Measures a table leaf from its config (intrinsic-sized; proposal-independent).
pub(crate) fn measure_table_node(
    table: &TableConfig,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    ViewDimensions::new(measure_table_intrinsic(table, state, env))
}

/// Renders a retained table leaf every flush. Table accessibility is not
/// render-driven and additionally binds the scroll handle the draw consumes, so
/// this node always runs `table_accessibility` first (mirroring the dispatch
/// wrapper's `accessibility`-then-`render` order); when the table is
/// accessibility-hidden it runs that step inside a suppression scope so the
/// handle is still bound while the a11y nodes are suppressed.
pub(crate) fn render_table_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<TableRenderState>>,
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
        table_accessibility(ctx.renderer_mut(), render_ctx, &state.borrow().config, env);
    }
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().pop_accessibility_suppression();
    }
    render_table_parts(ctx, state, env);
}

pub(crate) fn render_table_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<TableRenderState>>,
    env: &Environment,
) {
    let columns_signal = state.borrow().config.columns.clone();
    let columns: Vec<TableColumn> = ctx.renderer_mut().read_signal(&columns_signal);
    if columns.is_empty() {
        return;
    }
    let viewport = ctx.bounds;
    let handle = ctx
        .renderer_mut()
        .take_pending_scroll_handle("render_table");
    let scroll_metrics = handle.metrics();
    let slot_index = ctx.renderer_mut().lazy.lazy_table_controller.bind();
    let layout_metrics = widget_theme(env).table_metrics();
    {
        let (slot, state) = ctx.renderer_mut().table_slot_and_state_mut(slot_index);
        refresh_table_slot_baseline(&columns, slot, state, env);
    }
    let row_window = {
        let slot = &ctx.renderer_mut().lazy.lazy_table_controller.slots[slot_index];
        resolve_table_visible_rows(
            scroll_metrics.offset_y,
            viewport.height(),
            slot.max_rows,
            layout_metrics,
        )
    };
    let mut column_window = {
        let slot = &ctx.renderer_mut().lazy.lazy_table_controller.slots[slot_index];
        resolve_visible_column_window(
            &slot.column_widths,
            scroll_metrics.offset_x,
            scroll_metrics.offset_x + viewport.width(),
        )
    };
    {
        let (slot, state) = ctx.renderer_mut().table_slot_and_state_mut(slot_index);
        update_table_slot_visible_cell_widths(
            &columns,
            slot,
            row_window,
            column_window,
            state,
            env,
        );
    }
    {
        let slot = &ctx.renderer_mut().lazy.lazy_table_controller.slots[slot_index];
        column_window = resolve_visible_column_window(
            &slot.column_widths,
            scroll_metrics.offset_x,
            scroll_metrics.offset_x + viewport.width(),
        );
    }
    let table_metrics = {
        let slot = &ctx.renderer_mut().lazy.lazy_table_controller.slots[slot_index];
        table_metrics_from_slot(slot, layout_metrics)
    };

    ctx.push_layer_rect(1.0, viewport);

    let origin_x = viewport.x0 - scroll_metrics.offset_x;
    let origin_y = viewport.y0 - scroll_metrics.offset_y;
    {
        let table_rect = vello::kurbo::Rect::new(
            origin_x,
            origin_y,
            origin_x + table_metrics.table_width,
            origin_y + table_metrics.table_height,
        );
        let header_rect = vello::kurbo::Rect::new(
            origin_x,
            origin_y,
            origin_x + table_metrics.table_width,
            origin_y + layout_metrics.header_height,
        );
        let theme = widget_theme(env);
        let mut draw = ctx.draw_context();
        theme.draw_table_background(&mut draw, table_rect);
        theme.draw_table_header_background(&mut draw, header_rect);
    }

    // Begin a fresh frame for the per-cell content sub-view cache: only cells touched
    // in the visible loops below survive `end_frame`, preserving virtualization.
    state.borrow().item_cache.borrow_mut().begin_frame();
    let mut x_offset = column_window.leading_offset;
    for (column_index, column) in columns
        .iter()
        .enumerate()
        .take(column_window.end)
        .skip(column_window.start)
    {
        let width = table_metrics.column_widths[column_index];
        let header_cell =
            table_header_cell_rect(origin_x, origin_y, x_offset, width, layout_metrics);
        let cell_horizontal_inset = layout_metrics.cell_horizontal_padding * 0.5;
        // Render the header cell through a persistent node held in the per-widget cache,
        // keyed by column index, instead of re-dispatching it each frame. Cell a11y is
        // emitted by `table_accessibility`, so suppress the sub-view's own a11y
        // (matching the old `dispatch_in_rect_without_accessibility`).
        let header_view = AnyView::new(column.label());
        let header_rect = inset_rect(
            header_cell,
            cell_horizontal_inset,
            layout_metrics.cell_vertical_inset,
        );
        flush_cell_subview(ctx, state, env, TableCellKey::Header(column_index), header_view, header_rect);

        let rows = column.rows();
        for row_index in row_window.start..row_window.end {
            let cell_rect = table_data_cell_rect(
                origin_x,
                origin_y,
                x_offset,
                width,
                row_index,
                layout_metrics,
            );
            if let Some(cell) = rows.get_view(row_index) {
                let cell_view = AnyView::new(cell);
                let inset = inset_rect(
                    cell_rect,
                    cell_horizontal_inset,
                    layout_metrics.cell_vertical_inset,
                );
                flush_cell_subview(
                    ctx,
                    state,
                    env,
                    TableCellKey::Cell(column_index, row_index),
                    cell_view,
                    inset,
                );
            }
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_table_cell_border(&mut draw, cell_rect);
        }

        let separator_from = vello::kurbo::Point::new(origin_x + x_offset + width, origin_y);
        let separator_to = vello::kurbo::Point::new(
            origin_x + x_offset + width,
            origin_y + table_metrics.table_height,
        );
        let theme = widget_theme(env);
        let mut draw = ctx.draw_context();
        theme.draw_table_column_separator(&mut draw, separator_from, separator_to);
        x_offset += width;
    }
    // Evict content sub-views for cells no longer in the visible window.
    state.borrow().item_cache.borrow_mut().end_frame();

    ctx.pop_layer();

    let handle_for_input = handle.clone();
    let hit_transform = ctx.hit_transform;
    ctx.renderer_mut().register_scroll_target(
        transformed_rect(hit_transform, viewport),
        handle.clone(),
        move |dx, dy, is_line_delta| handle_for_input.apply_scroll_delta(dx, dy, is_line_delta),
    );
    draw_scroll_indicators(ctx, env, viewport, scroll_metrics, ScrollAxis::All);
}

/// Render one table cell's content through the per-widget [`VisibleSubviewCache`],
/// keyed by [`TableCellKey`], instead of re-dispatching it each frame. The cache
/// keeps a cell's node only while it stays visible (built on first appearance,
/// evicted by `end_frame` once it scrolls out), so reactive cell content stays live
/// across frames while virtualization is preserved. Cell a11y is emitted by
/// `table_accessibility`, so the sub-view's own a11y is suppressed (matching the old
/// `dispatch_in_rect_without_accessibility`).
fn flush_cell_subview(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<TableRenderState>>,
    env: &Environment,
    key: TableCellKey,
    view: AnyView,
    rect: vello::kurbo::Rect,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    #[cfg(feature = "accessibility")]
    ctx.renderer_mut().push_accessibility_suppression();
    let render_ctx = ctx.render_context();
    {
        let state_ref = state.borrow();
        let mut cache = state_ref.item_cache.borrow_mut();
        let subview = cache.entry(key, move || view);
        subview.flush_in_rect(ctx.renderer_mut(), render_ctx, env, rect);
    }
    #[cfg(feature = "accessibility")]
    ctx.renderer_mut().pop_accessibility_suppression();
}
