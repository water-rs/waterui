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
use nami::{Computed, Signal, SignalExt, impl_constant, signal::IntoSignal};
use waterui_core::{
    Native, NativeView,
    id::Identifiable,
    view::{ConfigurableView, ViewConfiguration},
    views::SharedAnyViews,
};
use waterui_locale::locale_binding;
use waterui_text::{IntoText, Text};

use crate::{Environment, View, views::Views};

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
    Native::new(config.resolve(env))
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

/// A column never changes once built: clones share its identity and its
/// content, so two columns are equal exactly when they are the same column.
impl PartialEq for TableColumn {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.identity, &other.identity)
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
