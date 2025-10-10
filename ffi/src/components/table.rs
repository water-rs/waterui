use crate::WuiTypeId;
use crate::{IntoFFI, WuiAnyView, array::WuiArray};
use alloc::vec::Vec;
use waterui::component::{
    Native,
    table::{TableColumn, TableConfig},
};
use waterui::views::Views;
use waterui::{AnyView, Signal};

#[repr(C)]
pub struct WuiTableColumn {
    pub rows: WuiArray<*mut WuiAnyView>,
}

#[repr(C)]
pub struct WuiTable {
    pub columns: WuiArray<WuiTableColumn>,
}

impl IntoFFI for TableColumn {
    type FFI = WuiTableColumn;

    fn into_ffi(self) -> Self::FFI {
        let rows = &*self.rows;
        let len = rows.len();
        let mut rendered_rows: Vec<AnyView> = Vec::with_capacity(len);
        for index in 0..len {
            if let Some(view) = rows.get_view(index) {
                rendered_rows.push(AnyView::new(view));
            }
        }

        WuiTableColumn {
            rows: rendered_rows.into_ffi(),
        }
    }
}

impl IntoFFI for TableConfig {
    type FFI = WuiTable;

    fn into_ffi(self) -> Self::FFI {
        let columns = self.columns.get();
        WuiTable {
            columns: columns.into_ffi(),
        }
    }
}

ffi_view!(
    Native<TableConfig>,
    WuiTable,
    waterui_table_id,
    waterui_force_as_table
);

#[unsafe(no_mangle)]
pub extern "C" fn waterui_table_column_id() -> WuiTypeId {
    core::any::TypeId::of::<TableColumn>().into_ffi()
}
