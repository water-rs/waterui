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
use waterui_core::{configurable};
use waterui_text::Text;

use crate::views::{{AnyViews, Views}};
use nami::{Computed, impl_constant, signal::IntoComputed};

/// Configuration for a table component.
#[derive(Debug)]
pub struct TableConfig {
    /// Columns that make up the table.
    pub columns: Computed<Vec<TableColumn>>,
}

configurable!(
    #[doc = "A tabular layout component composed of reactive text columns."]
    Table,
    TableConfig
);

impl Table {
    /// Creates a new table with the specified columns.
    ///
    /// # Arguments
    ///
    /// * `columns` - The columns to display in the table.
    pub fn new(columns: impl IntoComputed<Vec<TableColumn>>) -> Self {
        Self(TableConfig {
            columns: columns.into_computed(),
        })
    }
}

// Tip: no reactivity here
impl FromIterator<TableColumn> for Table {
    fn from_iter<T: IntoIterator<Item = TableColumn>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect::<Vec<_>>())
    }
}

impl_constant!(TableColumn);

/// Represents a column in a table.
#[derive(Clone)]
pub struct TableColumn {
    /// The rows of content in this column.
    pub rows: Rc<AnyViews<Text>>,
}

impl_debug!(TableColumn);

impl TableColumn {
    /// Creates a new table column with the given contents.
    ///
    /// # Arguments
    ///
    /// * `contents` - The text content to display in this column.
    pub fn new(contents: impl Views<View = Text> + 'static) -> Self {
        Self {
            rows: Rc::new( AnyViews::new(contents)),
        }
    }
}

impl<T> FromIterator<T> for TableColumn
where
    T: Into<Text>,
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let contents = iter
            .into_iter()
            .map(Into::into)
            .collect::<Vec<Text>>();
        Self::new(contents)
    }
}

/// Convenience constructor for building a `Table` from column data.
pub fn table(columns: impl IntoComputed<Vec<TableColumn>>) -> Table {
    Table::new(columns)
}

/// Convenience constructor for creating a single table column.
pub fn col(rows: impl Views<View = Text> + 'static) -> TableColumn {
    TableColumn::new(rows)
}
