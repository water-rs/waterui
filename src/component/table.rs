//! Provides components for creating and displaying data in a tabular format.
//!
//! A [`Table`] is row-major over an identifiable [`Collection`], paired with a
//! tuple (or slice/vector) of [`TableColumn`] / [`SortableTableColumn`]
//! definitions whose cell builders produce arbitrary [`View`]s. Rows carry the
//! collection's item identity, so [`.selection`](Table::selection) and
//! [`.multi_selection`](Table::multi_selection) bind directly to item ids, and
//! [`.sort_order`](Table::sort_order) binds to the active sortable column's key
//! and [`SortDirection`].
//!
//! # Example
//!
//! ```no_run
//! use waterui::prelude::*;
//!
//! #[derive(Clone)]
//! struct Person {
//!     id: u64,
//!     name: &'static str,
//!     email: &'static str,
//! }
//!
//! impl Identifiable for Person {
//!     type Id = u64;
//!     fn id(&self) -> Self::Id {
//!         self.id
//!     }
//! }
//!
//! #[derive(Clone, Copy, PartialEq, Eq)]
//! enum PersonSort {
//!     Name,
//! }
//!
//! fn people_table(
//!     people: StateList<Person>,
//!     selection: Binding<Option<u64>>,
//!     sort_order: Binding<Option<TableSort<PersonSort>>>,
//! ) -> impl View {
//!     Table::new(
//!         people,
//!         (
//!             TableColumn::new("Name", |p: &Person| text(p.name)).sortable(PersonSort::Name),
//!             TableColumn::new("Email", |p: &Person| text(p.email)),
//!         ),
//!     )
//!     .selection(&selection)
//!     .sort_order(&sort_order)
//! }
//! ```

use alloc::collections::BTreeSet;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::type_name;
use core::fmt;
use core::marker::PhantomData;

use nami::collection::Collection;
use nami::{Binding, Signal as _, SignalExt as _};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::id::{Id as RawId, Mapping};
use waterui_core::views::{ForEach, SharedAnyViews};
use waterui_core::{AnyView, Environment, Native, NativeView, OneOf2, View};
use waterui_text::{IntoText, Text};

pub use crate::component::selection_support::SelectionThemed;
use crate::component::selection_support::WithId;
use crate::contract_violation;

/// Direction of an active table column sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "sort order has exactly two directions; backends and callers match exhaustively"
)]
pub enum SortDirection {
    /// Ascending order (smallest to largest, A to Z).
    Ascending,
    /// Descending order (largest to smallest, Z to A).
    Descending,
}

impl SortDirection {
    /// Returns the opposite sort direction.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// Active sort state for a [`Table`]: which column is sorted and in which
/// [`SortDirection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableSort<K> {
    /// The key identifying the active sorted column.
    pub column: K,
    /// The direction the column is sorted in.
    pub direction: SortDirection,
}

impl<K> TableSort<K> {
    /// Creates a sort order for `column` in `direction`.
    #[must_use]
    pub const fn new(column: K, direction: SortDirection) -> Self {
        Self { column, direction }
    }

    /// Creates an ascending sort order for `column`.
    #[must_use]
    pub const fn ascending(column: K) -> Self {
        Self::new(column, SortDirection::Ascending)
    }

    /// Creates a descending sort order for `column`.
    #[must_use]
    pub const fn descending(column: K) -> Self {
        Self::new(column, SortDirection::Descending)
    }

    /// Returns this sort order with its direction inverted.
    #[must_use]
    pub fn toggled(self) -> Self {
        Self {
            column: self.column,
            direction: self.direction.toggled(),
        }
    }

    /// Maps the column key through `f`, preserving [`SortDirection`].
    #[must_use]
    pub fn map_column<U>(self, f: impl FnOnce(K) -> U) -> TableSort<U> {
        TableSort {
            column: f(self.column),
            direction: self.direction,
        }
    }
}

impl<K: PartialEq> TableSort<K> {
    /// Computes the next sort state after activating `column`: toggles
    /// direction if `column` is already active, otherwise starts ascending.
    #[must_use]
    pub fn next_for_column(current: Option<Self>, column: K) -> Self {
        match current {
            Some(active) if active.column == column => active.toggled(),
            _ => Self::ascending(column),
        }
    }
}

