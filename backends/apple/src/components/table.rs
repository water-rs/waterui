//! The `table` leaf: row-major `Native<TableConfig<Held>>` rendered through an
//! `NSTableView` on macOS and a manually framed header/row grid on iOS.
//!
//! Rows carry collection identity (`RawId`) and produce arbitrary view cells
//! realized through `Renderer::realize`. Selection binds to row identities
//! (single or multiple) and sortable column headers bind to `TableSort<usize>`
//! with native sort indicators and accessibility state.

#[cfg(target_os = "macos")]
use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;
#[cfg(target_os = "macos")]
use core::cell::Cell;
use core::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
#[cfg(target_os = "macos")]
use std::ptr;

#[cfg(target_os = "ios")]
use cocoa_ui::accessibility::{Role, apply_state, set_role};
use cocoa_ui::{Retained, view};
#[cfg(target_os = "ios")]
use nami::SignalExt as _;
use nami::watcher::{BoxWatcherGuard, Metadata};
use nami::{Binding, Computed, Signal};
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;
use waterui_controls::table::{
    SortDirection, TableColumnDescriptor, TableConfig, TableRow, TableSelection, TableSort,
};
use waterui_core::id::Id as RawId;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView, ViewDimensions,
};
use waterui_core::views::AnyViewsSnapshot;
use waterui_text::StyledStr;

use crate::contract::{Held, NativeLeaf, RenderContext, Renderer};

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, TableSortIndicator, TableView};
#[cfg(target_os = "ios")]
use cocoa_ui::gesture::{ButtonMask, GestureState};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HostView, gesture::GestureAttachment, gesture::tap};

#[cfg(target_os = "macos")]
use cocoa_ui::objc2_app_kit::{NSTableColumn, NSTableHeaderView};
#[cfg(target_os = "ios")]
use waterui_controls::theme::color::SelectionContainer;
#[cfg(target_os = "ios")]
use waterui_core::resolve::Resolvable as _;
#[cfg(target_os = "ios")]
use waterui_graphics::color::WorkingColor;

/// A row's identity: the collection id `SharedAnyViews` answers for an index.
type ItemId = RawId;

/// Cell horizontal inset.
const HORIZONTAL_PADDING: f64 = 12.0;
/// Cell vertical inset.
const VERTICAL_PADDING: f64 = 6.0;
/// A column never narrows below this width.
const MINIMUM_COLUMN_WIDTH: f64 = 80.0;
/// Extra horizontal space reserved for a sortable column's sort indicator.
const SORT_INDICATOR_ALLOWANCE: f64 = 18.0;
/// Fixed macOS header band height.
#[cfg(target_os = "macos")]
const NATIVE_HEADER_HEIGHT: f64 = 24.0;
/// Minimum header band height on iOS.
#[cfg(target_os = "ios")]
const MIN_HEADER_HEIGHT: f64 = 28.0;
/// Minimum row height.
const MIN_ROW_HEIGHT: f64 = 28.0;

/// A cell's platform view.
#[cfg(target_os = "ios")]
type CellStore = crate::contract::Mounted;
/// A cell's platform view — unmounted on `AppKit`: `NSTableView` takes the
/// view in `viewFor` and releases it on reuse.
#[cfg(target_os = "macos")]
type CellStore = NativeLeaf;

/// One row's background & selection-tap container on iOS.
#[cfg(target_os = "ios")]
struct RowHost {
    host: Retained<HostView>,
    _tap: Option<GestureAttachment>,
}

/// A rendered column: descriptor metadata, header view/column, and label
/// observation.
struct ColumnState {
    sortable: bool,
    #[cfg(target_os = "ios")]
    header_host: Retained<HostView>,
    #[cfg(target_os = "ios")]
    label: crate::contract::Mounted,
    #[cfg(target_os = "ios")]
    _header_tap: Option<GestureAttachment>,
    label_content: Computed<StyledStr>,
    _label_guard: <Computed<StyledStr> as Signal>::Guard,
    #[cfg(target_os = "macos")]
    ns_column: Retained<NSTableColumn>,
}

/// The table's shared state.
struct TableState {
    host: Retained<HostView>,
    #[cfg(target_os = "macos")]
    table: Retained<TableView>,
    #[cfg(target_os = "macos")]
    header: Retained<NSTableHeaderView>,
    renderer: Renderer,
    columns: Vec<ColumnState>,
    snapshot: AnyViewsSnapshot<TableRow<Held>>,
    ids: Vec<ItemId>,
    cells: HashMap<(ItemId, usize), CellStore>,
    #[cfg(target_os = "ios")]
    row_hosts: HashMap<ItemId, RowHost>,
    #[cfg(target_os = "ios")]
    selection_fill: Option<Computed<WorkingColor>>,
    #[cfg(target_os = "macos")]
    row_heights: Vec<f64>,
    selection: TableSelection<RawId>,
    #[cfg(target_os = "macos")]
    applying_selection: Cell<bool>,
    sort_order: Option<Binding<Option<TableSort<usize>>>>,
}

