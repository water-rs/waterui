use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashSet};
use std::rc::Rc;

#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
    SortDirection as AccessKitSortDirection,
};
use cherenkov_record::Draw as _;
use nami::SignalExt as _;
use nami::watcher::BoxWatcherGuard;
use waterui::component::table::{SortDirection, TableConfig, TableRow, TableSelection, TableSort};
use waterui::theme::color;
use waterui_core::Environment;
#[cfg(feature = "accessibility")]
use waterui_core::contract_violation;
use waterui_core::id::Id as RawId;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::resolve::Resolvable as _;
use waterui_core::views::AnyViewsSnapshot;
use waterui_layout::scroll::Axis as ScrollAxis;

use crate::num_cast::{f64_as_f32, i32_as_u32, usize_as_f64};
use crate::platform::Modifiers;
#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::lazy::{
    LazyTableSlot, resolve_table_visible_rows, resolve_visible_column_window,
    table_metrics_from_slot,
};
#[cfg(feature = "accessibility")]
use crate::renderer::lazy::{VisibleColumnWindow, VisibleIndexWindow};
use crate::renderer::{
    Dirty, HeldChild, HydroState, HydrolysisRenderer, InteractionKey, LazyViewport,
    MeasuredTableMetrics, NodeCore, RenderContext, RetainedSubview, ScrollSurfaceArea,
    SemanticCore, VisibleCells, VisibleSubviewCache, WidgetRenderContext, bounded_proposal,
    collect_replaced_ids, grow_rect, table_data_cell_rect, table_header_cell_rect,
};
use crate::scroll::ScrollHandle;
use crate::theme::{TableLook, TableMetrics, WidgetTheme};
use crate::widgets::{draw_scroll_indicators, inset_rect};

/// Extra width reserved inside sortable column headers for the sort chevron.
const SORT_INDICATOR_SPACE: f64 = 16.0;
/// Half-width of the drawn sort chevron.
const SORT_CHEVRON_HALF_W: f64 = 3.5;
/// Half-height of the drawn sort chevron.
const SORT_CHEVRON_HALF_H: f64 = 2.0;

/// Cache key for a table's retained data-cell sub-views, keyed by stable row
/// identity and column index so sorting/reordering rows reuses each row's
/// retained cell views.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct TableCellKey {
    row: RawId,
    column: usize,
}

const fn header_interaction_slot(column_index: usize) -> usize {
    column_index
        .checked_mul(2)
        .expect("hydrolysis table header interaction slot overflow")
}

fn row_interaction_slot(row_id: RawId) -> usize {
    (i32_as_u32(i32::from(row_id)) as usize)
        .checked_mul(2)
        .and_then(|slot| slot.checked_add(1))
        .expect("hydrolysis table row interaction slot overflow")
}

/// Shared row-selection controller for pointer and accessibility interactions.
struct TableRowSelection<E: crate::text::TextEngine> {
    selection: TableSelection<RawId>,
    snapshot: Rc<RefCell<AnyViewsSnapshot<TableRow<HeldChild<E>>>>>,
    anchor: Cell<Option<RawId>>,
}

impl<E: crate::text::TextEngine> TableRowSelection<E> {
    fn new(
        config: &TableConfig<HeldChild<E>>,
        snapshot: Rc<RefCell<AnyViewsSnapshot<TableRow<HeldChild<E>>>>>,
    ) -> Option<Rc<Self>> {
        match config.selection() {
            TableSelection::None => None,
            selection => Some(Rc::new(Self {
                selection: selection.clone(),
                snapshot,
                anchor: Cell::new(None),
            })),
        }
    }

    fn selected_ids(&self, renderer: &mut SemanticCore<E>) -> BTreeSet<RawId> {
        match &self.selection {
            TableSelection::None => BTreeSet::new(),
            TableSelection::Single(binding) => renderer.read_signal(binding).into_iter().collect(),
            TableSelection::Multiple(binding) => renderer.read_signal(binding),
        }
    }

    fn index_of(&self, id: RawId) -> Option<usize> {
        let snapshot = self.snapshot.borrow().clone();
        snapshot
            .range()
            .find(|&index| snapshot.get_id(index) == Some(id))
    }

