use crate::layout::Alignment;
use alloc::vec::Vec;
use waterui_core::{raw_view, view::TupleViews, AnyView};
#[derive(Debug)]
#[must_use]
pub struct Grid {
    pub alignment: Alignment,
    pub h_space: f64,
    pub v_space: f64,
    pub rows: Vec<GridRow>,
}

raw_view!(Grid);

impl Grid {
    pub fn new(alignment: Alignment, rows: impl IntoIterator<Item = GridRow>) -> Self {
        Grid {
            alignment,
            h_space: 5.0,
            v_space: 5.0,
            rows: rows.into_iter().collect(),
        }
    }
}
#[derive(Debug)]
pub struct GridRow {
    pub columns: Vec<AnyView>,
}

pub fn row(columns: impl TupleViews) -> GridRow {
    GridRow {
        columns: columns.into_views(),
    }
}