impl TableState {
    const fn allows_selection(&self) -> bool {
        !matches!(self.selection, TableSelection::None)
    }

    fn selected_ids(&self) -> BTreeSet<RawId> {
        match &self.selection {
            TableSelection::None => BTreeSet::new(),
            TableSelection::Single(binding) => binding.snapshot().into_iter().collect(),
            TableSelection::Multiple(binding) => binding.snapshot(),
        }
    }

    fn sort_binding_for_column(
        &self,
        column_index: usize,
    ) -> Option<Binding<Option<TableSort<usize>>>> {
        let column = self.columns.get(column_index)?;
        if !column.sortable {
            return None;
        }
        self.sort_order.clone()
    }

    #[cfg(target_os = "macos")]
    fn selection_write_plan(&self, rows: &[usize]) -> Option<SelectionWrite> {
        if self.applying_selection.get() {
            return None;
        }
        match &self.selection {
            TableSelection::None => None,
            TableSelection::Single(binding) => {
                let id = rows.first().and_then(|&row| self.ids.get(row).copied());
                Some(SelectionWrite::Single(binding.clone(), id))
            }
            TableSelection::Multiple(binding) => {
                let ids: BTreeSet<RawId> = rows
                    .iter()
                    .filter_map(|&row| self.ids.get(row).copied())
                    .collect();
                Some(SelectionWrite::Multiple(binding.clone(), ids))
            }
        }
    }

    #[cfg(target_os = "ios")]
    fn selection_for_row(&self) -> TableSelection<RawId> {
        self.selection.clone()
    }
}

#[cfg(target_os = "macos")]
enum SelectionWrite {
    Single(Binding<Option<RawId>>, Option<RawId>),
    Multiple(Binding<BTreeSet<RawId>>, BTreeSet<RawId>),
}

fn toggle_sort(state: &Rc<RefCell<TableState>>, column_index: usize) {
    let binding = state.borrow().sort_binding_for_column(column_index);
    if let Some(binding) = binding {
        let next = TableSort::next_for_column(binding.snapshot(), column_index);
        binding.set(Some(next));
    }
}

#[cfg(target_os = "macos")]
fn write_selection_indices(state: &Rc<RefCell<TableState>>, rows: &[usize]) {
    let plan = state.borrow().selection_write_plan(rows);
    match plan {
        Some(SelectionWrite::Single(binding, id)) => binding.set(id),
        Some(SelectionWrite::Multiple(binding, ids)) => binding.set(ids),
        None => {}
    }
}

#[cfg(target_os = "ios")]
fn select_row_id(state: &Rc<RefCell<TableState>>, id: RawId) {
    let selection = state.borrow().selection_for_row();
    match selection {
        TableSelection::None => {}
        TableSelection::Single(binding) => binding.set(Some(id)),
        TableSelection::Multiple(binding) => {
            binding.with_mut(|selected| {
                if !selected.insert(id) {
                    selected.remove(&id);
                }
            });
        }
    }
}

/// Measures a leaf under the fully unspecified proposal.
fn intrinsic_size(layout: &dyn SubView) -> (f64, f64) {
    let measured = layout.measure(ProposalSize::UNSPECIFIED);
    (
        f64::from(measured.size.width),
        f64::from(measured.size.height),
    )
}

/// Fitted width for one column: `max(MINIMUM_COLUMN_WIDTH, header_width,
/// cell_widths + HORIZONTAL_PADDING * 2)`.
fn fitted_column_width(header_width: f64, cell_widths: impl Iterator<Item = f64>) -> f64 {
    cell_widths.fold(MINIMUM_COLUMN_WIDTH.max(header_width), |width, cell| {
        width.max(HORIZONTAL_PADDING.mul_add(2.0, cell))
    })
}

/// Fitted height for one row: `max(MIN_ROW_HEIGHT, cells + VERTICAL_PADDING * 2)`.
fn fitted_row_height(cell_heights: impl Iterator<Item = f64>) -> f64 {
    cell_heights
        .reduce(f64::max)
        .unwrap_or(0.0)
        .mul_add(1.0, VERTICAL_PADDING * 2.0)
        .max(MIN_ROW_HEIGHT)
}

