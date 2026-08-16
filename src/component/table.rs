//! Provides components for creating and displaying data in a tabular format.
//!
//! The primary component is the [`Table`], which is configured with a collection
//! of [`TableColumn`]s. Each [`TableColumn`] represents a vertical column in the
//! table and contains rows of [`Text`] content.
//!
//! # Example
//!
//! ```no_run
//! use waterui::component::table;
//!
//! let table: table::Table<Vec<table::TableColumn>> = std::iter::empty::<table::TableColumn>().collect();
//! let _ = table;
//! ```
use core::any::type_name;

use alloc::{rc::Rc, vec::Vec};
use nami::{
    Computed, Signal, SignalExt, collection::SignalCollection, impl_constant, signal::IntoSignal,
};
use waterui_core::{
    Native, NativeView,
    id::{Identifiable, SelfId},
    view::{ConfigurableView, Hook, ViewConfiguration},
    views::{ForEach, SharedAnyViews},
};
use waterui_locale::locale_binding;
use waterui_text::{IntoText, Text};

use crate::{AnyView, Environment, View, views::Views};

/// Configuration for a table component.
#[derive(Debug)]
pub struct TableConfig {
    /// Columns that make up the table.
    pub columns: Computed<Vec<TableColumn>>,
}

impl NativeView for TableConfig {}

impl TableConfig {
    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        let env = env.clone();
        let locale = locale_binding(&env);
        self.columns = self
            .columns
            .zip(&locale)
            .map(move |(columns, _locale)| {
                columns
                    .into_iter()
                    .map(|column| column.resolve(&env))
                    .collect()
            })
            .computed();
        self
    }
}

/// A tabular layout component composed of reactive text columns.
#[derive(Debug)]
pub struct Table<Col> {
    columns: Col,
}

impl<Col> Table<Col>
where
    Col: Signal<Output = Vec<TableColumn>>,
{
    /// Creates a new table with the specified columns.
    ///
    /// # Arguments
    ///
    /// * `columns` - A collection of `TableColumn` views to be displayed in the table.
    pub const fn new(columns: Col) -> Self {
        Self { columns }
    }
}

impl<Col> ConfigurableView for Table<Col>
where
    Col: Signal<Output = Vec<TableColumn>> + 'static,
{
    type Config = TableConfig;

    fn config(self) -> Self::Config {
        Self::Config {
            columns: self.columns.computed(),
        }
    }
}

impl ViewConfiguration for TableConfig {
    type View = Table<Computed<Vec<TableColumn>>>;

    fn render(self) -> Self::View {
        Table::new(Computed::new(self.columns))
    }
}

fn render_table_config(config: TableConfig, env: &Environment) -> impl View {
    if let Some(hook) = env.get::<Hook<TableConfig>>() {
        AnyView::new(hook.apply(env, config))
    } else {
        let config = config.resolve(env);
        let fallback = DefaultTableView::new(config.columns.clone());
        AnyView::new(Native::new(config).with_fallback(fallback))
    }
}

impl<Col> View for Table<Col>
where
    Col: Signal<Output = Vec<TableColumn>> + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        render_table_config(ConfigurableView::config(self), env)
    }
}

// Tip: no reactivity here
impl FromIterator<TableColumn> for Table<Vec<TableColumn>> {
    fn from_iter<T: IntoIterator<Item = TableColumn>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect::<Vec<_>>())
    }
}

impl_constant!(TableColumn);

/// Represents a column in a table.
#[derive(Clone)]
pub struct TableColumn {
    label: Text,
    rows: SharedAnyViews<Text>,
    identity: Rc<()>,
}

impl_debug!(TableColumn);

waterui_core::raw_view!(TableColumn);

impl Identifiable for TableColumn {
    type Id = usize;

    fn id(&self) -> Self::Id {
        self.semantic_id()
    }
}

