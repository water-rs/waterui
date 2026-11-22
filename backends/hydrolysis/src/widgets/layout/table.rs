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
use crate::renderer::{
    HydroState, MeasuredTableMetrics, RenderContext, RetainedSubview, VisibleCells,
    VisibleSubviewCache, WidgetRenderContext, table_data_cell_rect, table_header_cell_rect,
};
use crate::scroll::ScrollHandle;
use crate::theme::TableLook;
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::Signal;
use waterui::component::table::{TableColumn, TableConfig};
use waterui_core::Environment;
#[cfg(feature = "accessibility")]
use waterui_core::contract_violation;
use waterui_core::id::Identifiable;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::views::ViewSnapshot;
use waterui_layout::scroll::Axis as ScrollAxis;

use crate::widgets::{draw_scroll_indicators, inset_rect};

/// The cache key for a table's retained data-cell sub-views: the cell's
/// position, the same index model the table uses for its column-width and
/// row-count slot. Column headers are retained per column identity in
/// [`TableRenderState::headers`] instead.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct TableCellKey {
    column: usize,
    row: usize,
}

/// Retained state for a table `Widget` node: the consumed config plus a per-widget
/// cache of the visible header/data cells' content sub-views, keyed by
/// [`TableCellKey`]. Only the cells in the current visible row/column window are
/// built and retained (evicted once they scroll out), so the table stays
/// virtualized — cost is bounded by visible cells.
pub struct TableRenderState<E: crate::text::TextEngine> {
    pub(crate) config: TableConfig,
    /// Column metrics belong to this semantic table node.
    slot: RefCell<LazyTableSlot>,
    /// Scroll state belongs to this semantic table node.
    scroll: RefCell<Option<ScrollHandle>>,
    /// Content sub-views for the cells currently in view, keyed by [`TableCellKey`]
    /// so a steady scroll reuses each visible cell's node (keeping its reactive
    /// content live) and only builds cells entering the window.
    item_cache: RefCell<VisibleSubviewCache<TableCellKey, E>>,
    /// Every column's header, realized once per column identity in column
    /// order: the header widths every column measures against, the header
    /// cells the flush places and the header labels the table emits.
    headers: RefCell<Vec<(usize, RetainedSubview<E>)>>,
    /// §7.1's scroll-surface bookkeeping: the extension and clearance
    /// bounds layout computed once (the scroll handle is rebound with
    /// them inside [`Self::bind_scroll`]), plus the focused-field state
    /// the flush drives.
    pub(crate) surface: std::rc::Rc<crate::renderer::ScrollSurfaceArea>,
    /// The look the theme gave the table, requested once: nothing structural
    /// about a table changes it.
    look: RefCell<Option<TableLook>>,
}

impl<E: crate::text::TextEngine> TableRenderState<E> {
    pub(crate) fn from_config(config: TableConfig) -> Self {
        Self {
            config,
            slot: RefCell::new(LazyTableSlot::default()),
            scroll: RefCell::new(None),
            item_cache: RefCell::new(VisibleSubviewCache::new()),
            headers: RefCell::new(Vec::new()),
            surface: std::rc::Rc::default(),
            look: RefCell::new(None),
        }
    }

    /// Keeps [`Self::headers`] realized for `columns`: a column keeps its
    /// header while its identity stays, a new column's header is built.
    /// Runs at build, before each measure the table is prepared for, and at
    /// the start of each render and semantic emit.
    pub(crate) fn refresh_headers(
        &self,
        columns: &[TableColumn],
        env: &Environment,
        renderer: &mut crate::renderer::SemanticCore<E>,
    ) {
        let mut headers = self.headers.borrow_mut();
        if headers
            .iter()
            .map(|(id, _)| *id)
            .eq(columns.iter().map(Identifiable::id))
        {
            return;
        }
        let mut previous = core::mem::take(&mut *headers);
        for column in columns {
            let id = column.id();
            let header = previous
                .iter()
                .position(|(kept, _)| *kept == id)
                .map_or_else(
                    || {
                        let mut header = RetainedSubview::new(column.label());
                        header.ensure_built(renderer, env);
                        header
                    },
                    |index| previous.swap_remove(index).1,
                );
            headers.push((id, header));
        }
    }

    /// [`Self::refresh_headers`] as the widget's layout-time prepare.
    pub(crate) fn prepare_headers(
        &self,
        renderer: &mut crate::renderer::HydrolysisRenderer<E>,
        env: &Environment,
    ) {
        let columns = renderer.read_signal(&self.config.columns);
        self.refresh_headers(&columns, env, renderer);
    }