    fn write(&self, index: usize, id: RawId, modifiers: Modifiers) {
        match &self.selection {
            TableSelection::None => {}
            TableSelection::Single(selection) => {
                selection.set(Some(id));
            }
            TableSelection::Multiple(selection) if modifiers.shift => {
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
                let snapshot = self.snapshot.borrow().clone();
                selection.set(
                    (start..=end)
                        .filter_map(|row| snapshot.get_id(row))
                        .collect(),
                );
            }
            TableSelection::Multiple(selection) => {
                if modifiers.control || modifiers.super_key {
                    selection.with_mut(|selected| {
                        if !selected.insert(id) {
                            selected.remove(&id);
                        }
                    });
                } else {
                    selection.set(BTreeSet::from([id]));
                }
                self.anchor.set(Some(id));
            }
        }
    }
}

/// Retained state for a row-major table `Widget` node: its config, row
/// snapshot, retained header sub-views, and virtualized identity-keyed cell
/// cache.
pub struct TableRenderState<E: crate::text::TextEngine> {
    pub(crate) config: TableConfig<HeldChild<E>>,
    slot: RefCell<LazyTableSlot>,
    scroll: RefCell<Option<ScrollHandle>>,
    item_cache: RefCell<VisibleSubviewCache<TableCellKey, E>>,
    headers: RefCell<Vec<RetainedSubview<E>>>,
    rows_snapshot: Rc<RefCell<AnyViewsSnapshot<TableRow<HeldChild<E>>>>>,
    replaced_row_ids: Rc<RefCell<HashSet<RawId>>>,
    row_selection: Option<Rc<TableRowSelection<E>>>,
    pub(crate) surface: Rc<ScrollSurfaceArea>,
    look: RefCell<Option<TableLook>>,
    _rows_guard: BoxWatcherGuard,
}

impl<E: crate::text::TextEngine> TableRenderState<E> {
    pub(crate) fn from_config(config: TableConfig<HeldChild<E>>, core: &NodeCore<E>) -> Self {
        let rows_snapshot = Rc::new(RefCell::new(config.rows().snapshot()));
        let snapshot_for_watch = Rc::clone(&rows_snapshot);
        let replaced_row_ids = Rc::new(RefCell::new(HashSet::new()));
        let replaced_for_watch = Rc::clone(&replaced_row_ids);
        let cell = Rc::downgrade(&core.cell);
        let rows_guard = config.rows().watch(.., move |ctx, change| {
            let event_snapshot = ctx.into_value();
            collect_replaced_ids(
                &event_snapshot,
                change,
                &mut replaced_for_watch.borrow_mut(),
            );
            *snapshot_for_watch.borrow_mut() = event_snapshot;
            if let Some(cell) = cell.upgrade() {
                cell.mark(Dirty::STRUCTURE);
            }
        });
        let row_selection = TableRowSelection::new(&config, Rc::clone(&rows_snapshot));
        Self {
            config,
            slot: RefCell::new(LazyTableSlot::default()),
            scroll: RefCell::new(None),
            item_cache: RefCell::new(VisibleSubviewCache::new()),
            headers: RefCell::new(Vec::new()),
            rows_snapshot,
            replaced_row_ids,
            row_selection,
            surface: Rc::default(),
            look: RefCell::new(None),
            _rows_guard: rows_guard,
        }
    }

    /// Evicts cached cell sub-views for any rows reported as replaced since the
    /// last pass so updated item content rebuilds cleanly.
    fn consume_replaced_rows(&self) {
        let replaced = core::mem::take(&mut *self.replaced_row_ids.borrow_mut());
        if replaced.is_empty() {
            return;
        }
        let col_count = self.config.columns().len();
        let mut cache = self.item_cache.borrow_mut();
        for row_id in replaced {
            for column in 0..col_count {
                let _ = cache.take(&TableCellKey {
                    row: row_id,
                    column,
                });
            }
        }
    }

    /// Ensures every column header sub-view is realized once in column order.
    pub(crate) fn refresh_headers(&self, env: &Environment, renderer: &mut SemanticCore<E>) {
        let mut headers = self.headers.borrow_mut();
        if headers.len() == self.config.columns().len() {
            return;
        }
        headers.clear();
        for column in self.config.columns() {
            let mut header = RetainedSubview::new(column.label().clone());
            header.ensure_built(renderer, env);
            headers.push(header);
        }
    }

    /// Widget layout-time prepare hook.
    pub(crate) fn prepare_headers(&self, renderer: &mut HydrolysisRenderer<E>, env: &Environment) {
        self.refresh_headers(env, renderer);
    }

