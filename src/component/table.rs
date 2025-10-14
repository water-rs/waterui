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
use core::any::type_name;

use alloc::vec::Vec;
use nami::{Computed, Signal, SignalExt, impl_constant, signal::IntoSignal};
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_text::Text;

use crate::{
    AnyView, Environment, View,
    views::{AnyViews, Views},
};

use waterui_core::Native;

/// Configuration for a table component.
#[derive(Debug)]
pub struct TableConfig {
    /// Columns that make up the table.
    pub columns: Computed<Vec<TableColumn>>,
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

impl<Col> View for Table<Col>
where
    Col: Signal<Output = Vec<TableColumn>> + 'static,
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
    rows: AnyViews<Text>,
}

impl_debug!(TableColumn);

waterui_core::raw_view!(TableColumn);

impl TableColumn {
    /// Creates a new table column with the given contents.
    ///
    /// # Arguments
    ///
    /// * `contents` - The text content to display in this column.
    pub fn new(label: impl Into<Text>, contents: impl Views<View = Text> + 'static) -> Self {
        Self {
            label: label.into(),
            rows: AnyViews::new(contents),
        }
    }

    /// Returns the rows of content in this column.
    #[must_use]
    pub fn rows(&self) -> AnyViews<Text> {
        self.rows.clone()
    }

    /// Returns the label of this column.
    #[must_use]
    pub fn label(&self) -> Text {
        self.label.clone()
    }
}

impl<T> FromIterator<T> for TableColumn
where
    T: Into<Text>,
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let contents = iter.into_iter().map(Into::into).collect::<Vec<Text>>();
        Self::new(type_name::<T>(), contents)
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
pub fn col(label: impl Into<Text>, rows: impl Views<View = Text> + 'static) -> TableColumn {
    TableColumn::new(label, rows)
}

// note: may table could be a widget, based on lazy
