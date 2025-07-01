use crate::Alignment;
use alloc::vec::Vec;
use waterui_core::{AnyView, raw_view, view::TupleViews};
#[derive(Debug)]
#[must_use]
/// Represents a grid layout for arranging views in rows and columns.
pub struct Grid {
    /// The alignment of the grid within its parent container.
    pub alignment: Alignment,
    /// The horizontal space between columns in the grid.
    pub h_space: f64,
    /// The vertical space between rows in the grid.
    pub v_space: f64,
    /// The rows of the grid, each containing a set of columns.
    pub rows: Vec<GridRow>,
}

raw_view!(Grid);

impl Grid {
    /// Creates a new `Grid` with the specified alignment and rows.
    pub fn new(alignment: Alignment, rows: impl IntoIterator<Item = GridRow>) -> Self {
        Self {
            alignment,
            h_space: 5.0,
            v_space: 5.0,
            rows: rows.into_iter().collect(),
        }
    }
}
#[derive(Debug)]
/// Represents a row in the grid layout.
pub struct GridRow {
    /// The columns in this row.
    pub columns: Vec<AnyView>,
}

/// Creates a new `GridRow` from a tuple of views.
pub fn row(columns: impl TupleViews) -> GridRow {
    GridRow {
        columns: columns.into_views(),
    }
}