/// Selection a [`Table`] binds to its rows' identities: none, single row, or
/// multiple rows.
///
/// In [`TableConfig`], `Id` is the erased [`RawId`] reported by
/// [`TableConfig::rows`].
#[derive(Clone, Default)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each selection mode variant by variant; a new selection mode must fail their build instead of falling into a wildcard arm"
)]
pub enum TableSelection<Id: 'static> {
    /// Rows are not selectable.
    #[default]
    None,
    /// A single selected row id — `None` inside the binding means nothing is
    /// selected.
    Single(Binding<Option<Id>>),
    /// A set of selected row ids.
    Multiple(Binding<BTreeSet<Id>>),
}

impl<Id: 'static> fmt::Debug for TableSelection<Id> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

fn report_foreign_row_id() {
    contract_violation!(
        "a table selection id was not taken from the table's rows, so the write ignores it"
    );
}

impl TableSelection<RawId> {
    fn single<Id: 'static + Ord + Clone>(
        selection: &Binding<Option<Id>>,
        ids: &Mapping<Id>,
    ) -> Self {
        Self::Single(Binding::mapping(
            selection,
            {
                let to = ids.clone();
                move |selected| selected.map(|id| to.to_id(id))
            },
            {
                let from = ids.clone();
                move |binding, erased: Option<RawId>| {
                    let selected = match erased {
                        Some(raw) => {
                            let Some(id) = from.to_data(raw) else {
                                report_foreign_row_id();
                                return;
                            };
                            Some(id)
                        }
                        None => None,
                    };
                    binding.set(selected);
                }
            },
        ))
    }

    fn multiple<Id: 'static + Ord + Clone>(
        selection: &Binding<BTreeSet<Id>>,
        ids: &Mapping<Id>,
    ) -> Self {
        let to = ids.clone();
        let from = ids.clone();
        Self::Multiple(Binding::mapping(
            selection,
            move |selected| selected.into_iter().map(|id| to.to_id(id)).collect(),
            move |binding, erased: BTreeSet<RawId>| {
                binding.set(
                    erased
                        .into_iter()
                        .filter_map(|raw| {
                            let id = from.to_data(raw);
                            if id.is_none() {
                                report_foreign_row_id();
                            }
                            id
                        })
                        .collect(),
                );
            },
        ))
    }
}

/// Backend descriptor for a single table column: header label, sort affordance,
/// and stable identity.
#[derive(Clone)]
pub struct TableColumnDescriptor {
    label: Text,
    sortable: bool,
    identity: Rc<()>,
}

impl fmt::Debug for TableColumnDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableColumnDescriptor")
            .field("sortable", &self.sortable)
            .field("identity", &self.identity())
            .finish_non_exhaustive()
    }
}

impl TableColumnDescriptor {
    /// Creates a new column descriptor.
    #[must_use]
    pub(crate) const fn new(label: Text, sortable: bool, identity: Rc<()>) -> Self {
        Self {
            label,
            sortable,
            identity,
        }
    }

    /// Returns the column's header label.
    #[must_use]
    pub const fn label(&self) -> &Text {
        &self.label
    }

    /// Whether the column supports interactive sorting.
    #[must_use]
    pub const fn is_sortable(&self) -> bool {
        self.sortable
    }

    /// Stable identity for this column descriptor.
    #[must_use]
    pub fn identity(&self) -> usize {
        Rc::as_ptr(&self.identity) as usize
    }

    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        self.label = Text::from(self.label.resolve(env));
        self
    }
}

/// One row of a [`Table`], carrying one cell per column in column order.
pub struct TableRow<C> {
    cells: Vec<C>,
}

impl<C> fmt::Debug for TableRow<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableRow")
            .field("cells", &self.cells.len())
            .finish()
    }
}

impl<C> TableRow<C> {
    /// Creates a table row from its ordered cells.
    #[must_use]
    pub(crate) const fn new(cells: Vec<C>) -> Self {
        Self { cells }
    }

    /// Returns the row's cells in column order.
    #[must_use]
    pub fn cells(&self) -> &[C] {
        &self.cells
    }

    /// Consumes the row and returns its cells in column order.
    #[must_use]
    pub fn into_cells(self) -> Vec<C> {
        self.cells
    }

    /// Transforms every cell in this row through `f`.
    #[must_use]
    pub fn map_cells<T>(self, f: impl Fn(C) -> T) -> TableRow<T> {
        TableRow {
            cells: self.cells.into_iter().map(f).collect(),
        }
    }
}

