use alloc::vec::Vec;
use waterui::{
    prelude::table::{TableColumn, TableConfig},
    views::ViewsExt,
};
use waterui_core::Native;

use crate::{
    IntoFFI, array::WuiArray, components::text::WuiText, ffi_computed, ffi_watcher_ctor,
    reactive::WuiComputed, views::WuiAnyViews,
};

into_ffi! {
   TableConfig,
   pub struct WuiTable {
       columns: *mut WuiComputed<Vec<TableColumn>>,
   }
}

ffi_computed!(Vec<TableColumn>, WuiArray<WuiTableColumn>, table_cols);
ffi_watcher_ctor!(Vec<TableColumn>, WuiArray<WuiTableColumn>, table_cols);

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

ffi_view!(Native<TableConfig>, WuiTable, table);

ffi_view!(TableColumn, WuiTableColumn);