#[cfg_attr(
    target_os = "macos",
    expect(
        clippy::missing_const_for_fn,
        reason = "the iOS branch measures leaves and cannot be const"
    )
)]
fn header_height(state: &TableState) -> f64 {
    if state.columns.is_empty() {
        return 0.0;
    }
    #[cfg(target_os = "ios")]
    {
        state
            .columns
            .iter()
            .map(|column| intrinsic_size(column.label.layout()).1)
            .reduce(f64::max)
            .unwrap_or(0.0)
            .mul_add(1.0, VERTICAL_PADDING * 2.0)
            .max(MIN_HEADER_HEIGHT)
    }
    #[cfg(target_os = "macos")]
    {
        NATIVE_HEADER_HEIGHT
    }
}

fn column_widths(state: &TableState) -> Vec<f64> {
    state
        .columns
        .iter()
        .enumerate()
        .map(|(col_index, column)| {
            let sort_allowance = if column.sortable {
                SORT_INDICATOR_ALLOWANCE
            } else {
                0.0
            };
            #[cfg(target_os = "ios")]
            let header_width = HORIZONTAL_PADDING.mul_add(
                2.0,
                intrinsic_size(column.label.layout()).0 + sort_allowance,
            );
            #[cfg(target_os = "macos")]
            let header_width = column.ns_column.headerCell().cellSize().width + sort_allowance;
            fitted_column_width(
                header_width,
                state
                    .ids
                    .iter()
                    .filter_map(|&id| state.cells.get(&(id, col_index)))
                    .map(|cell| intrinsic_size(cell.layout()).0),
            )
        })
        .collect()
}

fn row_heights(state: &TableState) -> Vec<f64> {
    let col_count = state.columns.len();
    state
        .ids
        .iter()
        .map(|&id| {
            fitted_row_height((0..col_count).filter_map(|col_index| {
                state
                    .cells
                    .get(&(id, col_index))
                    .map(|cell| intrinsic_size(cell.layout()).1)
            }))
        })
        .collect()
}

fn size_that_fits(state: &TableState) -> Size {
    let width: f64 = column_widths(state).iter().sum();
    let height = header_height(state) + row_heights(state).iter().sum::<f64>();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "kit geometry is f64; the layout contract is f32"
    )]
    Size::new(width as f32, height as f32)
}

#[cfg(target_os = "ios")]
fn update_attached_views(state: &TableState) {
    let mut desired: Vec<Retained<cocoa_ui::PlatformView>> = Vec::new();
    for column in &state.columns {
        desired.push(view::retain_base(&column.header_host));
    }
    for &id in &state.ids {
        if let Some(row_host) = state.row_hosts.get(&id) {
            desired.push(view::retain_base(&row_host.host));
            let mut row_cells: Vec<Retained<cocoa_ui::PlatformView>> =
                Vec::with_capacity(state.columns.len());
            for col_index in 0..state.columns.len() {
                if let Some(cell) = state.cells.get(&(id, col_index)) {
                    row_cells.push(view::retain_base(cell.view()));
                }
            }
            view::reconcile_subviews(&row_host.host, &row_cells);
        }
    }
    view::reconcile_subviews(&state.host, &desired);
}

#[cfg(target_os = "ios")]
fn apply_selection_and_sort_chrome(state: &TableState) {
    let selected_ids = state.selected_ids();
    let fill = state
        .selection_fill
        .as_ref()
        .map(|signal| crate::theme::platform_color(&signal.snapshot()));
    for &id in &state.ids {
        let is_selected = selected_ids.contains(&id);
        if let Some(row_host) = state.row_hosts.get(&id) {
            if is_selected {
                view::set_background_color(&row_host.host, fill.as_deref());
            } else {
                view::set_background_color(&row_host.host, None);
            }
            if state.allows_selection() {
                apply_state(
                    &row_host.host,
                    false,
                    is_selected,
                    None,
                    false,
                    None,
                    false,
                    None,
                    false,
                );
            }
        }
    }
    let active_sort = state.sort_order.as_ref().and_then(Signal::snapshot);
    for (col_index, column) in state.columns.iter().enumerate() {
        let title = column.label_content.snapshot().to_plain();
        let value = match active_sort {
            Some(sort) if sort.column == col_index => match sort.direction {
                SortDirection::Ascending => Some("ascending"),
                SortDirection::Descending => Some("descending"),
            },
            _ => None,
        };
        view::set_accessibility_content(&column.header_host, Some(&title), value);
    }
}

#[cfg(target_os = "ios")]
fn reload_content(state: &TableState) {
    update_attached_views(state);
    apply_selection_and_sort_chrome(state);
    state.host.set_needs_layout();
    crate::measure_memo::invalidate();
    view::invalidate_layout(&state.host);
    crate::measure_memo::invalidate();
}

