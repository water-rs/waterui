//! The `table` leaf: `Native<TableConfig>` rendered through an
//! `NSTableView` on macOS and a manually framed header/cell grid on iOS.
//!
//! Mirrors `WuiTable` + `WuiTableColumnNode`: columns reconcile by their
//! `semantic_id` inside `with_platform_animation`, each column keeps an
//! id-keyed set of rendered cells driven by `rows().watch(..)`, cells are
//! `ctx.render(AnyView::new(text))` leaves measured under
//! `ProposalSize::UNSPECIFIED`, and `sizeThatFits` ignores the proposal —
//! width is the sum of the per-column fitted widths, height is the header
//! band plus the per-row maxima.
//!
//! Contract friction: `SubView` has no `place` hook, so the
//! `setPlacementProposal(WuiProposalSize())` the Swift port delivered to
//! each cell at layout/viewFor time has no channel here — cells measure
//! under unspecified proposals like every other ported container's
//! children.

#[cfg(target_os = "macos")]
use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use std::collections::HashMap;

use cocoa_ui::{Retained, view};
use waterui::animation::Animation;
use waterui::component::table::{TableColumn, TableConfig};
use waterui::id::{Id as RawId, SelfId};
use waterui::reactive::watcher::Metadata;
use waterui::reactive::{Computed, Signal};
use waterui::text::{StyledStr, Text};
use waterui::views::{AnyViewsSnapshot, ViewSnapshot, Views};
use waterui_backend_core::AnyView;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{NativeLeaf, RenderContext, Renderer};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, TableView};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

#[cfg(target_os = "macos")]
use cocoa_ui::objc2_app_kit::{NSTableColumn, NSTableHeaderView};

/// A row's identity: the collection id `Views` answers for an index.
type ItemId = SelfId<RawId>;

/// `HORIZONTAL_PADDING` — the cell inset on the horizontal axis.
const HORIZONTAL_PADDING: f64 = 12.0;
/// `VERTICAL_PADDING` — the cell inset on the vertical axis.
const VERTICAL_PADDING: f64 = 6.0;
/// `minimumColumnWidth` — a column never narrows below this.
const MINIMUM_COLUMN_WIDTH: f64 = 80.0;
/// `nativeHeaderHeight` — the macOS header band's fixed height.
#[cfg(target_os = "macos")]
const NATIVE_HEADER_HEIGHT: f64 = 24.0;
/// The minimum header band height on iOS (`max(28, ...)`).
#[cfg(target_os = "ios")]
const MIN_HEADER_HEIGHT: f64 = 28.0;
/// The minimum row height (`max(28, ...)`).
const MIN_ROW_HEIGHT: f64 = 28.0;

/// `withPlatformAnimation`: the watcher metadata's `Animation` mapped to a
/// kit timing — the same mapping `container` applies.
fn with_platform_animation(metadata: &Metadata, body: impl FnOnce() + 'static) {
    let timing = match metadata.try_get::<Animation>() {
        None => return body(),
        Some(Animation::Default) => cocoa_ui::core_animation::Timing::Bezier {
            duration: 0.25,
            control_points: [0.42, 0.0, 0.58, 1.0],
        },
        Some(Animation::Bezier {
            duration,
            x1,
            y1,
            x2,
            y2,
        }) => cocoa_ui::core_animation::Timing::Bezier {
            duration: duration.as_secs_f64(),
            control_points: [x1, y1, x2, y2],
        },
        Some(Animation::Spring { stiffness, damping }) => {
            cocoa_ui::core_animation::Timing::Spring {
                stiffness: f64::from(stiffness),
                damping: f64::from(damping),
            }
        }
    };
    cocoa_ui::core_animation::animate_with(timing, body);
}

/// A cell's platform view.
#[cfg(target_os = "ios")]
type CellStore = crate::contract::Mounted;
/// A cell's platform view — unmounted on `AppKit`: `NSTableView` takes the
/// view in `viewFor` and releases it on reuse.
#[cfg(target_os = "macos")]
type CellStore = NativeLeaf;