    /// Each column's width floor from its realized header: the header's
    /// width plus the cell padding, never under the minimum column width.
    /// Columns that arrived since the headers were last refreshed answer the
    /// minimum until the next render realizes them.
    fn header_widths(
        &self,
        column_count: usize,
        state: &mut HydroState<E>,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
        metrics: crate::theme::TableMetrics,
    ) -> Vec<f64> {
        let headers = self.headers.borrow();
        (0..column_count)
            .map(|index| {
                headers
                    .get(index)
                    .map_or(metrics.min_column_width, |(_, header)| {
                        let size = header.measure_built(state, env, theme);
                        (f64::from(size.width) + metrics.cell_horizontal_padding)
                            .max(metrics.min_column_width)
                    })
            })
            .collect()
    }

    /// Rebases the slot on the columns: their count, their header widths and
    /// the longest column's row count.
    fn refresh_slot_baseline(
        &self,
        columns: &[TableColumn],
        state: &mut HydroState<E>,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
        metrics: crate::theme::TableMetrics,
    ) {
        let widths = self.header_widths(columns.len(), state, env, theme, metrics);
        let mut slot = self.slot.borrow_mut();
        slot.prepare_columns(columns.len(), metrics);
        slot.max_rows = 0;
        for (index, (column, width)) in columns.iter().zip(widths).enumerate() {
            if slot.column_widths[index] < width {
                slot.column_widths[index] = width;
            }
            slot.max_rows = slot.max_rows.max(column.rows().len().snapshot());
        }
    }

    /// Realizes the cell at `key` of `rows` if the cache does not hold it,
    /// marking it visible this frame. `false` when the column has no row
    /// there.
    fn realize_cell(
        &self,
        key: TableCellKey,
        rows: &impl ViewSnapshot<View = waterui_text::Text>,
        env: &Environment,
        renderer: &mut crate::renderer::SemanticCore<E>,
    ) -> bool {
        let mut cache = self.item_cache.borrow_mut();
        if cache.touch(&key) {
            return true;
        }
        let Some(cell) = rows.get_view(key.row) else {
            return false;
        };
        cache
            .entry(key, || RetainedSubview::new(cell))
            .ensure_built(renderer, env);
        true
    }

    /// Widens the slot's columns to the visible cells: each cell is realized
    /// and measured from its node.
    fn widen_to_visible_cells(
        &self,
        columns: &[TableColumn],
        visible: VisibleCells,
        renderer: &mut crate::renderer::SemanticCore<E>,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
        metrics: crate::theme::TableMetrics,
    ) {
        let VisibleCells {
            rows: row_window,
            columns: col_window,
        } = visible;
        for (column_index, column) in columns
            .iter()
            .enumerate()
            .take(col_window.end)
            .skip(col_window.start)
        {
            let rows = column.rows().snapshot();
            for row_index in row_window.start..row_window.end {
                let key = TableCellKey {
                    column: column_index,
                    row: row_index,
                };
                if !self.realize_cell(key, &rows, env, renderer) {
                    continue;
                }
                let size = self
                    .item_cache
                    .borrow()
                    .get(&key)
                    .expect("hydrolysis table: a realized cell is cached")
                    .measure_built(renderer.state_mut(), env, theme);
                let width = (f64::from(size.width) + metrics.cell_horizontal_padding)
                    .max(metrics.min_column_width);
                let mut slot = self.slot.borrow_mut();
                if slot.column_widths[column_index] < width {
                    slot.column_widths[column_index] = width;
                }
            }
        }
    }

    /// The table's look: the one already held, otherwise requested from
    /// `theme`.
    fn look(&self, theme: &Rc<dyn crate::theme::WidgetTheme>, env: &Environment) -> TableLook {
        self.look
            .borrow_mut()
            .get_or_insert_with(|| theme.table(env))
            .clone()
    }

