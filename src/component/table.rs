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

use alloc::vec::Vec;
use nami::{Computed, Signal, SignalExt, impl_constant, signal::IntoSignal};
use waterui_core::{
    view::{ConfigurableView, Hook, ViewConfiguration},
    views::SharedAnyViews,
};
use waterui_text::Text;

use crate::{AnyView, Environment, View, views::Views};

use waterui_core::{Native, NativeView};

/// Configuration for a table component.
#[derive(Debug)]
pub struct TableConfig {
    /// Columns that make up the table.
    pub columns: Computed<Vec<TableColumn>>,
}

impl NativeView for TableConfig {}

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
        // User customization via Hook takes precedence
        if let Some(hook) = env.get::<Hook<TableConfig>>() {
            return AnyView::new(hook.apply(env, config));
        }
        // Native backend can catch TableConfig, otherwise falls back to DefaultTableView
        let fallback = DefaultTableView::new(config.columns.clone());
        AnyView::new(Native::new(config).with_fallback(fallback))
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
            rows: SharedAnyViews::new(contents),
        }
    }

    /// Returns the rows of content in this column.
    #[must_use]
    pub fn rows(&self) -> SharedAnyViews<Text> {
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

// ============================================================================
// Default Table View Implementation
// ============================================================================

use crate::ViewExt;
use waterui_color::Grey;
use waterui_core::dynamic::watch;
use waterui_layout::scroll::scroll;
use waterui_layout::stack::{HorizontalAlignment, hstack, vstack};

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

        // Use watch to reactively rebuild when columns change
        watch(columns, move |cols: Vec<TableColumn>| {
            if cols.is_empty() {
                return AnyView::new(());
            }

            // Find the maximum number of rows across all columns
            let max_rows = cols.iter().map(|c| c.rows().len()).max().unwrap_or(0);

            // Build header row - each cell gets equal flex weight for consistent width
            let header_views: Vec<AnyView> = cols
                .iter()
                .map(|col| AnyView::new(col.label().bold().max_width(f32::MAX)))
                .collect();

            // Build data rows
            let mut row_views: Vec<AnyView> = Vec::with_capacity(max_rows + 2);

            // Add header row
            row_views.push(AnyView::new(hstack(header_views)));

            // Add divider between header and data
            row_views.push(AnyView::new(Grey.height(1.0).max_width(f32::MAX)));

            // Add data rows
            for row_idx in 0..max_rows {
                let row_cells: Vec<AnyView> = cols
                    .iter()
                    .map(|col| {
                        col.rows().get_view(row_idx).map_or_else(
                            || AnyView::new(Text::new("").max_width(f32::MAX)),
                            |text| AnyView::new(text.max_width(f32::MAX)),
                        )
                    })
                    .collect();

                // Add row divider between data rows
                if row_idx > 0 {
                    row_views.push(AnyView::new(Grey.height(1.0).max_width(f32::MAX)));
                }
                row_views.push(AnyView::new(hstack(row_cells)));
            }

            // Use leading alignment so all content is left-aligned
            AnyView::new(
                scroll(
                    vstack(row_views)
                        .alignment(HorizontalAlignment::Leading)
                        .spacing(4.0),
                )
                .padding(),
            )
        })
    }
}
