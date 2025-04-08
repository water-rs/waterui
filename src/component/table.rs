use alloc::vec::Vec;

use crate::component::{
    Text,
    views::{AnyViews, Views},
};
use waterui_reactive::{Computed, compute::IntoComputed};

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

impl_compute_result!(TableColumn);

/// Represents a column in a table.
#[derive(Clone)]
pub struct TableColumn {
    /// The rows of content in this column.
    pub rows: AnyViews<Text>,
}

impl_compute_result!(AnyViews<Text>);
impl_debug!(TableColumn);

impl TableColumn {
    /// Creates a new table column with the given contents.
    ///
    /// # Arguments
    ///
    /// * `contents` - The text content to display in this column.
    pub fn new(contents: impl Views<Item = Text> + 'static) -> Self {
        Self {
            rows: AnyViews::new(contents),
        }
    }
}
