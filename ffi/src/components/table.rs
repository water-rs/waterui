use crate::views::WuiSharedAnyViews;
use crate::{IntoFFI, WuiId, WuiTypeId};
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::fmt::Display;
use core::ops::RangeBounds;
use waterui::AnyView;
use waterui::component::{
    Native,
    table::{TableColumn, TableConfig},
};
use waterui::views::AnyViews;
use waterui_core::id::Id;
use waterui_core::views::Views;
use waterui_text::Text;

struct TableColumnRowViews {
    rows: Rc<AnyViews<Text>>,
}

impl Views for TableColumnRowViews {
    type Id = <AnyViews<Text> as Views>::Id;
    type Guard = <AnyViews<Text> as Views>::Guard;
    type View = AnyView;

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.rows.get_id(index)
    }

    fn len(&self) -> usize {
        self.rows.len()
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(nami::watcher::Context<Vec<Self::Id>>) + 'static,
    ) -> Self::Guard {
        self.rows.watch(range, watcher)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.rows.get_view(index).map(AnyView::new)
    }
}

fn into_any_row_views(rows: Rc<AnyViews<Text>>) -> Rc<AnyViews<AnyView>> {
    Rc::new(AnyViews::new(TableColumnRowViews { rows }))
}

fn expect_index_in_bounds<T: Display>(len: usize, index: usize, context: &str, detail: T) {
    if index < len {
        return;
    }
    panic!("{context}: index {index} out of range (len = {len}, detail = {detail})");
}

#[repr(C)]
pub struct WuiTableColumn {
    pub rows: *mut WuiSharedAnyViews,
}

#[repr(C)]
pub struct WuiTable {
    pub columns: *mut WuiTableColumns,
}

impl IntoFFI for TableColumn {
    type FFI = WuiTableColumn;

    fn into_ffi(self) -> Self::FFI {
        WuiTableColumn {
            rows: into_any_row_views(self.into_rows()).into_ffi(),
        }
    }
}

impl IntoFFI for TableConfig {
    type FFI = WuiTable;

    fn into_ffi(self) -> Self::FFI {
        WuiTable {
            columns: self.columns.into_ffi(),
        }
    }
}

ffi_view!(
    Native<TableConfig>,
    WuiTable,
    waterui_table_id,
    waterui_force_as_table
);

ffi_type!(
    WuiTableColumns,
    waterui::views::AnyViews<TableColumn>,
    waterui_drop_table_columns
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_table_columns_len(columns: *const WuiTableColumns) -> usize {
    assert!(
        !columns.is_null(),
        "waterui_table_columns_len: received null pointer"
    );
    unsafe { (*columns).0.len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_table_columns_get_column(
    columns: *const WuiTableColumns,
    index: usize,
) -> WuiTableColumn {
    assert!(
        !columns.is_null(),
        "waterui_table_columns_get_column: received null pointer"
    );
    let columns = unsafe { &*columns };
    let len = columns.0.len();
    expect_index_in_bounds(len, index, "waterui_table_columns_get_column", "column");
    columns
        .0
        .get_view(index)
        .unwrap_or_else(|| {
            panic!(
                "waterui_table_columns_get_column: len reported {len} but no view at index {index}"
            )
        })
        .into_ffi()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_table_columns_get_id(
    columns: *const WuiTableColumns,
    index: usize,
) -> WuiId {
    assert!(
        !columns.is_null(),
        "waterui_table_columns_get_id: received null pointer"
    );
    let columns = unsafe { &*columns };
    let len = columns.0.len();
    expect_index_in_bounds(len, index, "waterui_table_columns_get_id", "column");
    let id = columns.0.get_id(index).unwrap_or_else(|| {
        panic!("waterui_table_columns_get_id: len reported {len} but no id at index {index}")
    });
    let raw = id.into_inner().get();
    let raw = i32::try_from(raw).unwrap_or_else(|_| {
        panic!("waterui_table_columns_get_id: identifier {raw} exceeds i32 range")
    });
    let resolved = Id::try_from(raw)
        .unwrap_or_else(|_| panic!("waterui_table_columns_get_id: identifier must be non-zero"));
    IntoFFI::into_ffi(resolved)
}

#[unsafe(no_mangle)]
pub extern "C" fn waterui_table_column_id() -> WuiTypeId {
    core::any::TypeId::of::<TableColumn>().into_ffi()
}