/// A rendered column: the `WuiTableColumnNode` — its rows collection,
/// id-keyed cells, header label, and watcher guards.
struct ColumnState {
    /// `semantic_id` — the reconcile key.
    semantic_id: usize,
    /// The retained rows snapshot aligned with `ids` — every
    /// `get_id`/`get_view` answer during reconcile comes from it, never
    /// the live collection.
    snapshot: AnyViewsSnapshot<Text>,
    /// Current row ids in display order — the applied membership; the
    /// row count answers `ids.len()`, never the live `rows.len()`.
    ids: Vec<ItemId>,
    /// Coalesced `rows` notifications for this column — outside
    /// `TableState`, so an emission landing while cell materialization
    /// holds the state borrow never touches it.
    pending: Rc<RefCell<PendingColumnRows>>,
    /// Rendered cells by row id.
    cells: HashMap<ItemId, CellStore>,
    /// `UILabel` header — the label's rendered `Text` leaf (iOS).
    #[cfg(target_os = "ios")]
    label: crate::contract::Mounted,
    /// The label's `content` signal — on `AppKit` the header title reads its
    /// snapshot; the watch drives `reloadContent`.
    #[cfg(target_os = "macos")]
    label_content: Computed<StyledStr>,
    /// The same signal on `UIKit`, kept alive so `_label_guard`'s watch stays
    /// subscribed; the rendered label leaf reads its own copy.
    #[cfg(target_os = "ios")]
    _label_content: Computed<StyledStr>,
    /// `labelContentObservation` / `labelObservation` — kept alive by the
    /// field, never read: dropping it unsubscribes the watch.
    _label_guard: <Computed<StyledStr> as Signal>::Guard,
    /// The `rows.watch` guard — kept alive like `_label_guard`.
    _rows_guard: waterui::reactive::watcher::BoxWatcherGuard,
    /// `nativeColumns[id]` (macOS).
    #[cfg(target_os = "macos")]
    ns_column: Retained<NSTableColumn>,
}

/// The table's shared state — `WuiTable`'s stored properties.
struct TableState {
    /// The leaf's view: the `WuiTable` itself (`UIView` / `NSView` host).
    host: Retained<HostView>,
    /// `tableView` (macOS).
    #[cfg(target_os = "macos")]
    table: Retained<TableView>,
    /// `nativeHeader` (macOS).
    #[cfg(target_os = "macos")]
    header: Retained<NSTableHeaderView>,
    /// Renders row `Text`s and label `Text`s after `install` returns.
    renderer: Renderer,
    /// `collection.ordered` — columns in display order.
    columns: Vec<ColumnState>,
    /// `appKitRowHeights` — the cached row heights the delegate answers.
    #[cfg(target_os = "macos")]
    row_heights: Vec<f64>,
}

/// `sizeThatFits` measure of a leaf under the fully unspecified proposal —
/// `row.sizeThatFits(WuiProposalSize())`.
fn intrinsic_size(layout: &dyn SubView) -> (f64, f64) {
    let measured = layout.measure(ProposalSize::UNSPECIFIED);
    (
        f64::from(measured.size.width),
        f64::from(measured.size.height),
    )
}

/// `columnWidths` for one column: `max(minimumColumnWidth, headerWidth,
/// cellWidths + HORIZONTAL_PADDING * 2)`.
fn fitted_column_width(header_width: f64, cell_widths: impl Iterator<Item = f64>) -> f64 {
    cell_widths.fold(MINIMUM_COLUMN_WIDTH.max(header_width), |width, cell| {
        width.max(HORIZONTAL_PADDING.mul_add(2.0, cell))
    })
}

/// One entry of `rowHeights`: `max(28, cells + VERTICAL_PADDING * 2)`.
fn fitted_row_height(cell_heights: impl Iterator<Item = f64>) -> f64 {
    cell_heights
        .reduce(f64::max)
        .unwrap_or(0.0)
        .mul_add(1.0, VERTICAL_PADDING * 2.0)
        .max(MIN_ROW_HEIGHT)
}

/// `headerHeight` on iOS: `max(28, labels + VERTICAL_PADDING * 2)`; on
/// macOS the fixed `nativeHeaderHeight`. Empty table answers `0`.
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

/// `columnWidths` for every column.
fn column_widths(state: &TableState) -> Vec<f64> {
    state
        .columns
        .iter()
        .map(|column| {
            #[cfg(target_os = "ios")]
            let header_width =
                HORIZONTAL_PADDING.mul_add(2.0, intrinsic_size(column.label.layout()).0);
            // The native header cell measures its own title, padding
            // included.
            #[cfg(target_os = "macos")]
            let header_width = column.ns_column.headerCell().cellSize().width;
            fitted_column_width(
                header_width,
                column
                    .ids
                    .iter()
                    .filter_map(|id| column.cells.get(id))
                    .map(|cell| intrinsic_size(cell.layout()).0),
            )
        })
        .collect()
}

