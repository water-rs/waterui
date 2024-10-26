use waterui::component::layout::grid::{Grid, GridRow};

use crate::{array::waterui_array, waterui_anyview};

use super::waterui_alignment;

#[repr(C)]
pub struct waterui_grid {
    pub alignment: waterui_alignment,
    pub h_space: f64,
    pub v_space: f64,
    pub rows: waterui_array<waterui_grid_row>,
}

#[repr(C)]
pub struct waterui_grid_row {
    columns: waterui_array<*mut waterui_anyview>,
}

into_ffi!(GridRow, waterui_grid_row, columns);
into_ffi!(Grid, waterui_grid, alignment, h_space, v_space, rows);