impl<C: View> View for TableRow<C> {
    fn body(self, _env: &Environment) -> impl View {
        let _ = self;
    }
}

/// Configuration payload delivered to backends for a [`Table`].
///
/// `C` is the cell view type up to dispatch, then the backend's child slot.
#[non_exhaustive]
pub struct TableConfig<C> {
    columns: Vec<TableColumnDescriptor>,
    rows: SharedAnyViews<TableRow<C>>,
    selection: TableSelection<RawId>,
    sort_order: Option<Binding<Option<TableSort<usize>>>>,
}

impl<C> fmt::Debug for TableConfig<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableConfig")
            .field("columns", &self.columns)
            .field("selection", &self.selection)
            .field("sortable", &self.sort_order.is_some())
            .finish_non_exhaustive()
    }
}

impl<C> TableConfig<C> {
    /// Returns the ordered column descriptors for this table.
    #[must_use]
    pub fn columns(&self) -> &[TableColumnDescriptor] {
        &self.columns
    }

    /// Returns the reactive row collection; each row yields its cell views in
    /// column order and carries the item's erased [`RawId`].
    #[must_use]
    pub const fn rows(&self) -> &SharedAnyViews<TableRow<C>> {
        &self.rows
    }

    /// Returns the table's row selection binding, keyed by the [`RawId`]s
    /// reported by [`TableConfig::rows`].
    #[must_use]
    pub const fn selection(&self) -> &TableSelection<RawId> {
        &self.selection
    }

    /// Returns the active column sort binding, indexed into
    /// [`TableConfig::columns`], when sorting is bound.
    #[must_use]
    pub const fn sort_order(&self) -> Option<&Binding<Option<TableSort<usize>>>> {
        self.sort_order.as_ref()
    }

    /// Whether column `column_index` is interactive for sorting: both the
    /// column itself was marked sortable and a sort-order binding is attached.
    #[must_use]
    pub fn column_is_sortable(&self, column_index: usize) -> bool {
        self.sort_order.is_some()
            && self
                .columns
                .get(column_index)
                .is_some_and(TableColumnDescriptor::is_sortable)
    }

    /// Toggles sorting on `column_index` if it is sortable and a sort-order
    /// binding is attached.
    pub fn toggle_sort(&self, column_index: usize) {
        if !self.column_is_sortable(column_index) {
            return;
        }
        if let Some(binding) = &self.sort_order {
            let next = TableSort::next_for_column(binding.snapshot(), column_index);
            binding.set(Some(next));
        }
    }

    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        self.columns = self
            .columns
            .into_iter()
            .map(|column| column.resolve(env))
            .collect();
        self
    }
}

impl<C: 'static> NativeView for TableConfig<C> {}

impl<C: 'static> MapSlots<C> for TableConfig<C> {
    type With<T: 'static> = TableConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(C) -> T + Clone + 'static) -> TableConfig<T> {
        TableConfig {
            columns: self.columns,
            rows: self
                .rows
                .map_views(move |row: TableRow<C>| row.map_cells(&f)),
            selection: self.selection,
            sort_order: self.sort_order,
        }
    }
}

/// A non-sortable column definition for a [`Table`], building a cell view from
/// each row item.
pub struct TableColumn<Item, F, V> {
    label: Text,
    cell: F,
    identity: Rc<()>,
    _marker: PhantomData<fn(&Item) -> V>,
}

impl<Item, F, V> fmt::Debug for TableColumn<Item, F, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableColumn")
            .field("identity", &(Rc::as_ptr(&self.identity) as usize))
            .finish_non_exhaustive()
    }
}

impl<Item, F, V> TableColumn<Item, F, V>
where
    F: Fn(&Item) -> V + 'static,
    V: View,
{
    /// Creates a new table column with header `label` and row cell builder
    /// `cell`.
    pub fn new(label: impl IntoText, cell: F) -> Self {
        Self {
            label: label.into_text(),
            cell,
            identity: Rc::new(()),
            _marker: PhantomData,
        }
    }

    /// Marks this column as sortable, identifying it with `sort_key` in the
    /// table's [`TableSort`] binding.
    #[must_use]
    pub fn sortable<K: 'static + Eq + Clone>(
        self,
        sort_key: K,
    ) -> SortableTableColumn<Item, F, V, K> {
        SortableTableColumn {
            label: self.label,
            cell: self.cell,
            sort_key,
            identity: self.identity,
            _marker: PhantomData,
        }
    }
}

