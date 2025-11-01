use crate::views::WuiAnyViews;
use crate::{IntoFFI, WuiAnyView, WuiEnv};
use waterui::component::list::{ListConfig, ListItem};
use waterui::prelude::list::List;
use waterui::views::ViewsExt;

into_ffi! {
    ListItem, pub struct WuiListItem {
        content: *mut WuiAnyView,
    }
}

ffi_view!(ListItem, WuiListItem);

#[repr(C)]
pub struct WuiList {
    contents: *mut WuiAnyViews,
}

impl IntoFFI for ListConfig {
    type FFI = WuiList;

    fn into_ffi(self) -> Self::FFI {
        WuiList {
            contents: self.contents.erase().into_ffi(),
        }
    }
}

native_view!(List, WuiList);

/// Calls the delete callback for a list item.
///
/// # Safety
/// The caller must ensure that `item` and `env` are valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_list_item_call_delete(
    item: *mut WuiListItem,
    env: *const WuiEnv,
    index: usize,
) {
    let _ = (item, env, index);
    // TODO: expose deletion callbacks when backend support is implemented.
}
