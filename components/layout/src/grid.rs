use crate::Alignment;
use alloc::vec::Vec;
use waterui_core::{AnyView, raw_view, view::TupleViews};
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

pub(crate) mod ffi {
    use waterui_core::{AnyView, ffi_view};
    use waterui_ffi::{array::WuiArray, ffi_struct};

    use crate::ffi::WuiAlignment;

    use super::{Grid, GridRow};

    #[repr(C)]
    pub struct WuiGridRow {
        pub columns: WuiArray<*mut AnyView>,
    }

    ffi_struct!(GridRow, WuiGridRow, columns);

    #[repr(C)]
    pub struct WuiGrid {
        pub alignment: WuiAlignment,
        pub h_space: f64,
        pub v_space: f64,
        pub rows: WuiArray<WuiGridRow>,
    }

    ffi_struct!(Grid, WuiGrid, alignment, h_space, v_space, rows);

    ffi_view!(Grid, WuiGrid, waterui_grid_id, waterui_force_as_grid);
}