/// A sortable column definition produced by [`TableColumn::sortable`].
pub struct SortableTableColumn<Item, F, V, K> {
    label: Text,
    cell: F,
    sort_key: K,
    identity: Rc<()>,
    _marker: PhantomData<fn(&Item) -> V>,
}

impl<Item, F, V, K: fmt::Debug> fmt::Debug for SortableTableColumn<Item, F, V, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SortableTableColumn")
            .field("sort_key", &self.sort_key)
            .field("identity", &(Rc::as_ptr(&self.identity) as usize))
            .finish_non_exhaustive()
    }
}

/// Column set accepted by [`Table::new`].
///
/// Implemented for a single [`TableColumn`] or [`SortableTableColumn`], tuples
/// of up to 16 heterogeneous columns (combining their cell types statically via
/// [`OneOf2`] without type erasure), as well as `Vec<C>` and `[C; N]`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid set of table columns for `{Item}`",
    label = "expected a `TableColumn`, `SortableTableColumn`, or tuple of them",
    note = "build columns with `TableColumn::new(\"Title\", |item: &Item| ...)` and pass them as a tuple `(col1, col2)`"
)]
pub trait TableColumns<Item>: 'static {
    /// Unified cell view type produced across all columns in this set.
    type Cell: View;

    /// Appends this set's column descriptors in display order.
    fn push_descriptors(&self, out: &mut Vec<TableColumnDescriptor>);

    /// Appends this set's cell views for `item` in display order, wrapping each
    /// cell through `wrap`.
    fn push_cells<Out>(&self, item: &Item, out: &mut Vec<Out>, wrap: &impl Fn(Self::Cell) -> Out);
}

impl<Item: 'static, F, V> TableColumns<Item> for TableColumn<Item, F, V>
where
    F: Fn(&Item) -> V + 'static,
    V: View,
{
    type Cell = V;

    fn push_descriptors(&self, out: &mut Vec<TableColumnDescriptor>) {
        out.push(TableColumnDescriptor::new(
            self.label.clone(),
            false,
            Rc::clone(&self.identity),
        ));
    }

    fn push_cells<Out>(&self, item: &Item, out: &mut Vec<Out>, wrap: &impl Fn(Self::Cell) -> Out) {
        out.push(wrap((self.cell)(item)));
    }
}

impl<Item: 'static, F, V, K: 'static> TableColumns<Item> for SortableTableColumn<Item, F, V, K>
where
    F: Fn(&Item) -> V + 'static,
    V: View,
{
    type Cell = V;

    fn push_descriptors(&self, out: &mut Vec<TableColumnDescriptor>) {
        out.push(TableColumnDescriptor::new(
            self.label.clone(),
            true,
            Rc::clone(&self.identity),
        ));
    }

    fn push_cells<Out>(&self, item: &Item, out: &mut Vec<Out>, wrap: &impl Fn(Self::Cell) -> Out) {
        out.push(wrap((self.cell)(item)));
    }
}

impl<Item, C> TableColumns<Item> for Vec<C>
where
    C: TableColumns<Item>,
{
    type Cell = C::Cell;

    fn push_descriptors(&self, out: &mut Vec<TableColumnDescriptor>) {
        for column in self {
            column.push_descriptors(out);
        }
    }

    fn push_cells<Out>(&self, item: &Item, out: &mut Vec<Out>, wrap: &impl Fn(Self::Cell) -> Out) {
        for column in self {
            column.push_cells(item, out, wrap);
        }
    }
}

impl<Item, C, const N: usize> TableColumns<Item> for [C; N]
where
    C: TableColumns<Item>,
{
    type Cell = C::Cell;

    fn push_descriptors(&self, out: &mut Vec<TableColumnDescriptor>) {
        for column in self {
            column.push_descriptors(out);
        }
    }

    fn push_cells<Out>(&self, item: &Item, out: &mut Vec<Out>, wrap: &impl Fn(Self::Cell) -> Out) {
        for column in self {
            column.push_cells(item, out, wrap);
        }
    }
}

/// Type-level flag tracking whether a column set contains at least one column
/// sortable by key `K`.
pub trait SortPresence {
    /// Logical OR of two sort presences.
    type Or<Other: SortPresence>: SortPresence;
}

/// Marker indicating no column in the set is sortable.
#[derive(Debug, Clone, Copy)]
pub struct Unsortable;