#[cfg(target_os = "macos")]
fn apply_binding_selection(state: &Rc<RefCell<TableState>>) {
    let (table, indexes) = {
        let borrowed = state.borrow();
        if borrowed.applying_selection.get() {
            return;
        }
        borrowed.applying_selection.set(true);
        let indexes = if borrowed.allows_selection() {
            let selected_ids = borrowed.selected_ids();
            borrowed
                .ids
                .iter()
                .enumerate()
                .filter_map(|(index, id)| selected_ids.contains(id).then_some(index))
                .collect()
        } else {
            Vec::new()
        };
        (borrowed.table.clone(), indexes)
    };
    table.set_selected_rows(&indexes);
    state.borrow().applying_selection.set(false);
}

#[cfg(target_os = "macos")]
fn apply_sort_indicators(state: &Rc<RefCell<TableState>>) {
    let (table, header, updates, highlighted) = {
        let borrowed = state.borrow();
        let active_sort = borrowed.sort_order.as_ref().and_then(Signal::snapshot);
        let mut highlighted: Option<Retained<NSTableColumn>> = None;
        let updates: Vec<(Retained<NSTableColumn>, Option<TableSortIndicator>)> = borrowed
            .columns
            .iter()
            .enumerate()
            .map(|(col_index, column)| {
                let indicator = match active_sort {
                    Some(sort) if sort.column == col_index && column.sortable => {
                        highlighted = Some(column.ns_column.clone());
                        Some(match sort.direction {
                            SortDirection::Ascending => TableSortIndicator::Ascending,
                            SortDirection::Descending => TableSortIndicator::Descending,
                        })
                    }
                    _ => None,
                };
                (column.ns_column.clone(), indicator)
            })
            .collect();
        (
            borrowed.table.clone(),
            borrowed.header.clone(),
            updates,
            highlighted,
        )
    };
    for (ns_column, indicator) in &updates {
        table.set_sort_indicator(ns_column, *indicator);
    }
    table.set_highlighted_column(highlighted.as_deref());
    TableView::refresh_header(&header);
}

#[cfg(target_os = "macos")]
fn reload_content(state: &Rc<RefCell<TableState>>) {
    let (table, host) = {
        let mut borrowed = state.borrow_mut();
        for column in &borrowed.columns {
            column.ns_column.setTitle(&NSString::from_str(
                &column.label_content.snapshot().to_plain(),
            ));
        }
        borrowed.row_heights = row_heights(&borrowed);
        for (column, width) in borrowed.columns.iter().zip(column_widths(&borrowed)) {
            column.ns_column.setWidth(width);
        }
        (borrowed.table.clone(), borrowed.host.clone())
    };
    // Run every AppKit NSTableView method with no `RefCell` borrow held so
    // synchronous data-source and delegate callbacks can read `state`.
    table.reload_data();
    apply_binding_selection(state);
    apply_sort_indicators(state);
    host.set_needs_layout();
    crate::measure_memo::invalidate();
    view::invalidate_layout(&host);
    crate::measure_memo::invalidate();
}

/// Reconciles rendered cells for every row id: keeps surviving rows untouched,
/// materializes cells for joining rows via `Renderer::realize`, and drops
/// departed rows.
fn sync_rows(state_rc: &Rc<RefCell<TableState>>, ids: Vec<ItemId>) {
    let mut state = state_rc.borrow_mut();
    let col_count = state.columns.len();
    let renderer = state.renderer.clone();
    #[cfg(target_os = "ios")]
    let mtm = renderer.context().mtm();
    #[cfg(target_os = "ios")]
    let allows_selection = state.allows_selection();

    for (row_index, &id) in ids.iter().enumerate() {
        #[cfg(target_os = "ios")]
        let row_host_view = state
            .row_hosts
            .entry(id)
            .or_insert_with(|| {
                let row_host = HostView::new(mtm, cocoa_ui::Rect::ZERO);
                let tap = allows_selection.then(|| {
                    let weak = Rc::downgrade(state_rc);
                    tap(&row_host, 1, ButtonMask::PRIMARY, move |phase, _| {
                        if phase == GestureState::Ended
                            && let Some(state) = weak.upgrade()
                        {
                            select_row_id(&state, id);
                        }
                    })
                });
                RowHost {
                    host: row_host,
                    _tap: tap,
                }
            })
            .host
            .clone();

        let needs_build = (0..col_count).any(|col| !state.cells.contains_key(&(id, col)));
        if !needs_build {
            continue;
        }
        let Some(row) = state.snapshot.get_view(row_index) else {
            continue;
        };
        for (col_index, cell) in row.into_cells().into_iter().enumerate() {
            if state.cells.contains_key(&(id, col_index)) {
                continue;
            }
            let leaf = renderer.realize(cell);
            #[cfg(target_os = "ios")]
            let stored = {
                let mounted = leaf.mount(&row_host_view);
                view::set_translates_autoresizing(mounted.view(), true);
                mounted
            };
            #[cfg(target_os = "macos")]
            let stored = leaf;
            state.cells.insert((id, col_index), stored);
        }
    }

    let active: BTreeSet<ItemId> = ids.iter().copied().collect();
    let dropped_cells: Vec<(ItemId, usize)> = state
        .cells
        .keys()
        .copied()
        .filter(|(id, _)| !active.contains(id))
        .collect();
    for key in dropped_cells {
        if let Some(cell) = state.cells.remove(&key) {
            #[cfg(target_os = "macos")]
            view::remove_from_superview(cell.view());
            drop(cell);
        }
    }
    #[cfg(target_os = "ios")]
    {
        let dropped_hosts: Vec<ItemId> = state
            .row_hosts
            .keys()
            .copied()
            .filter(|id| !active.contains(id))
            .collect();
        for id in dropped_hosts {
            if let Some(row_host) = state.row_hosts.remove(&id) {
                view::remove_from_superview(&row_host.host);
            }
        }
    }

    state.ids = ids;
}

