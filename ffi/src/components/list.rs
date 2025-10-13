use crate::views::WuiAnyViews;
use crate::{IntoFFI, WuiAnyView, WuiEnv};
use waterui::component::{
    Native,
    list::{ListConfig, ListItem},
};
use waterui::views::ViewsExt;

#[repr(C)]
pub struct WuiListItem {
    pub content: *mut WuiAnyView,
}

ffi_struct!(ListItem, WuiListItem, content);

ffi_view!(
    ListItem,
    WuiListItem,
    waterui_list_item_id,
    waterui_force_as_list_item
);

#[repr(C)]
pub struct WuiList {
    pub contents: *mut WuiAnyViews,
}

impl IntoFFI for ListConfig {
    type FFI = WuiList;

    fn into_ffi(self) -> Self::FFI {
        WuiList {
            contents: self.contents.erase().into_ffi(),
        }
    }
}

ffi_view!(
    Native<ListConfig>,
    WuiList,
    waterui_list_id,
    waterui_force_as_list
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_list_item_call_delete(
    item: *mut WuiListItem,
    env: *const WuiEnv,
    index: usize,
) {
    let _ = (item, env, index);
    // TODO: expose deletion callbacks when backend support is implemented.
}
