use crate::{IntoFFI, WuiTexts, WuiTypeId};
use core::ptr::null_mut;
use waterui::component::table::TableColumn;
use waterui::views::AnyViews;

#[repr(C)]
pub struct WuiTableColumn {
    pub rows: *mut WuiTexts,
}

impl IntoFFI for TableColumn {
    type FFI = WuiTableColumn;
    fn into_ffi(self) -> Self::FFI {
        let rows: AnyViews<_> = (*self.rows).clone();
        WuiTableColumn {
            rows: rows.into_ffi(),
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_table_column_id() -> WuiTypeId {
    core::any::TypeId::of::<TableColumn>().into_ffi()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_force_as_table_column(
    _view: *mut crate::WuiAnyView,
) -> WuiTableColumn {
    WuiTableColumn { rows: null_mut() }
}
