//! Provides components for creating and displaying data in a tabular format.
//!
//! The primary component is the [`Table`], which is configured with a collection
//! of [`TableColumn`]s. Each [`TableColumn`] represents a vertical column in the
//! table and contains rows of [`Text`] content.
//!
//! # Example
//!
//! ```
//! use waterui::prelude::*;
//! use waterui::component::views::AnyViews;
//!
//! // Create two columns, each with two rows of text.
//! let column1 = table::TableColumn::new(AnyViews::new([
//!     Text::new("Row 1, Col 1"),
//!     Text::new("Row 2, Col 1"),
//! ]));
//!
//! let column2 = table::TableColumn::new(AnyViews::new([
//!     Text::new("Row 1, Col 2"),
//!     Text::new("Row 2, Col 2"),
//! ]));
//!
//! // Create a table with the defined columns.
//! let table = table::Table::new(vec![column1, column2]);
//! ```
use alloc::vec::Vec;
use waterui_core::{configurable, id::IdentifableExt};

use crate::{component::{
    views::{AnyViews, Views}, Text
}, prelude::views::ForEach};
use nami::{Computed, impl_constant, signal::IntoComputed};

/// Configuration for a table component.
#[derive(Debug)]
pub struct TableConfig {
    /// Columns that make up the table.
    pub columns: Computed<Vec<TableColumn>>,
}

configurable!(Table, TableConfig);

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
    pub rows: AnyViews<Text>,
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
            rows: AnyViews::new(contents),
        }
    }
}

impl <T>FromIterator<T> for TableColumn where T: Into<Text> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let contents = iter.into_iter().enumerate().map(|(index,item)| item.into().use_id(index)).collect::<Vec<_>>();
        //Self::new(contents)

        //ForEach::new(contents, |d|{});

        todo!()
    }
}


pub fn table(columns: impl IntoComputed<Vec<TableColumn>>) -> Table {
    Table::new(columns)
}

pub fn col(rows: impl Views<View = Text> + 'static) -> TableColumn {
    TableColumn::new(rows)
}