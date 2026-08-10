use waterui::{
    prelude::table::{TableColumn, TableConfig},
    views::ViewsExt,
};

use crate::{
    AnyView, IntoFFI, IntoRust, WuiAnyView, components::text::WuiText, views::WuiAnyViews,
};

/// C ABI mirror of [`TableConfig`], a view arranging its columns in a table layout.
#[repr(C)]
#[derive(Debug)]
pub struct WuiTable {
    /// Handle to the table's column collection.
    pub columns: *mut WuiAnyViews,
}

impl IntoFFI for TableConfig {
    type FFI = WuiTable;

    fn into_ffi(self) -> Self::FFI {
        WuiTable {
            columns: crate::views::signal_vec_views(
                self.columns,
                |columns, index| columns[index].semantic_id(),
                AnyView::new,
            ),
        }
    }
}

/// C ABI mirror of [`TableColumn`], one labeled column of a table.
#[repr(C)]
#[derive(Debug)]
pub struct WuiTableColumn {
    /// The column label as styled text.
    pub label: WuiText,
    /// The row views for this column.
    pub rows: *mut WuiAnyViews,
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

ffi_view!(TableConfig, WuiTable, table, any());

/// Consumes a table-column descriptor view extracted from a table's known column collection.
///
/// # Safety
///
/// `view` must own a `Native<TableColumn>` and must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_force_as_table_column(view: *mut WuiAnyView) -> WuiTableColumn {
    // SAFETY: the caller contract makes `view` a valid owning handle consumed here.
    let any: AnyView = unsafe { IntoRust::into_rust(view) };
    // SAFETY: the same contract guarantees the erased value is a `Native<TableColumn>`.
    let column = unsafe { *any.downcast_unchecked::<waterui_core::Native<TableColumn>>() };
    column.into_ffi()
}
