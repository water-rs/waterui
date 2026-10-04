use crate::renderer::bounded_proposal;
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::lazy::{
    LazyTableSlot, resolve_table_visible_rows, resolve_visible_column_window,
    table_metrics_from_slot,
};
#[cfg(feature = "accessibility")]
use crate::renderer::lazy::{VisibleColumnWindow, VisibleIndexWindow};
#[cfg(feature = "accessibility")]
use crate::renderer::transformed_rect;
use crate::renderer::{
    HydroNativeView, HydroState, MeasuredTableMetrics, RenderContext, VisibleSubviewCache,
    WidgetRenderContext, measure_table_metrics, refresh_table_slot_baseline, table_data_cell_rect,
    table_header_cell_rect, update_table_slot_visible_cell_widths,
};
use crate::scroll::ScrollHandle;
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::Signal;
use waterui::component::table::{TableColumn, TableConfig};
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::views::{ViewSnapshot, Views};
use waterui_core::{AnyView, Environment, Native};
use waterui_layout::scroll::Axis as ScrollAxis;

use crate::widgets::{draw_scroll_indicators, inset_rect};

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
pub struct TableRenderState {
    pub(crate) config: TableConfig,
    /// Column metrics belong to this semantic table node.
    slot: RefCell<LazyTableSlot>,
    /// Scroll state belongs to this semantic table node.
    scroll: RefCell<Option<ScrollHandle>>,
    /// Content sub-views for the cells currently in view, keyed by [`TableCellKey`]
    /// so a steady scroll reuses each visible cell's node (keeping its reactive
    /// content live) and only builds cells entering the window.
    item_cache: RefCell<VisibleSubviewCache<TableCellKey>>,
}

impl TableRenderState {
    pub(crate) fn from_config(config: TableConfig) -> Self {
        Self {
            config,
            slot: RefCell::new(LazyTableSlot::default()),
            scroll: RefCell::new(None),
            item_cache: RefCell::new(VisibleSubviewCache::new()),
        }
    }

    #[expect(
        clippy::option_if_let_else,
        reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
    )]
    fn bind_scroll(
        &self,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
    ) -> ScrollHandle {
        let mut scroll = self.scroll.borrow_mut();
        if let Some(handle) = scroll.as_mut() {
            handle.rebind(
                ScrollAxis::All,
                viewport_width,
                viewport_height,
                content_width,
                content_height,
            )
        } else {
            let handle = ScrollHandle::new(
                ScrollAxis::All,
                viewport_width,
                viewport_height,
                content_width,
                content_height,
                None,
            );
            *scroll = Some(handle.clone());
            handle
        }
    }
}