/// Marker indicating at least one column in the set is sortable.
#[derive(Debug, Clone, Copy)]
pub struct Sortable;

impl SortPresence for Unsortable {
    type Or<Other: SortPresence> = Other;
}

impl SortPresence for Sortable {
    type Or<Other: SortPresence> = Self;
}

/// Columns whose sortable entries all use sort key `K`.
#[diagnostic::on_unimplemented(
    message = "the columns of `{Self}` cannot be sorted by `{K}`",
    label = "incompatible sort key `{K}`",
    note = "every sortable column in a table must use the same sort key type passed to `.sort_order(&binding)`"
)]
pub trait SortableColumns<K> {
    /// Whether this column set contains at least one sortable column.
    type Presence: SortPresence;

    /// Appends `Some(key)` for each sortable column and `None` for each
    /// non-sortable column, in display order.
    fn push_sort_keys(&self, out: &mut Vec<Option<K>>);
}

impl<Item, F, V, K> SortableColumns<K> for TableColumn<Item, F, V> {
    type Presence = Unsortable;

    fn push_sort_keys(&self, out: &mut Vec<Option<K>>) {
        out.push(None);
    }
}

impl<Item, F, V, K: Clone> SortableColumns<K> for SortableTableColumn<Item, F, V, K> {
    type Presence = Sortable;

    fn push_sort_keys(&self, out: &mut Vec<Option<K>>) {
        out.push(Some(self.sort_key.clone()));
    }
}

impl<C, K> SortableColumns<K> for Vec<C>
where
    C: SortableColumns<K>,
{
    type Presence = C::Presence;

    fn push_sort_keys(&self, out: &mut Vec<Option<K>>) {
        for column in self {
            column.push_sort_keys(out);
        }
    }
}

impl<C, K, const N: usize> SortableColumns<K> for [C; N]
where
    C: SortableColumns<K>,
{
    type Presence = C::Presence;

    fn push_sort_keys(&self, out: &mut Vec<Option<K>>) {
        for column in self {
            column.push_sort_keys(out);
        }
    }
}

/// Proof that a column set has at least one column sortable by `K`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no column sortable by `{K}`",
    label = "no column sortable by `{K}`",
    note = "mark at least one column with `.sortable(key)` matching the key type of `.sort_order(&binding)`"
)]
pub trait HasSortableColumn<K>: SortableColumns<K> {}

impl<T, K> HasSortableColumn<K> for T where T: SortableColumns<K, Presence = Sortable> {}

macro_rules! any_sortable {
    ($K:ident; $T:ident) => {
        <$T as SortableColumns<$K>>::Presence
    };
    ($K:ident; $T:ident, $($rest:ident),+) => {
        <<$T as SortableColumns<$K>>::Presence as SortPresence>::Or<any_sortable!($K; $($rest),+)>
    };
}