/// `rowHeights`: per row index, the max fitted cell height across columns.
fn row_heights(state: &TableState) -> Vec<f64> {
    let count = state
        .columns
        .iter()
        .map(|column| column.ids.len())
        .max()
        .unwrap_or(0);
    (0..count)
        .map(|row| {
            fitted_row_height(state.columns.iter().filter_map(|column| {
                column
                    .ids
                    .get(row)
                    .and_then(|id| column.cells.get(id))
                    .map(|cell| intrinsic_size(cell.layout()).1)
            }))
        })
        .collect()
}

/// `sizeThatFits` — the proposal is ignored on both platforms.
fn size_that_fits(state: &TableState) -> Size {
    let width: f64 = column_widths(state).iter().sum();
    let height = header_height(state) + row_heights(state).iter().sum::<f64>();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "kit geometry is f64; the layout contract is f32"
    )]
    Size::new(width as f32, height as f32)
}

/// `updateAttachedViews` (iOS): the host's subviews are exactly the labels
/// in column order, then every column's cells in row order.
#[cfg(target_os = "ios")]
fn update_attached_views(state: &TableState) {
    let desired: Vec<Retained<cocoa_ui::PlatformView>> = state
        .columns
        .iter()
        .map(|column| view::retain_base(column.label.view()))
        .chain(state.columns.iter().flat_map(|column| {
            column
                .ids
                .iter()
                .filter_map(|id| column.cells.get(id))
                .map(|cell| view::retain_base(cell.view()))
        }))
        .collect();
    view::reconcile_subviews(&state.host, &desired);
}

/// `reloadContent` (iOS): reconcile the attached views and invalidate.
#[cfg(target_os = "ios")]
fn reload_content(state: &TableState) {
    update_attached_views(state);
    state.host.set_needs_layout();
    crate::measure_memo::invalidate();
    view::invalidate_layout(&state.host);
    crate::measure_memo::invalidate();
}

/// `reloadContent` (`AppKit`): refresh the column titles, recompute row
/// heights and column widths, reload, and propagate the invalidation
/// upward.
#[cfg(target_os = "macos")]
fn reload_content(state: &mut TableState) {
    for column in &state.columns {
        column
            .ns_column
            .setTitle(&objc2_foundation::NSString::from_str(
                &column.label_content.snapshot().to_plain(),
            ));
    }
    state.row_heights = row_heights(state);
    for (column, width) in state.columns.iter().zip(column_widths(state)) {
        column.ns_column.setWidth(width);
    }
    state.table.reload_data();
    TableView::refresh_header(&state.header);
    state.host.set_needs_layout();
    crate::measure_memo::invalidate();
    view::invalidate_layout(&state.host);
    crate::measure_memo::invalidate();
}

/// `WuiStableViewCollection`'s reconcile for one column's rows: reuse the
/// rendered cell of every unchanged id, render only the joins, drop the
/// leaves that left.
fn sync_column_cells(
    column: &mut ColumnState,
    renderer: &Renderer,
    host: &cocoa_ui::PlatformView,
    ids: Vec<ItemId>,
) {
    #[cfg(target_os = "macos")]
    let _ = host;
    for (index, &id) in ids.iter().enumerate() {
        if column.cells.contains_key(&id) {
            continue;
        }
        let text = column
            .snapshot
            .get_view(index)
            .expect("table row index is in bounds");
        let leaf = renderer.render(AnyView::new(text));
        #[cfg(target_os = "ios")]
        let cell = {
            let mounted = leaf.mount(host);
            view::set_translates_autoresizing(mounted.view(), true);
            mounted
        };
        #[cfg(target_os = "macos")]
        let cell = leaf;
        column.cells.insert(id, cell);
    }
    let dropped: Vec<ItemId> = column
        .cells
        .keys()
        .copied()
        .filter(|id| !ids.contains(id))
        .collect();
    for id in dropped {
        if let Some(cell) = column.cells.remove(&id) {
            // `AppKit`: the leaf is unmounted, so detach its view by hand —
            // `Mounted`'s drop does this on `UIKit`.
            #[cfg(target_os = "macos")]
            view::remove_from_superview(cell.view());
            drop(cell);
        }
    }
    column.ids = ids;
}

