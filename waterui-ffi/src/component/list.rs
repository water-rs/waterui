use waterui::component::list::List;

use super::lazy::waterui_lazy_view_list;

#[repr(C)]
pub struct waterui_list {
    pub contents: *mut waterui_lazy_view_list,
}

into_ffi!(List, waterui_list, contents);

ffi_view!(
    List,
    waterui_list,
    waterui_view_force_as_list,
    waterui_view_list_id
);