macro_rules! impl_table_columns_tuple {
    ($T:ident) => {
        impl<Item, $T: TableColumns<Item>> TableColumns<Item> for ($T,) {
            type Cell = $T::Cell;

            fn push_descriptors(&self, out: &mut Vec<TableColumnDescriptor>) {
                self.0.push_descriptors(out);
            }

            fn push_cells<Out>(
                &self,
                item: &Item,
                out: &mut Vec<Out>,
                wrap: &impl Fn(Self::Cell) -> Out,
            ) {
                self.0.push_cells(item, out, wrap);
            }
        }

        impl<K, $T: SortableColumns<K>> SortableColumns<K> for ($T,) {
            type Presence = $T::Presence;

            fn push_sort_keys(&self, out: &mut Vec<Option<K>>) {
                self.0.push_sort_keys(out);
            }
        }
    };
    ($First:ident, $($Rest:ident),+) => {
        impl<Item, $First: TableColumns<Item>, $($Rest: TableColumns<Item>),+> TableColumns<Item>
            for ($First, $($Rest,)+)
        {
            type Cell = OneOf2<$First::Cell, <($($Rest,)+) as TableColumns<Item>>::Cell>;

            #[expect(
                non_snake_case,
                reason = "tuple destructuring reuses type parameter identifiers"
            )]
            fn push_descriptors(&self, out: &mut Vec<TableColumnDescriptor>) {
                let ($First, $($Rest,)+) = self;
                $First.push_descriptors(out);
                $(
                    $Rest.push_descriptors(out);
                )+
            }

            #[expect(
                non_snake_case,
                reason = "tuple destructuring reuses type parameter identifiers"
            )]
            fn push_cells<Out>(
                &self,
                item: &Item,
                out: &mut Vec<Out>,
                wrap: &impl Fn(Self::Cell) -> Out,
            ) {
                let ($First, $($Rest,)+) = self;
                $First.push_cells(item, out, &|cell| wrap(OneOf2::A(cell)));
                impl_table_columns_tuple!(@tail_cells item, out, |cell| wrap(OneOf2::B(cell)); $($Rest),+);
            }
        }

        impl<K, $First: SortableColumns<K>, $($Rest: SortableColumns<K>),+> SortableColumns<K>
            for ($First, $($Rest,)+)
        {
            type Presence = any_sortable!(K; $First, $($Rest),+);

            #[expect(
                non_snake_case,
                reason = "tuple destructuring reuses type parameter identifiers"
            )]
            fn push_sort_keys(&self, out: &mut Vec<Option<K>>) {
                let ($First, $($Rest,)+) = self;
                $First.push_sort_keys(out);
                $(
                    $Rest.push_sort_keys(out);
                )+
            }
        }
    };
    (@tail_cells $item:ident, $out:ident, $wrap:expr; $Only:ident) => {
        $Only.push_cells($item, $out, &$wrap);
    };
    (@tail_cells $item:ident, $out:ident, $wrap:expr; $Head:ident, $($Tail:ident),+) => {
        $Head.push_cells($item, $out, &|cell| ($wrap)(OneOf2::A(cell)));
        impl_table_columns_tuple!(@tail_cells $item, $out, |cell| ($wrap)(OneOf2::B(cell)); $($Tail),+);
    };
}

macro_rules! table_column_tuples {
    ($macro:ident) => {
        $macro!(T0);
        $macro!(T0, T1);
        $macro!(T0, T1, T2);
        $macro!(T0, T1, T2, T3);
        $macro!(T0, T1, T2, T3, T4);
        $macro!(T0, T1, T2, T3, T4, T5);
        $macro!(T0, T1, T2, T3, T4, T5, T6);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13);
        $macro!(
            T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14
        );
        $macro!(
            T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15
        );
    };
}

table_column_tuples!(impl_table_columns_tuple);

/// A row-major tabular component over identifiable data with view-valued cells,
/// identity-based row selection, and sortable column headers.
pub struct Table<C: Collection, Cols> {
    data: C,
    columns: Cols,
    selection: TableSelection<C::Id>,
    sort_order: Option<Binding<Option<TableSort<usize>>>>,
}

impl<C: Collection, Cols> fmt::Debug for Table<C, Cols> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Table")
            .field("selection", &self.selection)
            .field("sortable", &self.sort_order.is_some())
            .finish_non_exhaustive()
    }
}