/// Coalesced `rows` notifications for one column — outside `TableState`,
/// so an emission landing while cell materialization holds the state
/// borrow never touches it. Each event records its own snapshot with the
/// ids captured from it and the metadata; the newest recorded triple
/// applies at the outermost transaction's finish and an older snapshot
/// is never replayed after a newer one. The cell lives exactly as long
/// as its column: a dropped column releases it, so nothing is delivered
/// to a column that no longer exists.
#[derive(Default)]
struct PendingColumnRows {
    /// The newest unapplied (snapshot, ids, metadata) triple.
    emission: Option<(AnyViewsSnapshot<Text>, Vec<ItemId>, Metadata)>,
}

impl PendingColumnRows {
    /// The newest emission is authoritative.
    fn record(&mut self, snapshot: AnyViewsSnapshot<Text>, ids: Vec<ItemId>, metadata: Metadata) {
        self.emission = Some((snapshot, ids, metadata));
    }

    /// Drains the pending emission; the apply owns delivery from here.
    const fn take(&mut self) -> Option<(AnyViewsSnapshot<Text>, Vec<ItemId>, Metadata)> {
        self.emission.take()
    }

    /// Returns an emission the apply could not deliver: anything recorded
    /// since the take is newer and stays authoritative.
    fn restore(&mut self, emission: (AnyViewsSnapshot<Text>, Vec<ItemId>, Metadata)) {
        if self.emission.is_none() {
            self.emission = Some(emission);
        }
    }
}

/// The table-owned delivery coordinator: the transaction depth shared by
/// every scope that holds the state borrow across `get_view`/render/
/// mount, plus the non-rows notifications — a pending `columns`
/// reconcile (payload + metadata, newest wins) and a `reloadContent`
/// request from a column label's `content` change.
#[derive(Default)]
struct TablePending {
    /// Active transaction depth — a notification raised inside one only
    /// records and the outermost finish delivers.
    depth: usize,
    /// The newest unapplied columns emission.
    columns: Option<(Vec<TableColumn>, Metadata)>,
    /// A label `content` change asked for `reloadContent`.
    reload: bool,
}

/// The table drain — the outermost transaction's finish and
/// `reconcile_columns`' attach boundary share it. One pass applies the
/// recorded columns reconcile (new columns' recorded rows deliver in the
/// same drain), then every attached column's rows emission, then a
/// recorded `reloadContent`; it loops until a whole pass records nothing
/// — a work queue drained to quiescence, not a poll.
fn drain_table(state: &Rc<RefCell<TableState>>, pending: &Rc<RefCell<TablePending>>) {
    pending.borrow_mut().depth += 1;
    loop {
        // `take` outside the `if let` scrutinee — the scrutinee would
        // hold the `RefMut` across the reconcile, and a reentrant
        // `record`/watch callback there would collide with it.
        let columns_event = pending.borrow_mut().columns.take();
        if let Some((columns, metadata)) = columns_event {
            with_platform_animation(&metadata, {
                let state = Rc::clone(state);
                let pending = Rc::clone(pending);
                move || reconcile_columns(&state, &pending, columns)
            });
            continue;
        }
        let mut progressed = false;
        let attached: Vec<(usize, Rc<RefCell<PendingColumnRows>>)> = state
            .borrow()
            .columns
            .iter()
            .map(|column| (column.semantic_id, Rc::clone(&column.pending)))
            .collect();
        for (semantic_id, column_pending) in attached {
            if column_pending.borrow().emission.is_some() {
                progressed = true;
                deliver_column_rows(state, &column_pending, semantic_id);
            }
        }
        if pending.borrow_mut().reload {
            pending.borrow_mut().reload = false;
            progressed = true;
            #[cfg(target_os = "ios")]
            reload_content(&state.borrow());
            #[cfg(target_os = "macos")]
            reload_content(&mut state.borrow_mut());
        }
        if !progressed {
            break;
        }
    }
    pending.borrow_mut().depth -= 1;
}