#[derive(Default)]
struct TablePending {
    depth: usize,
    rows: Option<(AnyViewsSnapshot<TableRow<Held>>, Vec<ItemId>, Metadata)>,
    reload: bool,
    selection_changed: bool,
    sort_changed: bool,
}

fn drain_table(state: &Rc<RefCell<TableState>>, pending: &Rc<RefCell<TablePending>>) {
    pending.borrow_mut().depth += 1;
    loop {
        let rows_event = pending.borrow_mut().rows.take();
        if let Some((snapshot, ids, metadata)) = rows_event {
            crate::animation::with_platform_animation(&metadata, {
                let state = Rc::clone(state);
                move || {
                    state.borrow_mut().snapshot = snapshot;
                    sync_rows(&state, ids);
                    #[cfg(target_os = "ios")]
                    reload_content(&state.borrow());
                    #[cfg(target_os = "macos")]
                    reload_content(&state);
                }
            });
            continue;
        }
        let mut progressed = false;
        if pending.borrow_mut().reload {
            pending.borrow_mut().reload = false;
            progressed = true;
            #[cfg(target_os = "ios")]
            reload_content(&state.borrow());
            #[cfg(target_os = "macos")]
            reload_content(state);
        }
        if pending.borrow_mut().selection_changed {
            pending.borrow_mut().selection_changed = false;
            progressed = true;
            #[cfg(target_os = "ios")]
            apply_selection_and_sort_chrome(&state.borrow());
            #[cfg(target_os = "macos")]
            apply_binding_selection(state);
        }
        if pending.borrow_mut().sort_changed {
            pending.borrow_mut().sort_changed = false;
            progressed = true;
            #[cfg(target_os = "ios")]
            apply_selection_and_sort_chrome(&state.borrow());
            #[cfg(target_os = "macos")]
            apply_sort_indicators(state);
        }
        if !progressed {
            break;
        }
    }
    pending.borrow_mut().depth -= 1;
}

fn with_table_tx<T>(
    state: &Rc<RefCell<TableState>>,
    pending: &Rc<RefCell<TablePending>>,
    body: impl FnOnce() -> T,
) -> T {
    pending.borrow_mut().depth += 1;
    let result = body();
    let outermost = {
        let mut pending = pending.borrow_mut();
        pending.depth -= 1;
        pending.depth == 0
    };
    if outermost {
        drain_table(state, pending);
    }
    result
}