impl HydroNativeView for Native<TableConfig> {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_table_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Measures a table's intrinsic size from its (reactive) columns.
fn measure_table_intrinsic(
    table: &TableConfig,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> LayoutSize {
    let columns = table.columns.snapshot();
    if columns.is_empty() {
        return LayoutSize::zero();
    }
    let metrics = measure_table_metrics(&columns, state, env, theme);
    LayoutSize::new(
        crate::num_cast::f64_as_f32(metrics.table_width),
        crate::num_cast::f64_as_f32(metrics.table_height),
    )
}

/// Emits a table's accessibility tree from its node-owned retained state.
///
/// The rendered flush passes its [`RenderContext`] and theme: column widths and
/// the visible windows are real. The semantic walk passes `None` for both and
/// emits every column header and every cell with no bounds — a table's cell
/// contents are already bounded by its data — while the scroll handle tracks
/// offsets in cell units.
// the names follow the domain vocabulary (header/cell/row groups); renaming would obscure rather than clarify
#[allow(clippy::similar_names)]
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn table_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn crate::engine::WidgetTheme>>,
    state: &Rc<RefCell<TableRenderState>>,
    env: &Environment,
) {
    let columns_signal = state.borrow().config.columns.clone();
    let columns = renderer.read_signal(&columns_signal);
    if columns.is_empty() {
        return;
    }
    {
        let state_ref = state.borrow();
        let mut slot = state_ref.slot.borrow_mut();
        if let Some(theme) = theme {
            refresh_table_slot_baseline(&columns, &mut slot, renderer.state_mut(), env, theme);
        } else {
            // The semantic walk has no theme to measure against: it keeps the
            // row count current so the scroll handle and cell emission cover
            // the full table.
            slot.max_rows = columns
                .iter()
                .map(|column| column.rows().len().snapshot())
                .max()
                .unwrap_or(0);
        }
    }
    let viewport = ctx.map_or(kurbo::Rect::ZERO, |ctx| ctx.bounds);
    let layout_metrics = theme.map(|theme| theme.table_metrics());
    let table_metrics = {
        let state_ref = state.borrow();
        match layout_metrics {
            Some(layout_metrics) => {
                table_metrics_from_slot(&state_ref.slot.borrow(), layout_metrics)
            }
            // The semantic scroll domain is in cell units: one unit per
            // column and per row, so ScrollLeft/ScrollDown move the emission
            // window by whole cells without a layout pass.
            None => MeasuredTableMetrics {
                column_widths: vec![1.0; columns.len()],
                table_width: crate::num_cast::usize_as_f64(columns.len()),
                table_height: crate::num_cast::usize_as_f64(state_ref.slot.borrow().max_rows),
            },
        }
    };
    let handle = state.borrow().bind_scroll(
        viewport.width(),
        viewport.height(),
        table_metrics.table_width.max(viewport.width()),
        table_metrics.table_height.max(viewport.height()),
    );
    #[cfg(feature = "accessibility")]
    {
        let scroll_metrics = handle.metrics();
        let rendered = layout_metrics.is_some();
        let row_window = {
            let state_ref = state.borrow();
            let slot = state_ref.slot.borrow();
            match layout_metrics {
                Some(layout_metrics) => resolve_table_visible_rows(
                    scroll_metrics.offset_y,
                    viewport.height(),
                    slot.max_rows,
                    layout_metrics,
                ),
                None => VisibleIndexWindow {
                    start: 0,
                    end: slot.max_rows,
                    leading_offset: 0.0,
                },
            }
        };
        let mut column_window = {
            let state_ref = state.borrow();
            let slot = state_ref.slot.borrow();
            if rendered {
                resolve_visible_column_window(
                    &slot.column_widths,
                    scroll_metrics.offset_x,
                    scroll_metrics.offset_x + viewport.width(),
                )
            } else {
                VisibleColumnWindow {
                    start: 0,
                    end: columns.len(),
                    leading_offset: 0.0,
                }
            }
        };
        if let Some(layout_metrics) = layout_metrics {
            let _ = layout_metrics;
            let theme = theme.expect("hydrolysis rendered table accessibility requires a theme");
            {
                let state_ref = state.borrow();
                let mut slot = state_ref.slot.borrow_mut();
                update_table_slot_visible_cell_widths(
                    &columns,
                    &mut slot,
                    row_window,
                    column_window,
                    renderer.state_mut(),
                    env,
                    theme,
                );
            }
            let state_ref = state.borrow();
            let slot = state_ref.slot.borrow();
            column_window = resolve_visible_column_window(
                &slot.column_widths,
                scroll_metrics.offset_x,
                scroll_metrics.offset_x + viewport.width(),
            );
        }
        let mut table_node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Table,
            ));
        let table_label = renderer.resolve_accessibility_label(env, None);
        if let Some(label) = table_label {
            table_node.set_label(label);
        }
        if let Some(value) = renderer.resolve_accessibility_value(env, None) {
            table_node.set_value(value);
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
            let width = table_metrics.column_widths[column_index];
            let header_cell = layout_metrics.map_or(kurbo::Rect::ZERO, |m| {
                table_header_cell_rect(origin_x, origin_y, x_offset, width, m)
            });
            let header_view = AnyView::new(column.label());
            let mut header_node =
                AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                    env,
                    AccessibilityNodeRole::ColumnHeader,
                ));
            let default_label = renderer.accessibility_label_from_view(&header_view, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                header_node.set_label(label);
            }
            header_node.add_action(AccessibilityAction::Focus);
            let column_key = i64::try_from(column_index)
                .expect("hydrolysis table column index exceeds accessibility identity range");
            let header_key = column_key
                .checked_add(1)
                .and_then(i64::checked_neg)
                .expect("hydrolysis table header accessibility identity overflow");
            let header_node_id = match ctx {
                Some(ctx) => renderer.register_accessibility_child_node_with_key(
                    header_key,
                    header_node,
                    transformed_rect(ctx.hit_transform, header_cell),
                    env,
                    None,
                ),
                None => renderer.register_accessibility_child_node_with_key_semantic(
                    header_key,
                    header_node,
                    env,
                    None,
                ),
            };
            if let Some(header_node_id) = header_node_id {
                table_node.push_child(header_node_id);
            }
            // One immutable row set per column for the whole window: the
            // cells this emit reads stay coherent with each other even if a
            // cell's own content mutates the source mid-pass.
            let rows = column.rows().snapshot();
            for row_index in row_window.start..row_window.end {
                let cell_rect = layout_metrics.map_or(kurbo::Rect::ZERO, |m| {
                    table_data_cell_rect(origin_x, origin_y, x_offset, width, row_index, m)
                });
                if let Some(cell) = rows.get_view(row_index) {
                    let cell_view = AnyView::new(cell);
                    let mut cell_node = AccessibilityNode::new(
                        crate::renderer::SemanticCore::resolve_accessibility_role(
                            env,
                            AccessibilityNodeRole::Cell,
                        ),
                    );
                    let default_label = renderer.accessibility_label_from_view(&cell_view, env);
                    let label = renderer.resolve_accessibility_label(env, default_label);
                    if let Some(label) = label {
                        cell_node.set_label(label);
                    }
                    cell_node.add_action(AccessibilityAction::Focus);
                    let row_key = i64::try_from(row_index)
                        .expect("hydrolysis table row index exceeds accessibility identity range");
                    let diagonal = column_key
                        .checked_add(row_key)
                        .expect("hydrolysis table cell accessibility identity overflow");
                    let cell_key = diagonal
                        .checked_add(1)
                        .and_then(|next| diagonal.checked_mul(next))
                        .and_then(|product| product.checked_div(2))
                        .and_then(|pair| pair.checked_add(row_key))
                        .and_then(|pair| pair.checked_add(1))
                        .expect("hydrolysis table cell accessibility identity overflow");
                    let cell_node_id = match ctx {
                        Some(ctx) => renderer.register_accessibility_child_node_with_key(
                            cell_key,
                            cell_node,
                            transformed_rect(ctx.hit_transform, cell_rect),
                            env,
                            None,
                        ),
                        None => renderer.register_accessibility_child_node_with_key_semantic(
                            cell_key, cell_node, env, None,
                        ),
                    };
                    if let Some(cell_node_id) = cell_node_id {
                        table_node.push_child(cell_node_id);
                    }
                }
            }
            x_offset += width;
        }

        let _ = renderer.register_accessibility_leaf(
            ctx,
            table_node,
            env,
            Some(AccessibilityActionTarget::Scroll {
                handle,
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
pub fn measure_table_node(
    table: &TableConfig,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    ViewDimensions::new(measure_table_intrinsic(table, state, env, theme))
}

/// Renders a retained table leaf every flush.
pub fn render_table_node(
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
        let theme = ctx.theme();
        table_accessibility(
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
    render_table_parts(ctx, state, env);
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_table_parts(
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
    let theme = ctx.theme();
    let layout_metrics = theme.table_metrics();
    {
        let state_ref = state.borrow();
        let mut slot = state_ref.slot.borrow_mut();
        refresh_table_slot_baseline(&columns, &mut slot, ctx.state_mut(), env, &theme);
    }
    let initial_table_metrics = {
        let state_ref = state.borrow();
        table_metrics_from_slot(&state_ref.slot.borrow(), layout_metrics)
    };
    let handle = state.borrow().bind_scroll(
        viewport.width(),
        viewport.height(),
        initial_table_metrics.table_width.max(viewport.width()),
        initial_table_metrics.table_height.max(viewport.height()),
    );
    let mut scroll_metrics = handle.metrics();
    let row_window = {
        let state_ref = state.borrow();
        let slot = state_ref.slot.borrow();
        resolve_table_visible_rows(
            scroll_metrics.offset_y,
            viewport.height(),
            slot.max_rows,
            layout_metrics,
        )
    };
    let mut column_window = {
        let state_ref = state.borrow();
        let slot = state_ref.slot.borrow();
        resolve_visible_column_window(
            &slot.column_widths,
            scroll_metrics.offset_x,
            scroll_metrics.offset_x + viewport.width(),
        )
    };
    {
        let state_ref = state.borrow();
        let mut slot = state_ref.slot.borrow_mut();
        update_table_slot_visible_cell_widths(
            &columns,
            &mut slot,
            row_window,
            column_window,
            ctx.state_mut(),
            env,
            &theme,
        );
    }
    let table_metrics = {
        let state_ref = state.borrow();
        table_metrics_from_slot(&state_ref.slot.borrow(), layout_metrics)
    };
    let handle = state.borrow().bind_scroll(
        viewport.width(),
        viewport.height(),
        table_metrics.table_width.max(viewport.width()),
        table_metrics.table_height.max(viewport.height()),
    );
    scroll_metrics = handle.metrics();
    {
        let state_ref = state.borrow();
        let slot = state_ref.slot.borrow();
        column_window = resolve_visible_column_window(
            &slot.column_widths,
            scroll_metrics.offset_x,
            scroll_metrics.offset_x + viewport.width(),
        );
    }

    ctx.push_layer_rect(1.0, viewport);
    // Register before the cells flush: scroll-target dispatch walks the
    // frame's targets newest-first, so a scroll region inside a cell wins the
    // delta until it hits its own edge, where it falls through to the table.
    let hit_transform = ctx.hit_transform;
    crate::widgets::scroll::register_scroll_wheel_target(
        ctx.renderer_mut(),
        hit_transform,
        viewport,
        &handle,
    );

    let origin_x = viewport.x0 - scroll_metrics.offset_x;
    let origin_y = viewport.y0 - scroll_metrics.offset_y;
    {
        let table_rect = kurbo::Rect::new(
            origin_x,
            origin_y,
            origin_x + table_metrics.table_width,
            origin_y + table_metrics.table_height,
        );
        let header_rect = kurbo::Rect::new(
            origin_x,
            origin_y,
            origin_x + table_metrics.table_width,
            origin_y + layout_metrics.header_height,
        );
        let theme = ctx.theme();
        ctx.draw_context(|draw| {
            theme.draw_table_background(&mut *draw, table_rect);
            theme.draw_table_header_background(&mut *draw, header_rect);
        });
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
        flush_cell_subview(
            ctx,
            state,
            env,
            TableCellKey::Header(column_index),
            header_view,
            header_rect,
        );

        let rows = column.rows().snapshot();
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
            let theme = ctx.theme();
            ctx.draw_context(|draw| {
                theme.draw_table_cell_border(&mut *draw, cell_rect);
            });
        }

        let separator_from = kurbo::Point::new(origin_x + x_offset + width, origin_y);
        let separator_to = kurbo::Point::new(
            origin_x + x_offset + width,
            origin_y + table_metrics.table_height,
        );
        let theme = ctx.theme();
        ctx.draw_context(|draw| {
            theme.draw_table_column_separator(&mut *draw, separator_from, separator_to);
            x_offset += width;
        });
    }
    // Evict content sub-views for cells no longer in the visible window.
    state.borrow().item_cache.borrow_mut().end_frame();

    ctx.pop_layer();

    draw_scroll_indicators(ctx, env, viewport, scroll_metrics, ScrollAxis::All, &handle);
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
    rect: kurbo::Rect,
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
        subview.flush_in_rect(
            ctx.renderer_mut(),
            render_ctx,
            env,
            bounded_proposal(rect),
            rect,
        );
    }
    #[cfg(feature = "accessibility")]
    ctx.renderer_mut().pop_accessibility_suppression();
}

/// Emits a retained table's accessibility tree for the semantic walk — the
/// same nodes `table_accessibility` registers, with no bounds and every cell
/// present.
#[cfg(feature = "accessibility")]
pub fn emit_table_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<TableRenderState>>,
    env: &Environment,
) {
    table_accessibility(renderer, None, None, state, env);
}