/// Delivers recorded rows changes for `semantic_id`'s column until the
/// newest has applied: each snapshot and its ids swap in atomically
/// under one borrow inside the emission's animation. Only called inside
/// `drain_table`, which marks the transaction active — a synchronous
/// emission raised by cell materialization only records, and this loop
/// drains it. Every scope that can run view generation or mounting
/// under the state borrow is a transaction, so the borrow is always
/// free here; the `borrow_mut` asserts it. An emission whose column is
/// not yet attached stays recorded and is delivered when the reconcile
/// installs it.
fn deliver_column_rows(
    state: &Rc<RefCell<TableState>>,
    pending: &Rc<RefCell<PendingColumnRows>>,
    semantic_id: usize,
) {
    loop {
        // `take` inside the loop body, not a `while let` scrutinee — the
        // scrutinee would hold the `RefMut` across the apply, and a
        // reentrant `record` there would collide with it.
        let Some((snapshot, ids, metadata)) = pending.borrow_mut().take() else {
            break;
        };
        let applied = Rc::new(Cell::new(false));
        with_platform_animation(&metadata, {
            let state = Rc::clone(state);
            let snapshot = snapshot.clone();
            let ids = ids.clone();
            let applied = Rc::clone(&applied);
            move || {
                let mut state = state.borrow_mut();
                let Some(index) = state
                    .columns
                    .iter()
                    .position(|column| column.semantic_id == semantic_id)
                else {
                    return;
                };
                let (renderer, host) = (state.renderer.clone(), state.host.clone());
                state.columns[index].snapshot = snapshot;
                sync_column_cells(&mut state.columns[index], &renderer, &host, ids);
                #[cfg(target_os = "ios")]
                reload_content(&state);
                #[cfg(target_os = "macos")]
                reload_content(&mut state);
                applied.set(true);
            }
        });
        if !applied.get() {
            // The column is not attached yet — the emission stays
            // recorded and `reconcile_columns`' install boundary
            // delivers it; if the column never attaches, it dies with
            // its owner.
            pending.borrow_mut().restore((snapshot, ids, metadata));
            break;
        }
    }
}

/// Runs `body` as a table transaction: notifications raised inside it
/// only record, and the outermost finish drains everything recorded.
/// Every scope that holds the state borrow across `get_view`/render/
/// mount goes through here.
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

/// `WuiTableColumnNode.init` — build a column: render the label, take the
/// initial row ids, and subscribe the row and label watches that drive
/// `reloadContent` (row changes additionally animate through
/// `withPlatformAnimation`).
fn materialize_column(
    state: &Rc<RefCell<TableState>>,
    pending: &Rc<RefCell<TablePending>>,
    column: &TableColumn,
) -> ColumnState {
    let (renderer, host, env) = {
        let state = state.borrow();
        (
            state.renderer.clone(),
            state.host.clone(),
            state.renderer.context().env().clone(),
        )
    };
    let rows = column.rows();

    // The label's `content` drives `reloadContent` (`onContentChange`) and,
    // on `AppKit`, the `NSTableColumn` title. The rest of the resolved
    // `TextConfig` — `paragraph_alignment`, `line_limit` — is consumed and
    // released here, as `WuiTableColumnNode` releases the alignment signal.
    let label_config = column.label().resolve(&env);
    let label_content = label_config.content;

    // A label `content` event only records the reload request while a
    // transaction is active; otherwise it drains immediately — the watch
    // never borrows the state itself.
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
    let label = {
        let leaf = renderer.render(AnyView::new(column.label()));
        let mounted = leaf.mount(&host);
        view::set_translates_autoresizing(mounted.view(), true);
        mounted
    };

    #[cfg(target_os = "macos")]
    let ns_column = {
        let id = column.semantic_id();
        let ns_column = NSTableColumn::initWithIdentifier(
            state
                .borrow()
                .renderer
                .context()
                .mtm()
                .alloc::<NSTableColumn>(),
            &objc2_foundation::NSString::from_str(&format!("waterui.table.{id}")),
        );
        ns_column.setMinWidth(MINIMUM_COLUMN_WIDTH);
        ns_column.setTitle(&objc2_foundation::NSString::from_str(
            &label_content.snapshot().to_plain(),
        ));
        ns_column
    };

    let semantic_id = column.semantic_id();
    let column_pending = Rc::new(RefCell::new(PendingColumnRows::default()));
    // A rows event only records into this column's pending cell while a
    // transaction is active; otherwise it drains immediately, so an
    // emission inside a cell's own materialization can never collide
    // with the borrow that materialization holds. An emission recorded
    // before this column is attached is delivered at
    // `reconcile_columns`' install boundary.
    let weak = Rc::downgrade(state);
    let rows_guard = rows.watch(.., {
        let column_pending = Rc::clone(&column_pending);
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
            column_pending.borrow_mut().record(snapshot, ids, metadata);
            if pending.borrow().depth == 0 {
                drain_table(&state, &pending);
            }
        }
    });

    let mut column_state = ColumnState {
        semantic_id,
        snapshot: rows.snapshot(),
        ids: Vec::new(),
        pending: column_pending,
        cells: HashMap::new(),
        #[cfg(target_os = "ios")]
        label,
        #[cfg(target_os = "macos")]
        label_content,
        #[cfg(target_os = "ios")]
        _label_content: label_content,
        _label_guard: label_guard,
        _rows_guard: rows_guard,
        #[cfg(target_os = "macos")]
        ns_column,
    };
    let ids: Vec<ItemId> = column_state
        .snapshot
        .range()
        .filter_map(|index| column_state.snapshot.get_id(index))
        .collect();
    sync_column_cells(&mut column_state, &renderer, &host, ids);
    column_state
}