fn materialize_column(
    state: &Rc<RefCell<TableState>>,
    pending: &Rc<RefCell<TablePending>>,
    descriptor: &TableColumnDescriptor,
    col_index: usize,
    has_sort_binding: bool,
) -> ColumnState {
    let sortable = has_sort_binding && descriptor.is_sortable();
    let (renderer, env) = {
        let state = state.borrow();
        (
            state.renderer.clone(),
            state.renderer.context().env().clone(),
        )
    };
    let label_content = descriptor.label().resolve(&env).into_content();
    let weak = Rc::downgrade(state);
    let label_guard = label_content.watch({
        let pending = Rc::clone(pending);
        move |_ctx| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            pending.borrow_mut().reload = true;
            if pending.borrow().depth == 0 {
                drain_table(&state, &pending);
            }
        }
    });

    #[cfg(target_os = "ios")]
    let (header_host, label, header_tap) = {
        let mtm = renderer.context().mtm();
        let header_host = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let role = if sortable { Role::Button } else { Role::Header };
        set_role(&header_host, role);
        let leaf = renderer.render(descriptor.label().clone());
        let mounted = leaf.mount(&header_host);
        view::set_translates_autoresizing(mounted.view(), true);
        let header_tap = sortable.then(|| {
            let weak = Rc::downgrade(state);
            tap(&header_host, 1, ButtonMask::PRIMARY, move |phase, _| {
                if phase == GestureState::Ended
                    && let Some(state) = weak.upgrade()
                {
                    toggle_sort(&state, col_index);
                }
            })
        });
        (header_host, mounted, header_tap)
    };

    #[cfg(target_os = "macos")]
    let ns_column = {
        let id = descriptor.identity();
        let ns_column = NSTableColumn::initWithIdentifier(
            renderer.context().mtm().alloc::<NSTableColumn>(),
            &NSString::from_str(&format!("waterui.table.{col_index}.{id}")),
        );
        ns_column.setMinWidth(MINIMUM_COLUMN_WIDTH);
        ns_column.setTitle(&NSString::from_str(&label_content.snapshot().to_plain()));
        ns_column
    };

    ColumnState {
        sortable,
        #[cfg(target_os = "ios")]
        header_host,
        #[cfg(target_os = "ios")]
        label,
        #[cfg(target_os = "ios")]
        _header_tap: header_tap,
        label_content,
        _label_guard: label_guard,
        #[cfg(target_os = "macos")]
        ns_column,
    }
}

#[cfg(target_os = "ios")]
fn layout_children(state: &TableState) {
    let widths = column_widths(state);
    let total_width: f64 = widths.iter().sum();
    let header_height = header_height(state);
    let row_heights = row_heights(state);
    let mut x = 0.0;
    for (index, column) in state.columns.iter().enumerate() {
        view::set_frame(
            &column.header_host,
            cocoa_ui::Rect::new(x, 0.0, widths[index], header_height),
        );
        view::set_frame(
            column.label.view(),
            cocoa_ui::Rect::new(
                HORIZONTAL_PADDING,
                VERTICAL_PADDING,
                HORIZONTAL_PADDING.mul_add(-2.0, widths[index]).max(0.0),
                VERTICAL_PADDING.mul_add(-2.0, header_height).max(0.0),
            ),
        );
        x += widths[index];
    }
    let mut y = header_height;
    for (row_index, row_height) in row_heights.iter().enumerate() {
        if let Some(&id) = state.ids.get(row_index) {
            if let Some(row_host) = state.row_hosts.get(&id) {
                view::set_frame(
                    &row_host.host,
                    cocoa_ui::Rect::new(0.0, y, total_width, *row_height),
                );
            }
            x = 0.0;
            for (col_index, &col_width) in widths.iter().enumerate() {
                if let Some(cell) = state.cells.get(&(id, col_index)) {
                    view::set_frame(
                        cell.view(),
                        cocoa_ui::Rect::new(
                            x + HORIZONTAL_PADDING,
                            VERTICAL_PADDING,
                            HORIZONTAL_PADDING.mul_add(-2.0, col_width).max(0.0),
                            VERTICAL_PADDING.mul_add(-2.0, *row_height).max(0.0),
                        ),
                    );
                }
                x += col_width;
            }
        }
        y += row_height;
    }
}

#[cfg(target_os = "macos")]
fn layout_children(state: &TableState) {
    for (column, width) in state.columns.iter().zip(column_widths(state)) {
        column.ns_column.setWidth(width);
    }
    let header_height = header_height(state);
    let bounds = view::bounds(&state.host);
    view::set_frame(
        &state.header,
        cocoa_ui::Rect::new(0.0, 0.0, bounds.size.width, header_height),
    );
    view::set_frame(
        &state.table,
        cocoa_ui::Rect::new(
            0.0,
            header_height,
            bounds.size.width,
            (bounds.size.height - header_height).max(0.0),
        ),
    );
}

struct TableSubView {
    state: Rc<RefCell<TableState>>,
}

impl core::fmt::Debug for TableSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TableSubView").finish_non_exhaustive()
    }
}

