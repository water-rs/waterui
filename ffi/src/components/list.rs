use crate::{IntoFFI, IntoRust, WuiAnyView, WuiEnv, WuiTypeId};
use waterui::component::list::ListItem;
use waterui::Environment;

#[repr(C)]
pub struct WuiListItem {
    pub content: *mut WuiAnyView,
}

ffi_struct!(
    ListItem,
    WuiListItem,
    content
);

ffi_view!(
    ListItem,
    WuiListItem,
    waterui_list_item_id,
    waterui_force_as_list_item
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