/// `reconcileColumns` — the `semantic_id`-keyed reconcile: keep surviving
/// columns (their cells, watches, and platform objects untouched), build
/// the joins, drop the leaves that left. Always runs inside a
/// transaction (its callers `drain_table` and the initial population
/// provide it), so rows emissions raised while materializing a column's
/// cells only record; after `columns` is installed, the drain delivers
/// each attached column's recorded emission — the attach boundary.
fn reconcile_columns(
    state: &Rc<RefCell<TableState>>,
    pending: &Rc<RefCell<TablePending>>,
    columns: Vec<TableColumn>,
) {
    let mut kept = HashMap::new();
    for existing in core::mem::take(&mut state.borrow_mut().columns) {
        kept.insert(existing.semantic_id, existing);
    }
    let mut ordered = Vec::with_capacity(columns.len());
    for column in columns {
        let id = column.semantic_id();
        ordered.push(
            kept.remove(&id)
                .unwrap_or_else(|| materialize_column(state, pending, &column)),
        );
    }
    // `kept`'s leftovers are the departed columns; dropping them
    // releases their cells and unsubscribes their watchers.
    drop(kept);
    state.borrow_mut().columns = ordered;
    #[cfg(target_os = "ios")]
    update_columns(&state.borrow());
    #[cfg(target_os = "macos")]
    update_columns(&mut state.borrow_mut());
}

/// `updateColumns` (iOS) — reconcile attached views, then reload.
#[cfg(target_os = "ios")]
fn update_columns(state: &TableState) {
    update_attached_views(state);
    reload_content(state);
}

/// `updateColumns` (`AppKit`) — push the current column list to the
/// `NSTableView`, then reload.
#[cfg(target_os = "macos")]
fn update_columns(state: &mut TableState) {
    {
        let ordered: Vec<Retained<NSTableColumn>> = state
            .columns
            .iter()
            .map(|column| column.ns_column.clone())
            .collect();
        state.table.set_columns(&ordered);
    }
    reload_content(state);
}

/// `layoutSubviews` on `UIKit` — frame labels across the top band and cells
/// on their row/column grid.
#[cfg(target_os = "ios")]
fn layout_children(state: &TableState) {
    let widths = column_widths(state);
    let header_height = header_height(state);
    let row_heights = row_heights(state);
    let mut x = 0.0;
    for (index, column) in state.columns.iter().enumerate() {
        view::set_frame(
            column.label.view(),
            cocoa_ui::Rect::new(x, 0.0, widths[index], header_height),
        );
        x += widths[index];
    }
    let mut y = header_height;
    for (row, row_height) in row_heights.iter().enumerate() {
        x = 0.0;
        for (index, column) in state.columns.iter().enumerate() {
            if let Some(cell) = column.ids.get(row).and_then(|id| column.cells.get(id)) {
                view::set_frame(
                    cell.view(),
                    cocoa_ui::Rect::new(
                        x + HORIZONTAL_PADDING,
                        y + VERTICAL_PADDING,
                        HORIZONTAL_PADDING.mul_add(-2.0, widths[index]),
                        VERTICAL_PADDING.mul_add(-2.0, *row_height),
                    ),
                );
            }
            x += widths[index];
        }
        y += row_height;
    }
}

