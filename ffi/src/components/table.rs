use alloc::vec::Vec;
use nami::Computed;
use waterui::{
    prelude::{
        Native,
        table::{TableColumn, TableConfig},
    },
    views::ViewsExt,
};

use crate::{
    IntoFFI, array::WuiArray, components::text::WuiText, impl_computed, views::WuiAnyViews,
};

#[repr(C)]
pub struct WuiTable {
    columns: *mut Computed<Vec<TableColumn>>,
}

impl_computed!(
    Vec<TableColumn>,
    WuiArray<WuiTableColumn>,
    waterui_read_computed_table_cols,
    waterui_watch_computed_table_cols,
    waterui_drop_computed_table_cols
);

#[repr(C)]
pub struct WuiTableColumn {
    label: WuiText,
    rows: *mut WuiAnyViews,
}

impl IntoFFI for TableColumn {
    type FFI = WuiTableColumn;

    fn into_ffi(self) -> Self::FFI {
        WuiTableColumn {
            label: self.label().into_ffi(),
            rows: self.rows().erase().into_ffi(),
        }
    }
}

ffi_struct!(TableConfig, WuiTable, columns);

ffi_view!(
    Native<TableConfig>,
    WuiTable,
    waterui_table_id,
    waterui_force_as_table
);

ffi_view!(
    TableColumn,
    WuiTableColumn,
    waterui_table_column_id,
    waterui_force_as_table_column
);
