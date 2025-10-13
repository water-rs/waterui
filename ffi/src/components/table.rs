use alloc::vec::Vec;
use nami::Computed;
use waterui::{
    prelude::{
        Native,
        table::{TableColumn, TableConfig},
    },
    views::ViewsExt,
};

use crate::{IntoFFI, array::WuiArray, impl_computed, views::WuiAnyViews};

#[repr(C)]
pub struct WuiTable {
    columns: *mut Computed<Vec<TableColumn>>,
}

impl_computed!(
    Vec<TableColumn>,
    WuiArray<*mut WuiAnyViews>,
    waterui_read_computed_table_cols,
    waterui_watch_computed_table_cols,
    waterui_drop_computed_table_cols
);

impl IntoFFI for TableColumn {
    type FFI = *mut WuiAnyViews; // Rows
    fn into_ffi(self) -> Self::FFI {
        self.rows().erase().into_ffi()
    }
}

ffi_struct!(TableConfig, WuiTable, columns);

ffi_view!(
    Native<TableConfig>,
    WuiTable,
    waterui_table_id,
    waterui_force_as_table
);