impl TableColumn {
    /// Creates a new table column with the given contents.
    ///
    /// # Arguments
    ///
    /// * `contents` - The text content to display in this column.
    pub fn new(label: impl IntoText, contents: impl Views<View = Text> + 'static) -> Self {
        Self {
            label: label.into_text(),
            rows: SharedAnyViews::new(contents),
            identity: Rc::new(()),
        }
    }

    /// Returns the rows of content in this column.
    #[must_use]
    pub fn rows(&self) -> SharedAnyViews<Text> {
        self.rows.clone()
    }

    /// Returns the label of this column.
    pub fn label(&self) -> Text {
        self.label.clone()
    }

    /// Stable semantic identity used by native collection reconciliation.
    #[doc(hidden)]
    #[must_use]
    pub fn semantic_id(&self) -> usize {
        Rc::as_ptr(&self.identity) as usize
    }

    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        self.label = Text::from(self.label.resolve(env));
        self
    }
}

impl<T> FromIterator<T> for TableColumn
where
    T: IntoText,
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let contents = iter
            .into_iter()
            .map(IntoText::into_text)
            .collect::<Vec<Text>>();
        Self::new(Text::verbatim(type_name::<T>()), contents)
    }
}

/// Convenience constructor for building a `Table` from column data.
pub fn table<C>(columns: C) -> Table<C::Signal>
where
    C: IntoSignal<Vec<TableColumn>>,
{
    Table::new(columns.into_signal())
}

/// Convenience constructor for creating a single table column.
pub fn col(label: impl IntoText, rows: impl Views<View = Text> + 'static) -> TableColumn {
    TableColumn::new(label, rows)
}

// ============================================================================
// Default Table View Implementation
// ============================================================================

use crate::ViewExt;
use crate::component::list::{List as UiList, ListItem};
use nami::watcher::{BoxWatcherGuard, Context, WatcherGuard};
use waterui_graphics::color::Grey;
use waterui_layout::stack::{HStack, HorizontalAlignment, vstack};

#[derive(Clone)]
struct TableRowCountSignal {
    columns: Computed<Vec<TableColumn>>,
}

impl TableRowCountSignal {
    const fn new(columns: Computed<Vec<TableColumn>>) -> Self {
        Self { columns }
    }

    fn max_rows(columns: &[TableColumn]) -> usize {
        columns
            .iter()
            .map(|column| column.rows().len().get())
            .max()
            .unwrap_or(0)
    }
}

struct TableRowCountWatchGuard {
    _columns_guard: BoxWatcherGuard,
    _row_guards: Rc<core::cell::RefCell<Vec<BoxWatcherGuard>>>,
}

impl WatcherGuard for TableRowCountWatchGuard {}

impl Signal for TableRowCountSignal {
    type Output = usize;
    type Guard = TableRowCountWatchGuard;

    fn get(&self) -> Self::Output {
        Self::max_rows(&self.columns.get())
    }

    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
        let watcher = Rc::new(watcher);
        let row_guards = Rc::new(core::cell::RefCell::new(Vec::new()));

        let subscribe_rows = {
            let watcher = Rc::clone(&watcher);
            let row_guards = Rc::clone(&row_guards);
            move |columns: Vec<TableColumn>| {
                let mut guards: Vec<BoxWatcherGuard> = Vec::with_capacity(columns.len());
                for column in &columns {
                    let columns = columns.clone();
                    let watcher = Rc::clone(&watcher);
                    guards.push(column.rows().len().watch(move |ctx| {
                        watcher(Context::new(
                            Self::max_rows(&columns),
                            ctx.metadata().clone(),
                        ));
                    }));
                }
                *row_guards.borrow_mut() = guards;
            }
        };

        subscribe_rows(self.columns.get());
        let columns_guard = self.columns.watch({
            let watcher = Rc::clone(&watcher);
            move |ctx| {
                let metadata = ctx.metadata().clone();
                let columns = ctx.into_value();
                let max_rows = Self::max_rows(&columns);
                subscribe_rows(columns);
                watcher(Context::new(max_rows, metadata));
            }
        });