impl<C, Cols> Table<C, Cols>
where
    C: Collection + Clone + 'static,
    C::Item: Clone,
    Cols: TableColumns<C::Item>,
{
    /// Creates a row-major table displaying `data` across `columns`.
    #[must_use]
    pub const fn new(data: C, columns: Cols) -> Self {
        Self {
            data,
            columns,
            selection: TableSelection::None,
            sort_order: None,
        }
    }

    /// Binds single-row selection to `selection`, keyed by each row's
    /// [`Collection::Id`].
    #[must_use]
    pub fn selection(mut self, selection: &Binding<Option<C::Id>>) -> Self {
        self.selection = TableSelection::Single(selection.clone());
        self
    }

    /// Binds multi-row selection to `selection`, keyed by each row's
    /// [`Collection::Id`].
    #[must_use]
    pub fn multi_selection(mut self, selection: &Binding<BTreeSet<C::Id>>) -> Self {
        self.selection = TableSelection::Multiple(selection.clone());
        self
    }

    /// Binds the active column sort order to `order`.
    ///
    /// At least one column must be marked `.sortable(key)` with matching key
    /// type `K`.
    #[must_use]
    pub fn sort_order<K>(mut self, order: &Binding<Option<TableSort<K>>>) -> Self
    where
        Cols: HasSortableColumn<K>,
        K: 'static + Eq + Clone,
    {
        let mut keys = Vec::new();
        self.columns.push_sort_keys(&mut keys);
        let keys = Rc::new(keys);
        let mapped = Binding::mapping(
            order,
            {
                let keys = Rc::clone(&keys);
                move |sort: Option<TableSort<K>>| {
                    let sort = sort?;
                    let index = keys
                        .iter()
                        .position(|entry| entry.as_ref() == Some(&sort.column))?;
                    Some(TableSort::new(index, sort.direction))
                }
            },
            {
                let keys = Rc::clone(&keys);
                move |binding, erased: Option<TableSort<usize>>| {
                    let typed = match erased {
                        Some(sort) => {
                            let Some(key) = keys.get(sort.column).and_then(Option::as_ref).cloned()
                            else {
                                contract_violation!(
                                    "table sort write named non-sortable column index {}; ignoring",
                                    sort.column
                                );
                                return;
                            };
                            Some(TableSort::new(key, sort.direction))
                        }
                        None => None,
                    };
                    binding.set(typed);
                }
            },
        );
        self.sort_order = Some(mapped);
        self
    }

    /// Builds the backend [`TableConfig`] for this table.
    #[must_use]
    pub fn config(self) -> TableConfig<SelectionThemed<Cols::Cell>>
    where
        C::Id: 'static + Ord + Clone,
    {
        let Self {
            data,
            columns,
            selection,
            sort_order,
        } = self;
        let columns = Rc::new(columns);
        let mut descriptors = Vec::new();
        columns.push_descriptors(&mut descriptors);
        let column_count = descriptors.len();

        let raw_rows = ForEach::new(data, move |item: C::Item| {
            let mut cells = Vec::with_capacity(column_count);
            columns.push_cells(&item, &mut cells, &|cell| cell);
            TableRow::new(cells)
        });

        let themed_selection = selection.clone();
        let themed_rows = WithId::new(raw_rows, move |id: C::Id, row: TableRow<Cols::Cell>| {
            let selected = match &themed_selection {
                TableSelection::None => None,
                TableSelection::Single(binding) => Some(
                    binding
                        .clone()
                        .map(move |current| current.as_ref() == Some(&id))
                        .computed(),
                ),
                TableSelection::Multiple(binding) => Some(
                    binding
                        .clone()
                        .map(move |current| current.contains(&id))
                        .computed(),
                ),
            };
            row.map_cells(|content| SelectionThemed::new(content, selected.clone()))
        });

        let (rows, ids) = SharedAnyViews::new_with_ids(themed_rows);
        let erased_selection = match &selection {
            TableSelection::None => TableSelection::None,
            TableSelection::Single(binding) => TableSelection::single(binding, &ids),
            TableSelection::Multiple(binding) => TableSelection::multiple(binding, &ids),
        };

        TableConfig {
            columns: descriptors,
            rows,
            selection: erased_selection,
            sort_order,
        }
    }
}

impl<C, Cols> View for Table<C, Cols>
where
    C: Collection + Clone + 'static,
    C::Item: Clone,
    C::Id: 'static + Ord + Clone,
    Cols: TableColumns<C::Item>,
{
    fn body(self, env: &Environment) -> impl View {
        let config = self.config().resolve(env).map_slots(AnyView::new);
        Native::new(config)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let config = self.config().resolve(env).map_slots(B::Slot::from_view);
        backend.native(config, env, cx)
    }
}

/// Convenience constructor for a row-major [`Table`].
#[must_use]
pub const fn table<C, Cols>(data: C, columns: Cols) -> Table<C, Cols>
where
    C: Collection + Clone + 'static,
    C::Item: Clone,
    Cols: TableColumns<C::Item>,
{
    Table::new(data, columns)
}