    fn header_widths(
        &self,
        state: &mut HydroState<E>,
        env: &Environment,
        theme: &Rc<dyn WidgetTheme>,
        metrics: TableMetrics,
    ) -> Vec<f64> {
        let headers = self.headers.borrow();
        self.config
            .columns()
            .iter()
            .enumerate()
            .map(|(index, column)| {
                let sort_extra = if self.config.sort_order().is_some() && column.is_sortable() {
                    SORT_INDICATOR_SPACE
                } else {
                    0.0
                };
                headers
                    .get(index)
                    .map_or(metrics.min_column_width, |header| {
                        let size = header.measure_built(state, env, theme);
                        (f64::from(size.width) + metrics.cell_horizontal_padding + sort_extra)
                            .max(metrics.min_column_width)
                    })
            })
            .collect()
    }

    fn refresh_slot_baseline(
        &self,
        row_count: usize,
        state: &mut HydroState<E>,
        env: &Environment,
        theme: &Rc<dyn WidgetTheme>,
        metrics: TableMetrics,
    ) {
        let widths = self.header_widths(state, env, theme, metrics);
        let mut slot = self.slot.borrow_mut();
        slot.prepare_columns(self.config.columns().len(), metrics);
        slot.max_rows = row_count;
        for (index, width) in widths.into_iter().enumerate() {
            if slot.column_widths[index] < width {
                slot.column_widths[index] = width;
            }
        }
    }

    /// Ensures every cell in `columns` for row `row_index` (identity
    /// `row_id`) is cached and built, invoking `snapshot.get_view(row_index)`
    /// at most once even when multiple columns are missing.
    fn realize_row_cells(
        &self,
        row_index: usize,
        row_id: RawId,
        columns: core::ops::Range<usize>,
        snapshot: &AnyViewsSnapshot<TableRow<HeldChild<E>>>,
        env: &Environment,
        renderer: &mut SemanticCore<E>,
    ) {
        let mut missing = false;
        {
            let mut cache = self.item_cache.borrow_mut();
            for column in columns.clone() {
                if !cache.touch(&TableCellKey {
                    row: row_id,
                    column,
                }) {
                    missing = true;
                }
            }
        }
        if !missing {
            return;
        }
        let Some(row) = snapshot.get_view(row_index) else {
            return;
        };
        let mut cache = self.item_cache.borrow_mut();
        for (column, cell) in row.into_cells().into_iter().enumerate() {
            if !columns.contains(&column) {
                continue;
            }
            let key = TableCellKey {
                row: row_id,
                column,
            };
            if !cache.touch(&key) {
                cache
                    .entry(key, || RetainedSubview::held(cell))
                    .ensure_built(renderer, env);
            }
        }
    }

    fn widen_to_visible_cells(
        &self,
        snapshot: &AnyViewsSnapshot<TableRow<HeldChild<E>>>,
        visible: VisibleCells,
        renderer: &mut SemanticCore<E>,
        env: &Environment,
        theme: &Rc<dyn WidgetTheme>,
        metrics: TableMetrics,
    ) {
        let VisibleCells {
            rows: row_window,
            columns: col_window,
        } = visible;
        for row_index in row_window.start..row_window.end {
            let Some(row_id) = snapshot.get_id(row_index) else {
                continue;
            };
            self.realize_row_cells(
                row_index,
                row_id,
                col_window.start..col_window.end,
                snapshot,
                env,
                renderer,
            );
            for column_index in col_window.start..col_window.end {
                let key = TableCellKey {
                    row: row_id,
                    column: column_index,
                };
                let size = self
                    .item_cache
                    .borrow()
                    .get(&key)
                    .map(|cell| cell.measure_built(renderer.state_mut(), env, theme));
                if let Some(size) = size {
                    let width = (f64::from(size.width) + metrics.cell_horizontal_padding)
                        .max(metrics.min_column_width);
                    let mut slot = self.slot.borrow_mut();
                    if slot.column_widths[column_index] < width {
                        slot.column_widths[column_index] = width;
                    }
                }
            }
        }
    }