impl SubView for TableSubView {
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(size_that_fits(&self.state.borrow()))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

#[cfg(target_os = "macos")]
fn install_table_handlers(state: &Rc<RefCell<TableState>>) {
    let table = &state.borrow().table;
    table.set_allows_multiple_selection(matches!(
        state.borrow().selection,
        TableSelection::Multiple(_)
    ));
    table.set_row_count_handler({
        let weak = Rc::downgrade(state);
        move || weak.upgrade().map_or(0, |state| state.borrow().ids.len())
    });
    table.set_row_height_handler({
        let weak = Rc::downgrade(state);
        move |row| {
            weak.upgrade().map_or(MIN_ROW_HEIGHT, |state| {
                state
                    .borrow()
                    .row_heights
                    .get(row)
                    .copied()
                    .unwrap_or(MIN_ROW_HEIGHT)
            })
        }
    });
    table.set_cell_view_handler({
        let weak = Rc::downgrade(state);
        move |ns_column, row| {
            let state = weak.upgrade()?;
            let state = state.borrow();
            let col_index = state.columns.iter().position(|column| {
                ptr::eq(ptr::from_ref(&*column.ns_column), ptr::from_ref(ns_column))
            })?;
            let &id = state.ids.get(row)?;
            let cell = state.cells.get(&(id, col_index))?;
            Some(view::retain_base(cell.view()))
        }
    });
    table.set_should_select_row_handler({
        let weak = Rc::downgrade(state);
        move |_row| {
            weak.upgrade()
                .is_some_and(|state| state.borrow().allows_selection())
        }
    });
    table.set_selection_did_change_handler({
        let weak = Rc::downgrade(state);
        move |selected_rows| {
            if let Some(state) = weak.upgrade() {
                write_selection_indices(&state, selected_rows);
            }
        }
    });
    table.set_column_clicked_handler({
        let weak = Rc::downgrade(state);
        move |ns_column| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let col_index = state.borrow().columns.iter().position(|column| {
                ptr::eq(ptr::from_ref(&*column.ns_column), ptr::from_ref(ns_column))
            });
            if let Some(col_index) = col_index {
                toggle_sort(&state, col_index);
            }
        }
    });
}

fn watch_table_inputs(
    state: &Rc<RefCell<TableState>>,
    pending: &Rc<RefCell<TablePending>>,
    config: &TableConfig<Held>,
) -> Vec<BoxWatcherGuard> {
    let mut guards: Vec<BoxWatcherGuard> = Vec::new();
    guards.push(config.rows().watch(.., {
        let weak = Rc::downgrade(state);
        let pending = Rc::clone(pending);
        move |ctx, _change| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let snapshot = ctx.value().clone();
            let metadata = ctx.metadata().clone();
            let ids: Vec<ItemId> = snapshot
                .range()
                .filter_map(|index| snapshot.get_id(index))
                .collect();
            pending.borrow_mut().rows = Some((snapshot, ids, metadata));
            if pending.borrow().depth == 0 {
                drain_table(&state, &pending);
            }
        }
    }));

    match config.selection() {
        TableSelection::None => {}
        TableSelection::Single(binding) => {
            let weak = Rc::downgrade(state);
            let pending = Rc::clone(pending);
            guards.push(Box::new(binding.watch(move |_ctx| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                pending.borrow_mut().selection_changed = true;
                if pending.borrow().depth == 0 {
                    drain_table(&state, &pending);
                }
            })));
        }
        TableSelection::Multiple(binding) => {
            let weak = Rc::downgrade(state);
            let pending = Rc::clone(pending);
            guards.push(Box::new(binding.watch(move |_ctx| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                pending.borrow_mut().selection_changed = true;
                if pending.borrow().depth == 0 {
                    drain_table(&state, &pending);
                }
            })));
        }
    }

    if let Some(binding) = config.sort_order() {
        let weak = Rc::downgrade(state);
        let pending = Rc::clone(pending);
        guards.push(Box::new(binding.watch(move |_ctx| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            pending.borrow_mut().sort_changed = true;
            if pending.borrow().depth == 0 {
                drain_table(&state, &pending);
            }
        })));
    }

    #[cfg(target_os = "ios")]
    if let Some(signal) = state.borrow().selection_fill.as_ref() {
        let weak = Rc::downgrade(state);
        let pending = Rc::clone(pending);
        guards.push(Box::new(signal.watch(move |_ctx| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            pending.borrow_mut().selection_changed = true;
            if pending.borrow().depth == 0 {
                drain_table(&state, &pending);
            }
        })));
    }

    guards
}

fn install_host_handlers(
    host: &HostView,
    state: &Rc<RefCell<TableState>>,
    pending: &Rc<RefCell<TablePending>>,
) {
    host.set_measure_handler({
        let state = Rc::clone(state);
        let pending = Rc::clone(pending);
        move |_host, _proposal| {
            let size = with_table_tx(&state, &pending, || size_that_fits(&state.borrow()));
            cocoa_ui::Size::new(f64::from(size.width), f64::from(size.height))
        }
    });
    host.set_layout_handler({
        let state = Rc::clone(state);
        let pending = Rc::clone(pending);
        move |_host| with_table_tx(&state, &pending, || layout_children(&state.borrow()))
    });
}