/// Convenience constructor for a single [`TableColumn`].
pub fn col<Item, F, V>(label: impl IntoText, cell: F) -> TableColumn<Item, F, V>
where
    F: Fn(&Item) -> V + 'static,
    V: View,
{
    TableColumn::new(label, cell)
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeSet;
    use alloc::vec;

    use nami::collection::StateList;
    use nami::{Binding, Signal as _};
    use waterui_core::id::{Id as RawId, Identifiable};

    use super::{SortDirection, Table, TableColumn, TableSelection, TableSort, col, table};
    use crate::Color;
    use crate::component::text;

    #[derive(Clone)]
    struct Person {
        id: u64,
        name: &'static str,
        active: bool,
    }

    impl Identifiable for Person {
        type Id = u64;

        fn id(&self) -> Self::Id {
            self.id
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PersonSort {
        Name,
    }

    fn sample_people() -> StateList<Person> {
        StateList::from(vec![
            Person {
                id: 10,
                name: "Ada",
                active: true,
            },
            Person {
                id: 20,
                name: "Grace",
                active: false,
            },
        ])
    }

    #[test]
    fn row_major_table_maps_heterogeneous_cells_selection_and_sort_order() {
        let people = sample_people();
        let selection = Binding::new(Some(10_u64));
        let sort_order = Binding::new(Some(TableSort::ascending(PersonSort::Name)));

        let config = Table::new(
            people,
            (
                TableColumn::new("Name", |p: &Person| text(p.name)).sortable(PersonSort::Name),
                col("Status", |p: &Person| {
                    if p.active {
                        Color::srgb(0, 200, 80)
                    } else {
                        Color::srgb(160, 160, 160)
                    }
                }),
            ),
        )
        .selection(&selection)
        .sort_order(&sort_order)
        .config();

        assert_eq!(config.columns().len(), 2);
        assert!(config.columns()[0].is_sortable());
        assert!(!config.columns()[1].is_sortable());
        assert!(config.column_is_sortable(0));
        assert!(!config.column_is_sortable(1));

        let snapshot = config.rows().snapshot();
        assert_eq!(snapshot.len(), 2);
        let row0_id = snapshot.get_id(0).expect("row 0 id");
        let row1_id = snapshot.get_id(1).expect("row 1 id");
        assert_ne!(row0_id, row1_id);
        let row0 = snapshot.get_view(0).expect("row 0 cells");
        assert_eq!(row0.cells().len(), 2);

        let TableSelection::Single(erased_sel) = config.selection() else {
            panic!("expected single selection");
        };
        assert_eq!(erased_sel.snapshot(), Some(row0_id));
        erased_sel.set(Some(row1_id));
        assert_eq!(selection.snapshot(), Some(20));

        let erased_sort = config.sort_order().expect("sort order bound");
        assert_eq!(erased_sort.snapshot(), Some(TableSort::ascending(0)));
        config.toggle_sort(0);
        assert_eq!(
            sort_order.snapshot(),
            Some(TableSort::descending(PersonSort::Name))
        );
        // Clicking an unsortable column leaves sort order unchanged.
        config.toggle_sort(1);
        assert_eq!(
            sort_order.snapshot(),
            Some(TableSort::descending(PersonSort::Name))
        );
    }

    #[test]
    fn multi_selection_round_trips_identities() {
        let people = sample_people();
        let selection = Binding::new(BTreeSet::from([20_u64]));
        let config = table(people, (col("Name", |p: &Person| text(p.name)),))
            .multi_selection(&selection)
            .config();

        let snapshot = config.rows().snapshot();
        let row0_id = snapshot.get_id(0).expect("row 0 id");
        let row1_id = snapshot.get_id(1).expect("row 1 id");

        let TableSelection::Multiple(erased_sel) = config.selection() else {
            panic!("expected multi selection");
        };
        assert_eq!(erased_sel.snapshot(), BTreeSet::from([row1_id]));
        erased_sel.set(BTreeSet::from([row0_id, row1_id]));
        assert_eq!(selection.snapshot(), BTreeSet::from([10, 20]));
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "was not taken from the table's rows")
    )]
    fn foreign_selection_id_is_rejected() {
        let people = sample_people();
        let selection = Binding::new(Some(10_u64));
        let config = table(people, (col("Name", |p: &Person| text(p.name)),))
            .selection(&selection)
            .config();

        let TableSelection::Single(erased_sel) = config.selection() else {
            panic!("expected single selection");
        };
        let foreign = RawId::try_from(999_999).expect("nonzero id");
        erased_sel.set(Some(foreign));
        assert_eq!(selection.snapshot(), Some(10));
    }

    #[test]
    fn sort_direction_toggles_cleanly() {
        assert_eq!(
            SortDirection::Ascending.toggled(),
            SortDirection::Descending
        );
        assert_eq!(
            SortDirection::Descending.toggled(),
            SortDirection::Ascending
        );
        assert_eq!(
            TableSort::next_for_column(None, PersonSort::Name),
            TableSort::ascending(PersonSort::Name)
        );
        assert_eq!(
            TableSort::next_for_column(
                Some(TableSort::ascending(PersonSort::Name)),
                PersonSort::Name
            ),
            TableSort::descending(PersonSort::Name)
        );
    }
}