        TableRowCountWatchGuard {
            _columns_guard: columns_guard,
            _row_guards: row_guards,
        }
    }
}

fn build_table_header(columns: Computed<Vec<TableColumn>>) -> impl View {
    HStack::for_each(SignalCollection::new(columns), |column: TableColumn| {
        column.label().bold().max_width(f32::INFINITY)
    })
}

fn build_table_rows(columns: Computed<Vec<TableColumn>>) -> impl View {
    let indices = TableRowCountSignal::new(columns.clone())
        .map(|max_rows| (0..max_rows).map(SelfId::new).collect::<Vec<_>>())
        .computed();
    let rows = ForEach::new(
        SignalCollection::new(indices),
        move |index: SelfId<usize>| {
            let row = index.into_inner();
            let cells = HStack::for_each(
                SignalCollection::new(columns.clone()),
                move |column: TableColumn| {
                    column.rows().get_view(row).map_or_else(
                        || Text::new("").max_width(f32::INFINITY),
                        |text| text.max_width(f32::INFINITY),
                    )
                },
            );
            ListItem::new(cells)
        },
    );

    UiList::new(rows)
}

/// Default table view that renders columns as a grid using stacks.
///
/// This is used as a fallback when no native table implementation is available.
/// It renders:
/// - A header row with column labels (bold)
/// - A divider between header and data
/// - Data rows with proper left alignment
#[derive(Debug)]
struct DefaultTableView {
    columns: Computed<Vec<TableColumn>>,
}

impl DefaultTableView {
    const fn new(columns: Computed<Vec<TableColumn>>) -> Self {
        Self { columns }
    }
}

impl View for DefaultTableView {
    fn body(self, _env: &Environment) -> impl View {
        let columns = self.columns;
        let visible = columns.map(|columns| !columns.is_empty());
        vstack((
            build_table_header(columns.clone()),
            Grey.height(1.0).max_width(f32::INFINITY),
            build_table_rows(columns),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(4.0)
        .padding()
        .visible(visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, rc::Rc, vec, vec::Vec};
    use core::cell::RefCell;
    use nami::collection::List;
    use nami::watcher::Context;

    fn rows_from(list: List<SelfId<usize>>) -> impl Views<View = Text> {
        ForEach::new(list, |id: SelfId<usize>| {
            Text::new(format!("{}", id.into_inner()))
        })
    }

    #[test]
    fn row_count_signal_tracks_max_rows_across_columns() {
        let col1_rows = List::from(vec![SelfId::new(0usize)]);
        let col2_rows = List::from(vec![SelfId::new(0usize), SelfId::new(1usize)]);

        let signal = TableRowCountSignal::new(Computed::constant(vec![
            TableColumn::new("A", rows_from(col1_rows.clone())),
            TableColumn::new("B", rows_from(col2_rows)),
        ]));

        assert_eq!(signal.get(), 2);

        col1_rows.insert(1, SelfId::new(1usize));
        col1_rows.insert(2, SelfId::new(2usize));
        assert_eq!(signal.get(), 3);
    }

    #[test]
    #[allow(
        clippy::manual_contains,
        reason = "`.contains()` resolves to a different in-scope method for this element type; `.iter().any()` is the correct call"
    )]
    fn row_count_signal_watches_column_len_updates() {
        let col_rows = List::from(vec![SelfId::new(0usize)]);
        let signal = TableRowCountSignal::new(Computed::constant(vec![TableColumn::new(
            "Only",
            rows_from(col_rows.clone()),
        )]));

        let seen: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
        let seen_ref = seen.clone();
        let _guard = signal.watch(move |ctx: Context<usize>| {
            seen_ref.borrow_mut().push(ctx.into_value());
        });

        col_rows.insert(1, SelfId::new(1usize));

        let values = seen.borrow();
        assert!(values.iter().any(|&value| value == 1));
        assert!(values.iter().any(|&value| value == 2));
    }
}