    /// §7.1's scroll-surface rule: [`ScrollSurfaceArea::extended`] grows the
    /// viewport and content by the extension layout computed, so the
    /// scrollable range keeps the resting edges on the avoided boundary
    /// while scrolling paints through the band.
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
        let (viewport, content) = self.surface.extended(
            kurbo::Size::new(viewport_width, viewport_height),
            kurbo::Size::new(content_width, content_height),
        );
        let mut scroll = self.scroll.borrow_mut();
        if let Some(handle) = scroll.as_mut() {
            handle.rebind(
                ScrollAxis::All,
                viewport.width,
                viewport.height,
                content.width,
                content.height,
                (0.0, 0.0),
                None,
            )
        } else {
            let handle = ScrollHandle::new(
                ScrollAxis::All,
                viewport.width,
                viewport.height,
                content.width,
                content.height,
                None,
            );
            *scroll = Some(handle.clone());
            handle
        }
    }
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
pub fn table_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn crate::theme::WidgetTheme>>,
    state: &Rc<RefCell<TableRenderState<E>>>,
    env: &Environment,
) {
    let columns_signal = state.borrow().config.columns.clone();
    let columns = renderer.read_signal(&columns_signal);
    if columns.is_empty() {
        return;
    }
    state.borrow().refresh_headers(&columns, env, renderer);
    let layout_metrics = theme.map(|theme| state.borrow().look(theme, env).metrics());
    if let Some((theme, metrics)) = theme.zip(layout_metrics) {
        state
            .borrow()
            .refresh_slot_baseline(&columns, renderer.state_mut(), env, theme, metrics);
    } else {
        {
            let state_ref = state.borrow();
            let mut slot = state_ref.slot.borrow_mut();
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
    let viewport = ctx.map_or(kurbo::Rect::ZERO, |ctx| ctx.bounds());
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
        let row_span = state
            .borrow()
            .surface
            .visible_span(&scroll_metrics, ScrollAxis::Vertical);
        let column_span = state
            .borrow()
            .surface
            .visible_span(&scroll_metrics, ScrollAxis::Horizontal);
        let row_window = {
            let state_ref = state.borrow();
            let slot = state_ref.slot.borrow();
            match layout_metrics {
                Some(layout_metrics) => resolve_table_visible_rows(
                    row_span.start,
                    row_span.end - row_span.start,
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
                    column_span.start,
                    column_span.end,
                )
            } else {
                VisibleColumnWindow {
                    start: 0,
                    end: columns.len(),
                    leading_offset: 0.0,
                }
            }
        };
        if layout_metrics.is_some() && theme.is_none() {
            contract_violation!(
                "hydrolysis rendered table accessibility requires a theme; the visible cell widths are not refreshed"
            );
        }
        if let Some((metrics, theme)) = layout_metrics.zip(theme) {
            state.borrow().widen_to_visible_cells(
                &columns,
                VisibleCells {
                    rows: row_window,
                    columns: column_window,
                },
                renderer,
                env,
                theme,
                metrics,
            );
            let state_ref = state.borrow();
            let slot = state_ref.slot.borrow();
            column_window = resolve_visible_column_window(
                &slot.column_widths,
                column_span.start,
                column_span.end,
            );
        }
        let mut table_node = AccessibilityNode::new(
            crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Table,
            ),
        );
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
        // The semantic walk emits every cell: it keeps realized exactly the
        // cells it emitted. A rendered emit leaves the frame to the flush.
        if !rendered {
            state.borrow().item_cache.borrow_mut().begin_frame();
        }
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
            let mut header_node = AccessibilityNode::new(
                crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                    env,
                    AccessibilityNodeRole::ColumnHeader,
                ),
            );
            let default_label = state
                .borrow()
                .headers
                .borrow()
                .get(column_index)
                .and_then(|(_, header)| header.accessibility_label(renderer));
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
                Some(_ctx) => renderer.register_accessibility_child_node_with_key(
                    header_key,
                    header_node,
                    header_cell,
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
                let key = TableCellKey {
                    column: column_index,
                    row: row_index,
                };
                if state.borrow().realize_cell(key, &rows, env, renderer) {
                    let mut cell_node = AccessibilityNode::new(
                        crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                            env,
                            AccessibilityNodeRole::Cell,
                        ),
                    );
                    let default_label = state
                        .borrow()
                        .item_cache
                        .borrow()
                        .get(&key)
                        .and_then(|cell| cell.accessibility_label(renderer));
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
                        Some(_ctx) => renderer.register_accessibility_child_node_with_key(
                            cell_key, cell_node, cell_rect, env, None,
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
        if !rendered {
            state.borrow().item_cache.borrow_mut().end_frame();
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

/// Measures a table leaf: its columns' widths from their realized headers
/// over the header and row bands (intrinsic-sized; proposal-independent).
pub fn measure_table_node<E: crate::text::TextEngine>(
    table: &TableRenderState<E>,
    _proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    let columns = state.measure_signal(&table.config.columns);
    if columns.is_empty() {
        return ViewDimensions::new(LayoutSize::zero());
    }
    let metrics = table.look(theme, env).metrics();
    let widths = table.header_widths(columns.len(), state, env, theme, metrics);
    let max_rows = columns
        .iter()
        .map(|column| state.measure_signal(&column.rows().len()))
        .max()
        .unwrap_or(0);
    let height = metrics.row_height.mul_add(
        crate::num_cast::usize_as_f64(max_rows),
        metrics.header_height,
    );
    ViewDimensions::new(LayoutSize::new(
        crate::num_cast::f64_as_f32(widths.iter().sum()),
        crate::num_cast::f64_as_f32(height),
    ))
}

/// Renders a retained table leaf every flush.
pub fn render_table_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<TableRenderState<E>>>,
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
pub fn render_table_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<TableRenderState<E>>>,
    env: &Environment,
) {
    let columns_signal = state.borrow().config.columns.clone();
    let columns: Vec<TableColumn> = ctx.renderer_mut().read_signal(&columns_signal);
    if columns.is_empty() {
        return;
    }
    let viewport = ctx.bounds;
    // §7.1's scroll surface: the clip, the wheel target and the visible
    // windows all run on the frame grown by the extension layout computed —
    // the surface paints through the bands it touched. The cell origin math
    // stays anchored on the laid-out frame.
    let extension = state.borrow().surface.extension();
    let surface_viewport = crate::renderer::grow_rect(viewport, extension);
    let theme = ctx.theme();
    let look = state.borrow().look(&theme, env);
    let layout_metrics = look.metrics();
    state
        .borrow()
        .refresh_headers(&columns, env, ctx.renderer_mut());
    state
        .borrow()
        .refresh_slot_baseline(&columns, ctx.state_mut(), env, &theme, layout_metrics);
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
    // The keyboard-moving clearance runs before the cells paint: while the
    // host's keyboard animation is in flight the offset follows it frame by
    // frame, so this flush paints the field already clear.
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    state
        .borrow()
        .surface
        .begin_flush(ctx.renderer_mut(), &handle);
    let mut scroll_metrics = handle.metrics();
    let row_span = state
        .borrow()
        .surface
        .visible_span(&scroll_metrics, ScrollAxis::Vertical);
    let column_span = state
        .borrow()
        .surface
        .visible_span(&scroll_metrics, ScrollAxis::Horizontal);
    let row_window = {
        let state_ref = state.borrow();
        let slot = state_ref.slot.borrow();
        resolve_table_visible_rows(
            row_span.start,
            row_span.end - row_span.start,
            slot.max_rows,
            layout_metrics,
        )
    };
    let mut column_window = {
        let state_ref = state.borrow();
        let slot = state_ref.slot.borrow();
        resolve_visible_column_window(&slot.column_widths, column_span.start, column_span.end)
    };
    state.borrow().widen_to_visible_cells(
        &columns,
        VisibleCells {
            rows: row_window,
            columns: column_window,
        },
        ctx.renderer_mut(),
        env,
        &theme,
        layout_metrics,
    );
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
        column_window =
            resolve_visible_column_window(&slot.column_widths, column_span.start, column_span.end);
    }

    // Register before the cells flush: scroll-target dispatch walks the
    // frame's targets newest-first, so a scroll region inside a cell wins the
    // delta until it hits its own edge, where it falls through to the table.
    crate::widgets::scroll::register_scroll_wheel_target(
        ctx.renderer_mut(),
        surface_viewport,
        &handle,
    );

    // The cells scroll as the widget's inner layer `ScrollOffset`, exactly as
    // a `scroll` node's content does (decision 1): they paint and place at
    // their resting positions in content space and the inner layer shifts
    // them, so a scroll frame writes the one scroll-offset property instead of
    // re-placing every visible cell. The viewport placement carries the same
    // offset so the cells' hit regions and emitted bounds follow it without a
    // re-record.
    let offset = kurbo::Vec2::new(scroll_metrics.offset_x, scroll_metrics.offset_y);
    {
        let renderer = ctx.renderer_mut();
        renderer.push_placement_scope(
            crate::renderer::mount::ScopeDelta::RECORD_SPACE,
            Some(surface_viewport),
        );
        let viewport_placement = renderer.current_placement();
        viewport_placement.set_content_offset(-offset);
        let stretch = renderer.scroll_stretch(&handle);
        renderer.program().begin_inner(
            surface_viewport,
            offset,
            stretch,
            std::rc::Rc::clone(&viewport_placement),
        );
        // The cells record at `viewport.origin + content`: the content's local
        // transform is the frame's origin — the relation `ScrollNode`
        // publishes through `content_ctx.local` — so the lazy window maps the
        // content-space spans through it before the scroll shift.
        let world = renderer.record_world(kurbo::Affine::translate(kurbo::Vec2::new(
            viewport.x0,
            viewport.y0,
        )));
        // The spans resolve against the post-rebind metrics — the surface can
        // have shifted the range when the measured extent arrived.
        let lazy_row_span = state
            .borrow()
            .surface
            .visible_span(&scroll_metrics, ScrollAxis::Vertical);
        let lazy_column_span = state
            .borrow()
            .surface
            .visible_span(&scroll_metrics, ScrollAxis::Horizontal);
        renderer.push_lazy_viewport(crate::renderer::LazyViewport {
            bounds: kurbo::Rect::new(
                lazy_column_span.start,
                lazy_row_span.start,
                lazy_column_span.end,
                lazy_row_span.end,
            ),
            transform: world * kurbo::Affine::translate(-offset),
        });
    }

    // The cell origin is the table's content corner in content space: column
    // and row offsets accumulate from it unshifted.
    let origin_x = viewport.x0;
    let origin_y = viewport.y0;
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
        ctx.draw_context(|draw| {
            look.record_background(&mut *draw, table_rect);
            look.record_header_background(&mut *draw, header_rect);
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
        let header_rect = inset_rect(
            header_cell,
            cell_horizontal_inset,
            layout_metrics.cell_vertical_inset,
        );
        {
            let state_ref = state.borrow();
            let mut headers = state_ref.headers.borrow_mut();
            let (_, header) = headers
                .get_mut(column_index)
                .expect("hydrolysis table: every column's header is realized before the flush");
            flush_cell_subview(ctx, env, header_rect, header);
        }

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
                let inset = inset_rect(
                    cell_rect,
                    cell_horizontal_inset,
                    layout_metrics.cell_vertical_inset,
                );
                let key = TableCellKey {
                    column: column_index,
                    row: row_index,
                };
                let state_ref = state.borrow();
                let mut cache = state_ref.item_cache.borrow_mut();
                flush_cell_subview(
                    ctx,
                    env,
                    inset,
                    cache.entry(key, move || RetainedSubview::new(cell)),
                );
            }
            ctx.draw_context(|draw| {
                look.record_cell_border(&mut *draw, cell_rect);
            });
        }

        // The separator draws over the zero-width rect between its two ends.
        let separator = kurbo::Rect::new(
            origin_x + x_offset + width,
            origin_y,
            origin_x + x_offset + width,
            origin_y + table_metrics.table_height,
        );
        ctx.draw_context(|draw| {
            look.record_column_separator(&mut *draw, separator);
            x_offset += width;
        });
    }
    // Evict content sub-views for cells no longer in the visible window.
    state.borrow().item_cache.borrow_mut().end_frame();

    {
        let renderer = ctx.renderer_mut();
        renderer.pop_lazy_viewport("hydrolysis table");
        renderer.program().end_inner();
        renderer.pop_placement_scope();
    }

    // The focused-field clearance runs after recording, over the retained
    // input targets (§7.1).
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    state
        .borrow()
        .surface
        .register_clearance(ctx.renderer_mut(), &handle);

    draw_scroll_indicators(
        ctx,
        env,
        viewport,
        scroll_metrics,
        ScrollAxis::All,
        &handle,
        state.borrow().surface.extension(),
    );
}

/// Places one table cell's retained content in `rect`. The cell's own
/// accessibility is suppressed — `table_accessibility` emits the table's
/// cells.
fn flush_cell_subview<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    env: &Environment,
    rect: kurbo::Rect,
    subview: &mut RetainedSubview<E>,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    #[cfg(feature = "accessibility")]
    ctx.renderer_mut().push_accessibility_suppression();
    let render_ctx = ctx.render_context();
    // A table cell is scroll content: it lays out with no §7.1 context (its
    // surface owns the edges).
    subview.place(
        ctx.renderer_mut(),
        render_ctx,
        env,
        bounded_proposal(rect),
        rect,
        None,
    );
    #[cfg(feature = "accessibility")]
    ctx.renderer_mut().pop_accessibility_suppression();
}

/// Emits a retained table's accessibility tree for the semantic walk — the
/// same nodes `table_accessibility` registers, with no bounds and every cell
/// present.
#[cfg(feature = "accessibility")]
#[cfg(hydrolysis_cherenkov)]
pub fn emit_table_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    state: &Rc<RefCell<TableRenderState<E>>>,
    env: &Environment,
) {
    table_accessibility(renderer, None, None, state, env);
}
