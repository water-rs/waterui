use crate::action::{WuiIndexAction, WuiMoveAction};
use crate::reactive::WuiComputed;
use crate::views::WuiAnyViews;
use crate::{IntoFFI, WuiAnyView};
use waterui::component::list::{ListConfig, ListItem};
use waterui::views::ViewsExt;

/// FFI representation of a list item.
#[repr(C)]
pub struct WuiListItem {
    /// The content view for this item.
    pub content: *mut WuiAnyView,
    /// Read-only signal indicating whether this item can be deleted.
    pub deletable: *mut WuiComputed<bool>,
}

impl IntoFFI for ListItem {
    type FFI = WuiListItem;

    fn into_ffi(self) -> Self::FFI {
        WuiListItem {
            content: self.content.into_ffi(),
            deletable: self.deletable.into_ffi(),
        }
    }
}

#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_force_as_list_item(view: *mut WuiAnyView) -> WuiListItem {
    let any: waterui::AnyView = unsafe { crate::IntoRust::into_rust(view) };
    unsafe { (*any.downcast_unchecked::<ListItem>()).into_ffi() }
}

#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub extern "C" fn waterui_list_item_id() -> crate::WuiTypeId {
    crate::WuiTypeId::of::<ListItem>()
}

#[cfg(feature = "android-jni")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_listItemId<'local>(
    mut env: crate::jni::JNIEnv<'local>,
    _class: crate::jni::JClass<'local>,
) -> crate::jni::jobject {
    let type_id = crate::WuiTypeId::of::<ListItem>();
    crate::jni::type_id_to_java(&mut env, type_id).into_raw()
}

#[cfg(feature = "android-jni")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_forceAsListItem<'local>(
    mut env: crate::jni::JNIEnv<'local>,
    _class: crate::jni::JClass<'local>,
    view_ptr: crate::jni::jlong,
) -> crate::jni::jobject {
    use crate::jni::convert::jlong_to_ptr_mut;
    let view_ptr: *mut WuiAnyView = unsafe { jlong_to_ptr_mut(view_ptr) };
    let any: waterui::AnyView = unsafe { crate::IntoRust::into_rust(view_ptr) };
    let ffi_struct: WuiListItem = unsafe { (*any.downcast_unchecked::<ListItem>()).into_ffi() };
    crate::jni::convert::struct_to_java(&mut env, &ffi_struct).into_raw()
}

/// FFI representation of a list.
#[repr(C)]
pub struct WuiList {
    /// The list contents (array of list items).
    pub contents: *mut WuiAnyViews,
    /// Read-only signal for edit mode state.
    pub editing: *mut WuiComputed<bool>,
    /// Optional delete callback (null if not deletable).
    pub on_delete: *mut WuiIndexAction,
    /// Optional move callback (null if not reorderable).
    pub on_move: *mut WuiMoveAction,
}

impl IntoFFI for ListConfig {
    type FFI = WuiList;

    fn into_ffi(self) -> Self::FFI {
        WuiList {
            contents: self.contents.erase().into_ffi(),
            editing: self.editing.into_ffi(),
            on_delete: self.on_delete.into_ffi(),
            on_move: self.on_move.into_ffi(),
        }
    }
}

ffi_view!(ListConfig, WuiList, list);