fn render(config: &TableConfig<Held>, ctx: &RenderContext<'_>) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, cocoa_ui::Rect::ZERO);
    #[cfg(target_os = "ios")]
    view::set_clips_to_bounds(&host, true);

    #[cfg(target_os = "macos")]
    let (table, header) = {
        let table = TableView::new(mtm);
        let header = table.install_header();
        host.add_subview(&table);
        host.add_subview(&header);
        (table, header)
    };

    let descriptors = config.columns();
    let selection = config.selection().clone();
    let sort_order = config.sort_order().cloned();
    let has_sort_binding = sort_order.is_some();
    #[cfg(target_os = "ios")]
    let selection_fill = (!matches!(selection, TableSelection::None))
        .then(|| SelectionContainer.resolve(ctx.env()).computed());

    let initial_snapshot = config.rows().snapshot();
    let initial_ids: Vec<ItemId> = initial_snapshot
        .range()
        .filter_map(|index| initial_snapshot.get_id(index))
        .collect();

    let state = Rc::new(RefCell::new(TableState {
        host: host.clone(),
        #[cfg(target_os = "macos")]
        table,
        #[cfg(target_os = "macos")]
        header,
        renderer: ctx.renderer(),
        columns: Vec::with_capacity(descriptors.len()),
        snapshot: initial_snapshot,
        ids: Vec::new(),
        cells: HashMap::new(),
        #[cfg(target_os = "ios")]
        row_hosts: HashMap::new(),
        #[cfg(target_os = "ios")]
        selection_fill,
        #[cfg(target_os = "macos")]
        row_heights: Vec::new(),
        selection,
        #[cfg(target_os = "macos")]
        applying_selection: Cell::new(false),
        sort_order,
    }));

    let pending = Rc::new(RefCell::new(TablePending::default()));

    let columns: Vec<ColumnState> = descriptors
        .iter()
        .enumerate()
        .map(|(col_index, descriptor)| {
            materialize_column(&state, &pending, descriptor, col_index, has_sort_binding)
        })
        .collect();
    state.borrow_mut().columns = columns;

    #[cfg(target_os = "macos")]
    {
        let (table, ordered) = {
            let borrowed = state.borrow();
            let ordered: Vec<Retained<NSTableColumn>> = borrowed
                .columns
                .iter()
                .map(|column| column.ns_column.clone())
                .collect();
            (borrowed.table.clone(), ordered)
        };
        table.set_columns(&ordered);
        install_table_handlers(&state);
    }

    install_host_handlers(&host, &state, &pending);

    let guards = watch_table_inputs(&state, &pending, config);

    with_table_tx(&state, &pending, || {
        sync_rows(&state, initial_ids);
        #[cfg(target_os = "ios")]
        reload_content(&state.borrow());
        #[cfg(target_os = "macos")]
        reload_content(&state);
    });

    let mut leaf = NativeLeaf::new(
        &*host,
        TableSubView {
            state: Rc::clone(&state),
        },
    );
    leaf.keep(guards);
    leaf.keep(state);
    leaf
}

/// Realizes a row-major `TableConfig<Held>`.
pub fn realize_table_config(value: &TableConfig<Held>, ctx: &RenderContext<'_>) -> NativeLeaf {
    render(value, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_f64_eq(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < f64::EPSILON,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn column_width_floors_at_the_minimum() {
        assert_f64_eq(
            fitted_column_width(0.0, [].into_iter()),
            MINIMUM_COLUMN_WIDTH,
        );
    }

    #[test]
    fn column_width_takes_the_header_when_wider() {
        assert_f64_eq(fitted_column_width(120.0, [40.0].into_iter()), 120.0);
    }

    #[test]
    fn column_width_takes_the_widest_cell_plus_padding() {
        assert_f64_eq(
            fitted_column_width(30.0, [100.0, 50.0].into_iter()),
            HORIZONTAL_PADDING.mul_add(2.0, 100.0),
        );
    }

    #[test]
    fn row_height_floors_at_28() {
        assert_f64_eq(fitted_row_height([10.0].into_iter()), MIN_ROW_HEIGHT);
        assert_f64_eq(fitted_row_height([].into_iter()), MIN_ROW_HEIGHT);
    }

    #[test]
    fn row_height_takes_the_tallest_cell_plus_padding() {
        assert_f64_eq(
            fitted_row_height([20.0, 44.0].into_iter()),
            VERTICAL_PADDING.mul_add(2.0, 44.0),
        );
    }
}
