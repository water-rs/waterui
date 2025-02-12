use waterui::component::list::ListConfig;

use super::lazy::waterui_lazy_view_list;

#[repr(C)]
pub struct waterui_list {
    pub contents: *mut waterui_lazy_view_list,
}

into_ffi!(ListConfig, waterui_list, contents);

native_view!(
    ListConfig,
    waterui_list,
    waterui_view_force_as_list,
    waterui_view_list_id
);