    fn look(&self, theme: &Rc<dyn WidgetTheme>, env: &Environment) -> TableLook {
        self.look
            .borrow_mut()
            .get_or_insert_with(|| theme.table(env))
            .clone()
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

    fn active_sort(&self, renderer: &mut SemanticCore<E>) -> Option<TableSort<usize>> {
        self.config
            .sort_order()
            .and_then(|binding| renderer.read_signal(binding))
    }
}

/// Emits a table's accessibility tree (`Table` -> `ColumnHeader`s + `Row`s ->
/// `Cell`s) with selected state on rows and sort direction + click activation
/// on sortable column headers.
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous accessibility emission pass over headers, rows and cells"
)]
pub fn table_accessibility<E: crate::text::TextEngine>(
    renderer: &mut SemanticCore<E>,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn WidgetTheme>>,
    state: &Rc<RefCell<TableRenderState<E>>>,
    env: &Environment,
) {
    let col_count = state.borrow().config.columns().len();
    if col_count == 0 {
        return;
    }
    state.borrow().consume_replaced_rows();
    state.borrow().refresh_headers(env, renderer);
    let snapshot = state.borrow().rows_snapshot.borrow().clone();
    let row_count = snapshot.len();
    let active_sort = state.borrow().active_sort(renderer);
    let selected_ids = state
        .borrow()
        .row_selection
        .as_ref()
        .map(|sel| sel.selected_ids(renderer));

    let layout_metrics = theme.map(|theme| state.borrow().look(theme, env).metrics());
    if let Some((theme, metrics)) = theme.zip(layout_metrics) {
        state
            .borrow()
            .refresh_slot_baseline(row_count, renderer.state_mut(), env, theme, metrics);
    } else {
        state.borrow().slot.borrow_mut().max_rows = row_count;
    }

    let viewport = ctx.map_or(kurbo::Rect::ZERO, |ctx| ctx.bounds());
    let table_metrics = {
        let state_ref = state.borrow();
        match layout_metrics {
            Some(layout_metrics) => {
                table_metrics_from_slot(&state_ref.slot.borrow(), layout_metrics)
            }
            None => MeasuredTableMetrics {
                column_widths: vec![1.0; col_count],
                table_width: usize_as_f64(col_count),
                table_height: usize_as_f64(row_count),
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
        let is_rendered_pass = layout_metrics.is_some();
        let row_span = state
            .borrow()
            .surface
            .visible_span(&scroll_metrics, ScrollAxis::Vertical);
        let column_span = state
            .borrow()
            .surface
            .visible_span(&scroll_metrics, ScrollAxis::Horizontal);
        let row_window = layout_metrics.map_or(
            VisibleIndexWindow {
                start: 0,
                end: row_count,
                leading_offset: 0.0,
            },
            |layout_metrics| {
                resolve_table_visible_rows(
                    row_span.start,
                    row_span.end - row_span.start,
                    row_count,
                    layout_metrics,
                )
            },
        );
        let mut column_window = if is_rendered_pass {
            let state_ref = state.borrow();
            let slot = state_ref.slot.borrow();
            resolve_visible_column_window(&slot.column_widths, column_span.start, column_span.end)
        } else {
            VisibleColumnWindow {
                start: 0,
                end: col_count,
                leading_offset: 0.0,
            }
        };
        if layout_metrics.is_some() && theme.is_none() {
            contract_violation!(
                "hydrolysis rendered table accessibility requires a theme; the visible cell widths are not refreshed"
            );
        }
        if let Some((metrics, theme)) = layout_metrics.zip(theme) {
            state.borrow().widen_to_visible_cells(
                &snapshot,
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

        let mut table_node = AccessibilityNode::new(SemanticCore::<E>::resolve_accessibility_role(
            env,
            AccessibilityNodeRole::Table,
        ));
        if let Some(label) = renderer.resolve_accessibility_label(env, None) {
            table_node.set_label(label);
        }
        if let Some(value) = renderer.resolve_accessibility_value(env, None) {
            table_node.set_value(value);
        }
        table_node.set_row_count(row_count);
        table_node.set_column_count(col_count);
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
        if !is_rendered_pass {
            state.borrow().item_cache.borrow_mut().begin_frame();
        }

        // Emit ColumnHeader nodes.
        let mut x_offset = column_window.leading_offset;
        for (column_index, width) in table_metrics
            .column_widths
            .iter()
            .copied()
            .enumerate()
            .take(column_window.end)
            .skip(column_window.start)
        {
            let header_rect = layout_metrics.map_or(kurbo::Rect::ZERO, |m| {
                table_header_cell_rect(origin_x, origin_y, x_offset, width, m)
            });
            let sortable = state.borrow().config.column_is_sortable(column_index);
            let mut header_node =
                AccessibilityNode::new(SemanticCore::<E>::resolve_accessibility_role(
                    env,
                    AccessibilityNodeRole::ColumnHeader,
                ));
            header_node.set_column_index(column_index);
            let default_label = state
                .borrow()
                .headers
                .borrow()
                .get(column_index)
                .and_then(|header| header.accessibility_label(renderer));
            if let Some(label) = renderer.resolve_accessibility_label(env, default_label) {
                header_node.set_label(label);
            }
            header_node.add_action(AccessibilityAction::Focus);
            let action_target = if sortable {
                header_node.add_action(AccessibilityAction::Click);
                if let Some(sort) = active_sort
                    && sort.column == column_index
                {
                    header_node.set_sort_direction(match sort.direction {
                        SortDirection::Ascending => AccessKitSortDirection::Ascending,
                        SortDirection::Descending => AccessKitSortDirection::Descending,
                    });
                }
                let weak = Rc::downgrade(state);
                Some(AccessibilityActionTarget::Activate {
                    action: Rc::new(RefCell::new(
                        move |_: &mut SemanticCore<E>, _: &Environment| {
                            weak.upgrade().is_some_and(|state| {
                                state.borrow().config.toggle_sort(column_index);
                                true
                            })
                        },
                    )),
                })
            } else {
                None
            };

            let column_key = i64::try_from(column_index)
                .expect("hydrolysis table column index exceeds accessibility identity range");
            let header_key = column_key
                .checked_add(1)
                .and_then(i64::checked_neg)
                .expect("hydrolysis table header accessibility identity overflow");
            let header_node_id = match ctx {
                Some(_) => renderer.register_accessibility_child_node_with_key(
                    header_key,
                    header_node,
                    header_rect,
                    env,
                    action_target,
                ),
                None => renderer.register_accessibility_child_node_with_key_semantic(
                    header_key,
                    header_node,
                    env,
                    action_target,
                ),
            };
            if let Some(header_node_id) = header_node_id {
                table_node.push_child(header_node_id);
                if sortable {
                    renderer.register_accessibility_focus_link(
                        &InteractionKey::for_rc(state, header_interaction_slot(column_index)),
                        header_node_id,
                    );
                }
            }
            x_offset += width;
        }

        // Emit Row nodes and their Cell children.
        for row_index in row_window.start..row_window.end {
            let Some(row_id) = snapshot.get_id(row_index) else {
                continue;
            };
            state.borrow().realize_row_cells(
                row_index,
                row_id,
                column_window.start..column_window.end,
                &snapshot,
                env,
                renderer,
            );
            let row_rect = layout_metrics.map_or(kurbo::Rect::ZERO, |m| {
                let y0 = m
                    .row_height
                    .mul_add(usize_as_f64(row_index), origin_y + m.header_height);
                kurbo::Rect::new(
                    origin_x,
                    y0,
                    origin_x + table_metrics.table_width,
                    y0 + m.row_height,
                )
            });

            let mut row_node = AccessibilityNode::new(
                SemanticCore::<E>::resolve_accessibility_role(env, AccessibilityNodeRole::Row),
            );
            row_node.set_row_index(row_index);
            row_node.add_action(AccessibilityAction::Focus);
            let row_action = selected_ids.as_ref().and_then(|selected_set| {
                row_node.set_selected(selected_set.contains(&row_id));
                row_node.add_action(AccessibilityAction::Click);
                state.borrow().row_selection.as_ref().map(|selection| {
                    let selection = Rc::clone(selection);
                    AccessibilityActionTarget::Activate {
                        action: Rc::new(RefCell::new(
                            move |renderer: &mut SemanticCore<E>, _: &Environment| {
                                selection.write(row_index, row_id, renderer.modifiers());
                                true
                            },
                        )),
                    }
                })
            });

            let row_id_i64 = i64::from(i32::from(row_id));
            let mut cell_x = column_window.leading_offset;
            for (column_index, width) in table_metrics
                .column_widths
                .iter()
                .copied()
                .enumerate()
                .take(column_window.end)
                .skip(column_window.start)
            {
                let cell_rect = layout_metrics.map_or(kurbo::Rect::ZERO, |m| {
                    table_data_cell_rect(origin_x, origin_y, cell_x, width, row_index, m)
                });
                let key = TableCellKey {
                    row: row_id,
                    column: column_index,
                };
                if let Some(default_label) = state
                    .borrow()
                    .item_cache
                    .borrow()
                    .get(&key)
                    .map(|cell| cell.accessibility_label(renderer))
                {
                    let mut cell_node =
                        AccessibilityNode::new(SemanticCore::<E>::resolve_accessibility_role(
                            env,
                            AccessibilityNodeRole::Cell,
                        ));
                    cell_node.set_row_index(row_index);
                    cell_node.set_column_index(column_index);
                    if let Some(label) = renderer.resolve_accessibility_label(env, default_label) {
                        cell_node.set_label(label);
                    }
                    cell_node.add_action(AccessibilityAction::Focus);
                    // Distinct positive key per (row_id, column_index):
                    // shift row_id above the column index range.
                    let cell_key = (row_id_i64 << 16)
                        .wrapping_add(i64::try_from(column_index).unwrap_or(0))
                        .wrapping_add(1);
                    let cell_node_id = match ctx {
                        Some(_) => renderer.register_accessibility_child_node_with_key(
                            cell_key, cell_node, cell_rect, env, None,
                        ),
                        None => renderer.register_accessibility_child_node_with_key_semantic(
                            cell_key, cell_node, env, None,
                        ),
                    };
                    if let Some(cell_node_id) = cell_node_id {
                        row_node.push_child(cell_node_id);
                    }
                }
                cell_x += width;
            }

            // Distinct positive key for the row node (column slot = 0xffff).
            let row_key = (row_id_i64 << 16).wrapping_add(0xffff);
            let row_node_id = match ctx {
                Some(_) => renderer.register_accessibility_child_node_with_key(
                    row_key, row_node, row_rect, env, row_action,
                ),
                None => renderer.register_accessibility_child_node_with_key_semantic(
                    row_key, row_node, env, row_action,
                ),
            };
            if let Some(row_node_id) = row_node_id {
                table_node.push_child(row_node_id);
                if selected_ids.is_some() {
                    renderer.register_accessibility_focus_link(
                        &InteractionKey::for_rc(state, row_interaction_slot(row_id)),
                        row_node_id,
                    );
                }
            }
        }

        if !is_rendered_pass {
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
        let _ = (handle, active_sort, selected_ids);
    }
}

/// Measures a table leaf: its columns' widths from their realized headers
/// over the header and row bands (intrinsic-sized; proposal-independent).
pub fn measure_table_node<E: crate::text::TextEngine>(
    table: &TableRenderState<E>,
    _proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> ViewDimensions {
    if table.config.columns().is_empty() {
        return ViewDimensions::new(LayoutSize::zero());
    }
    let metrics = table.look(theme, env).metrics();
    let widths = table.header_widths(state, env, theme, metrics);
    let row_count = state.measure_signal(&table.config.rows().len());
    let height = metrics
        .row_height
        .mul_add(usize_as_f64(row_count), metrics.header_height);
    ViewDimensions::new(LayoutSize::new(
        f64_as_f32(widths.iter().sum()),
        f64_as_f32(height),
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

fn draw_sort_chevron<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    header_cell: kurbo::Rect,
    direction: SortDirection,
    env: &Environment,
) {
    let color_signal = color::MutedForeground.resolve(env).computed();
    let stroke_color = ctx.renderer_mut().read_signal(&color_signal);
    let cx = SORT_INDICATOR_SPACE
        .mul_add(-0.5, header_cell.x1)
        .max(header_cell.x0 + 4.0);
    let cy = header_cell.center().y;
    let mut path = kurbo::BezPath::new();
    match direction {
        SortDirection::Ascending => {
            path.move_to((cx - SORT_CHEVRON_HALF_W, cy + SORT_CHEVRON_HALF_H));
            path.line_to((cx, cy - SORT_CHEVRON_HALF_H));
            path.line_to((cx + SORT_CHEVRON_HALF_W, cy + SORT_CHEVRON_HALF_H));
        }
        SortDirection::Descending => {
            path.move_to((cx - SORT_CHEVRON_HALF_W, cy - SORT_CHEVRON_HALF_H));
            path.line_to((cx, cy + SORT_CHEVRON_HALF_H));
            path.line_to((cx + SORT_CHEVRON_HALF_W, cy - SORT_CHEVRON_HALF_H));
        }
    }
    ctx.draw_context(|draw| {
        draw.stroke(path, kurbo::Stroke::new(1.5), stroke_color);
    });
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous flush pass over headers, rows, selection, and cells"
)]
pub fn render_table_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<TableRenderState<E>>>,
    env: &Environment,
) {
    let col_count = state.borrow().config.columns().len();
    if col_count == 0 {
        return;
    }
    state.borrow().consume_replaced_rows();
    let snapshot = state.borrow().rows_snapshot.borrow().clone();
    let row_count = snapshot.len();
    let active_sort = state.borrow().active_sort(ctx.renderer_mut());
    let selected_ids = state
        .borrow()
        .row_selection
        .as_ref()
        .map(|sel| sel.selected_ids(ctx.renderer_mut()));
    let selection_fill = selected_ids.as_ref().map(|_| {
        let signal = color::SelectionContainer.resolve(env).computed();
        ctx.renderer_mut().read_signal(&signal)
    });

    let viewport = ctx.bounds;
    let extension = state.borrow().surface.extension();
    let surface_viewport = grow_rect(viewport, extension);
    let theme = ctx.theme();
    let look = state.borrow().look(&theme, env);
    let layout_metrics = look.metrics();
    state.borrow().refresh_headers(env, ctx.renderer_mut());
    state
        .borrow()
        .refresh_slot_baseline(row_count, ctx.state_mut(), env, &theme, layout_metrics);

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
    let row_window = resolve_table_visible_rows(
        row_span.start,
        row_span.end - row_span.start,
        row_count,
        layout_metrics,
    );
    let mut column_window = {
        let state_ref = state.borrow();
        let slot = state_ref.slot.borrow();
        resolve_visible_column_window(&slot.column_widths, column_span.start, column_span.end)
    };
    state.borrow().widen_to_visible_cells(
        &snapshot,
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

    crate::widgets::scroll::register_scroll_wheel_target(
        ctx.renderer_mut(),
        surface_viewport,
        &handle,
    );

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
            Rc::clone(&viewport_placement),
        );
        let world = renderer.record_world(kurbo::Affine::translate(kurbo::Vec2::new(
            viewport.x0,
            viewport.y0,
        )));
        let lazy_row_span = state
            .borrow()
            .surface
            .visible_span(&scroll_metrics, ScrollAxis::Vertical);
        let lazy_column_span = state
            .borrow()
            .surface
            .visible_span(&scroll_metrics, ScrollAxis::Horizontal);
        renderer.push_lazy_viewport(LazyViewport {
            bounds: kurbo::Rect::new(
                lazy_column_span.start,
                lazy_row_span.start,
                lazy_column_span.end,
                lazy_row_span.end,
            ),
            transform: world * kurbo::Affine::translate(-offset),
        });
    }

    let origin_x = viewport.x0;
    let origin_y = viewport.y0;
    let painted_width = table_metrics.table_width.max(viewport.width());
    {
        let table_rect = kurbo::Rect::new(
            origin_x,
            origin_y,
            origin_x + painted_width,
            origin_y + table_metrics.table_height,
        );
        let header_rect = kurbo::Rect::new(
            origin_x,
            origin_y,
            origin_x + painted_width,
            origin_y + layout_metrics.header_height,
        );
        ctx.draw_context(|draw| {
            look.record_background(&mut *draw, table_rect);
            look.record_header_background(&mut *draw, header_rect);
        });
    }

    // Draw selected row backgrounds and register row selection hit targets
    // before cells flush so interactive controls inside a cell take priority.
    if let (Some(selected_set), Some(fill), Some(row_selection)) = (
        &selected_ids,
        selection_fill,
        state.borrow().row_selection.clone(),
    ) {
        for row_index in row_window.start..row_window.end {
            let Some(row_id) = snapshot.get_id(row_index) else {
                continue;
            };
            let y0 = layout_metrics.row_height.mul_add(
                usize_as_f64(row_index),
                origin_y + layout_metrics.header_height,
            );
            let row_rect = kurbo::Rect::new(
                origin_x,
                y0,
                origin_x + painted_width,
                y0 + layout_metrics.row_height,
            );
            if selected_set.contains(&row_id) {
                ctx.draw_context(|draw| {
                    draw.fill(row_rect, fill);
                });
            }
            let key = InteractionKey::for_rc(state, row_interaction_slot(row_id));
            let (_, press_slot, _) = ctx
                .renderer_mut()
                .bind_interaction_target(key, row_rect, env);
            let row_selection = Rc::clone(&row_selection);
            ctx.renderer_mut().register_interactive_pointer_target(
                row_rect,
                press_slot,
                move |renderer: &mut SemanticCore<E>, _point, _env| {
                    row_selection.write(row_index, row_id, renderer.modifiers());
                    true
                },
            );
        }
    }

    // Flush visible column headers (with sort hit target & chevron) and data
    // cells, keeping virtualization bounded by visible cells.
    state.borrow().item_cache.borrow_mut().begin_frame();
    let cell_horizontal_inset = layout_metrics.cell_horizontal_padding * 0.5;

    // Realize every visible row once before iterating columns.
    for row_index in row_window.start..row_window.end {
        if let Some(row_id) = snapshot.get_id(row_index) {
            state.borrow().realize_row_cells(
                row_index,
                row_id,
                column_window.start..column_window.end,
                &snapshot,
                env,
                ctx.renderer_mut(),
            );
        }
    }

    let mut x_offset = column_window.leading_offset;
    for (column_index, width) in table_metrics
        .column_widths
        .iter()
        .copied()
        .enumerate()
        .take(column_window.end)
        .skip(column_window.start)
    {
        let header_cell =
            table_header_cell_rect(origin_x, origin_y, x_offset, width, layout_metrics);
        let sortable = state.borrow().config.column_is_sortable(column_index);
        let header_label_bounds = if sortable {
            kurbo::Rect::new(
                header_cell.x0,
                header_cell.y0,
                (header_cell.x1 - SORT_INDICATOR_SPACE).max(header_cell.x0),
                header_cell.y1,
            )
        } else {
            header_cell
        };
        let header_rect = inset_rect(
            header_label_bounds,
            cell_horizontal_inset,
            layout_metrics.cell_vertical_inset,
        );
        if sortable {
            let key = InteractionKey::for_rc(state, header_interaction_slot(column_index));
            let (_, press_slot, _) =
                ctx.renderer_mut()
                    .bind_interaction_target(key, header_cell, env);
            let weak = Rc::downgrade(state);
            ctx.renderer_mut().register_interactive_pointer_target(
                header_cell,
                press_slot,
                move |_renderer: &mut SemanticCore<E>, _point, _env| {
                    weak.upgrade().is_some_and(|state| {
                        state.borrow().config.toggle_sort(column_index);
                        true
                    })
                },
            );
        }
        {
            let state_ref = state.borrow();
            let mut headers = state_ref.headers.borrow_mut();
            let header = headers
                .get_mut(column_index)
                .expect("hydrolysis table: every column's header is realized before the flush");
            flush_cell_subview(ctx, env, header_rect, header);
        }
        if sortable
            && let Some(sort) = active_sort
            && sort.column == column_index
        {
            draw_sort_chevron(ctx, header_cell, sort.direction, env);
        }

        for row_index in row_window.start..row_window.end {
            let cell_rect = table_data_cell_rect(
                origin_x,
                origin_y,
                x_offset,
                width,
                row_index,
                layout_metrics,
            );
            if let Some(row_id) = snapshot.get_id(row_index) {
                let inset = inset_rect(
                    cell_rect,
                    cell_horizontal_inset,
                    layout_metrics.cell_vertical_inset,
                );
                let key = TableCellKey {
                    row: row_id,
                    column: column_index,
                };
                let state_ref = state.borrow();
                let mut cache = state_ref.item_cache.borrow_mut();
                if let Some(cell) = cache.get_mut(&key) {
                    flush_cell_subview(ctx, env, inset, cell);
                }
            }
            ctx.draw_context(|draw| {
                look.record_cell_border(&mut *draw, cell_rect);
            });
        }

        let separator = kurbo::Rect::new(
            origin_x + x_offset + width,
            origin_y,
            origin_x + x_offset + width,
            origin_y + table_metrics.table_height,
        );
        ctx.draw_context(|draw| {
            look.record_column_separator(&mut *draw, separator);
        });
        x_offset += width;
    }
    state.borrow().item_cache.borrow_mut().end_frame();

    {
        let renderer = ctx.renderer_mut();
        renderer.pop_lazy_viewport("hydrolysis table");
        renderer.program().end_inner();
        renderer.pop_placement_scope();
    }

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

/// Emits a retained table's accessibility tree for the semantic walk.
#[cfg(feature = "accessibility")]
#[cfg(hydrolysis_cherenkov)]
pub fn emit_table_accessibility<E: crate::text::TextEngine>(
    renderer: &mut SemanticCore<E>,
    state: &Rc<RefCell<TableRenderState<E>>>,
    env: &Environment,
) {
    table_accessibility(renderer, None, None, state, env);
}