/// `layout` on `AppKit` — refresh the column widths, then frame the header
/// band and the table inside the host's bounds.
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

/// The table's layout face: `sizeThatFits` — the proposal is ignored.
struct TableSubView {
    state: Rc<RefCell<TableState>>,
}

impl core::fmt::Debug for TableSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TableSubView").finish_non_exhaustive()
    }
}

impl SubView for TableSubView {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(size_that_fits(&self.state.borrow()))
    }

    /// `NSTableView`-driven stretch: `.none`.
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The `NSTableView` data handlers (macOS) — row count and heights read
/// the applied `column.ids`, and `cell_view` returns the already
/// materialized leaf for a snapshot position.
#[cfg(target_os = "macos")]
fn install_table_handlers(state: &Rc<RefCell<TableState>>) {
    let table = &state.borrow().table;
    table.set_row_count_handler({
        let weak = Rc::downgrade(state);
        move || {
            weak.upgrade().map_or(0, |state| {
                state
                    .borrow()
                    .columns
                    .iter()
                    .map(|column| column.ids.len())
                    .max()
                    .unwrap_or(0)
            })
        }
    });
    table.set_row_height_handler({
        let weak = Rc::downgrade(state);
        move |row| {
            weak.upgrade()
                .map_or(0.0, |state| state.borrow().row_heights[row])
        }
    });
    table.set_cell_view_handler({
        let weak = Rc::downgrade(state);
        move |ns_column, row| {
            let state = weak.upgrade()?;
            let state = state.borrow();
            let column = state.columns.iter().find(|column| {
                std::ptr::eq(
                    std::ptr::from_ref(&*column.ns_column),
                    std::ptr::from_ref(ns_column),
                )
            })?;
            let id = column.ids.get(row)?;
            let cell = column.cells.get(id)?;
            Some(view::retain_base(cell.view()))
        }
    });
}

/// Renders a `TableConfig` into the host: `WuiTable.init(columns:env:)`.
fn render(config: TableConfig, ctx: &RenderContext<'_>) -> NativeLeaf {
    let TableConfig { columns } = config;
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

    let state = Rc::new(RefCell::new(TableState {
        host: host.clone(),
        #[cfg(target_os = "macos")]
        table,
        #[cfg(target_os = "macos")]
        header,
        renderer: ctx.renderer(),
        columns: Vec::new(),
        #[cfg(target_os = "macos")]
        row_heights: Vec::new(),
    }));

    // The table delivery coordinator — every scope that borrows the
    // state across `get_view`/render/mount enters a transaction, so a
    // notification raised inside only records and the outermost finish
    // drains all of them.
    let pending = Rc::new(RefCell::new(TablePending::default()));
    host.set_measure_handler({
        let state = Rc::clone(&state);
        let pending = Rc::clone(&pending);
        move |_host, _proposal| {
            let size = with_table_tx(&state, &pending, || size_that_fits(&state.borrow()));
            cocoa_ui::Size::new(f64::from(size.width), f64::from(size.height))
        }
    });
    host.set_layout_handler({
        let state = Rc::clone(&state);
        let pending = Rc::clone(&pending);
        move |_host| with_table_tx(&state, &pending, || layout_children(&state.borrow()))
    });

    #[cfg(target_os = "macos")]
    install_table_handlers(&state);

    // `watchAnyViewsIds` — the columns watch. An event only records the
    // newest payload and metadata while a transaction is active;
    // otherwise it drains immediately — the reconcile itself applies at
    // the outermost finish inside the emission's animation.
    let columns_guard = columns.watch({
        let weak = Rc::downgrade(&state);
        let pending = Rc::clone(&pending);
        move |ctx| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let metadata = ctx.metadata().clone();
            let columns = ctx.into_value();
            pending.borrow_mut().columns = Some((columns, metadata));
            if pending.borrow().depth == 0 {
                drain_table(&state, &pending);
            }
        }
    });

    // `reconcileColumns(ids: source.allIds())` — the initial population,
    // a transaction since materializing columns may emit back.
    with_table_tx(&state, &pending, || {
        reconcile_columns(&state, &pending, columns.snapshot());
    });

    let mut leaf = NativeLeaf::new(
        &*host,
        TableSubView {
            state: Rc::clone(&state),
        },
    );
    leaf.keep(columns_guard);
    leaf.keep(state);
    leaf
}

/// Registers the `table` leaf.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<TableConfig>(render);
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
