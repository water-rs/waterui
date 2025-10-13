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
//! let table: table::Table = std::iter::empty::<table::TableColumn>().collect();
//! let _ = table;
//! ```
use alloc::rc::Rc;
use alloc::vec::Vec;
use nami::impl_constant;
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_text::Text;

use crate::{
    AnyView, Environment, View,
    component::Native,
    views::{AnyViews, Views},
};

/// Configuration for a table component.
#[derive(Debug)]
pub struct TableConfig {
    /// Columns that make up the table.
    pub columns: AnyViews<TableColumn>,
}

/// A tabular layout component composed of reactive text columns.
#[derive(Debug)]
pub struct Table<C: Views<View = TableColumn> = AnyViews<TableColumn>>(C);

impl<C> Table<C>
where
    C: Views<View = TableColumn>,
{
    /// Creates a new table with the specified columns.
    pub const fn new(columns: C) -> Self {
        Self(columns)
    }
}

impl<C> ConfigurableView for Table<C>
where
    C: Views<View = TableColumn> + 'static,
{
    type Config = TableConfig;

    fn config(self) -> Self::Config {
        TableConfig {
            columns: AnyViews::new(self.0),
        }
    }
}

impl ViewConfiguration for TableConfig {
    type View = Table<AnyViews<TableColumn>>;

    fn render(self) -> Self::View {
        Table::new(self.columns)
    }
}

impl From<TableConfig> for Table<AnyViews<TableColumn>> {
    fn from(value: TableConfig) -> Self {
        value.render()
    }
}

impl<C> View for Table<C>
where
    C: Views<View = TableColumn> + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        let config = ConfigurableView::config(self);
        if let Some(hook) = env.get::<Hook<TableConfig>>() {
            AnyView::new(hook.apply(env, config))
        } else {
            AnyView::new(Native(config))
        }
    }
}

impl FromIterator<TableColumn> for Table {
    fn from_iter<T: IntoIterator<Item = TableColumn>>(iter: T) -> Self {
        let columns = AnyViews::new(iter.into_iter().collect::<Vec<_>>());
        TableConfig { columns }.into()
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
    /// The rows of content in this column.
    rows: Rc<AnyViews<Text>>,
}

impl_debug!(TableColumn);

waterui_core::raw_view!(TableColumn);

impl TableColumn {
    /// Creates a new table column with the given contents.
    ///
    /// # Arguments
    ///
    /// * `contents` - The text content to display in this column.
    pub fn new(contents: impl Views<View = Text> + 'static) -> Self {
        let rows = AnyViews::new(contents);
        Self {
            rows: Rc::new(rows),
        }
    }

    /// Consumes the column and returns the underlying row collection.
    #[must_use]
    pub fn into_rows(self) -> Rc<AnyViews<Text>> {
        self.rows
    }
}

impl<T> FromIterator<T> for TableColumn
where
    T: Into<Text>,
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let contents = iter.into_iter().map(Into::into).collect::<Vec<Text>>();
        Self::new(contents)
    }
}

/// Convenience constructor for building a `Table` from column data.
pub const fn table<C>(columns: C) -> Table<C>
where
    C: Views<View = TableColumn>,
{
    Table::new(columns)
}

/// Convenience constructor for creating a single table column.
pub fn col(rows: impl Views<View = Text> + 'static) -> TableColumn {
    TableColumn::new(rows)
}
