use crate::action::{WuiIndexAction, WuiMoveAction};
use crate::reactive::WuiComputed;
use crate::views::WuiAnyViews;
use crate::{IntoFFI, WuiAnyView, WuiStr};
use core::fmt;
use nami::SignalExt;
use waterui::Str;
use waterui::component::list::{ListConfig, ListItem, ListSection};
use waterui::views::ViewsExt;

/// FFI representation of a list item.
///
/// `section_label` and `section_footer` are owned by the consumer — when
/// they're empty the item carries no section break, otherwise the item opens
/// a new logical section visible to the renderer (`UITableView` sections,
/// `NSTableView` group rows, Material list groups, ...). Both fields are
/// passed by value so ownership of the underlying byte buffers transfers
/// cleanly to the backend; no separate drop call is required.
#[repr(C)]
pub struct WuiListItem {
    /// The content view for this item.
    pub content: *mut WuiAnyView,
    /// Read-only signal indicating whether this item can be deleted.
    pub deletable: *mut WuiComputed<bool>,
    /// Section header carried by this item, or empty when the item does not
    /// start a new section.
    pub section_label: WuiStr,
    /// Section footer carried by this item, or empty when no footer is set.
    pub section_footer: WuiStr,
}

impl fmt::Debug for WuiListItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WuiListItem")
            .field("content", &self.content)
            .field("deletable", &self.deletable)
            .finish_non_exhaustive()
    }
}

fn empty_wuistr() -> WuiStr {
    Str::default().into_ffi()
}

fn section_to_ffi(section: Option<ListSection>) -> (WuiStr, WuiStr) {
    match section {
        None => (empty_wuistr(), empty_wuistr()),
        Some(ListSection { label, footer }) => (
            label.unwrap_or_default().into_ffi(),
            footer.unwrap_or_default().into_ffi(),
        ),
    }
}

impl IntoFFI for ListItem {
    type FFI = WuiListItem;

    fn into_ffi(self) -> Self::FFI {
        let (section_label, section_footer) = section_to_ffi(self.section);
        WuiListItem {
            content: self.content.into_ffi(),
            deletable: self.deletable.into_ffi(),
            section_label,
            section_footer,
        }
    }
}

/// Consumes a view handle and reinterprets it as a `WuiListItem`.
///
/// # Safety
///
/// `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
/// `ListItem`; it is consumed by this call and must not be used afterwards.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_force_as_list_item(view: *mut WuiAnyView) -> WuiListItem {
    // SAFETY: the caller contract makes `view` a valid owning handle consumed here.
    let any: waterui::AnyView = unsafe { crate::IntoRust::into_rust(view) };
    // SAFETY: the same contract guarantees the erased value is a `ListItem`.
    unsafe { (*any.downcast_unchecked::<ListItem>()).into_ffi() }
}

#[cfg(feature = "android-jni")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_listItemId<'local>(
    mut env: crate::jni::JNIEnv<'local>,
    _class: crate::jni::JClass<'local>,
) -> crate::jni::jobject {
    crate::jni::with_env(&mut env, |env| {
        let type_id = crate::WuiTypeId::of::<ListItem>();
        crate::jni::type_id_to_java(env, type_id).into_raw()
    })
}

#[cfg(feature = "android-jni")]
#[unsafe(no_mangle)]
/// Forces an owned view handle into a list-item descriptor.
///
/// # Safety
///
/// `view_ptr` must be a valid owning `ListItem` view pointer and must not be
/// used after this call.
pub unsafe extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_forceAsListItem<'local>(
    mut env: crate::jni::JNIEnv<'local>,
    _class: crate::jni::JClass<'local>,
    view_ptr: crate::jni::jlong,
) -> crate::jni::jobject {
    use crate::jni::convert::jlong_to_ptr_mut;
    crate::jni::with_env(&mut env, |env| {
        let view_ptr: *mut WuiAnyView = unsafe { jlong_to_ptr_mut(view_ptr) };
        let any: waterui::AnyView = unsafe { crate::IntoRust::into_rust(view_ptr) };
        let ffi_struct: WuiListItem = unsafe { (*any.downcast_unchecked::<ListItem>()).into_ffi() };
        crate::jni::convert::struct_to_java(env, &ffi_struct).into_raw()
    })
}

/// FFI representation of a list.
#[repr(C)]
#[derive(Debug)]
pub struct WuiList {
    /// The list contents (array of list items).
    pub contents: *mut WuiAnyViews,
    /// Read-only signal for edit mode state.
    pub editing: *mut WuiComputed<bool>,
    /// Optional delete callback (null if not deletable).
    pub on_delete: *mut WuiIndexAction,
    /// Optional move callback (null if not reorderable).
    pub on_move: *mut WuiMoveAction,
    /// Optional requested row index (null when uncontrolled).
    pub target_index: *mut WuiComputed<i32>,
    /// Optional scroll request generation (null when uncontrolled).
    pub scroll_generation: *mut WuiComputed<i32>,
    /// Whether rows carry semantic section markers.
    pub uses_sections: bool,
}

impl IntoFFI for ListConfig {
    type FFI = WuiList;

    fn into_ffi(self) -> Self::FFI {
        let (target_index, scroll_generation) = self.scroll_controller.map_or_else(
            || (core::ptr::null_mut(), core::ptr::null_mut()),
            |controller| {
                (
                    controller
                        .target()
                        .map(|index| {
                            i32::try_from(index)
                                .expect("list scroll target exceeds the platform index range")
                        })
                        .computed()
                        .into_ffi(),
                    controller.generation().into_ffi(),
                )
            },
        );
        WuiList {
            contents: self.contents.erase().into_ffi(),
            editing: self.editing.into_ffi(),
            on_delete: self.on_delete.into_ffi(),
            on_move: self.on_move.into_ffi(),
            target_index,
            scroll_generation,
            uses_sections: self.uses_sections,
        }
    }
}

ffi_view!(ListConfig, WuiList, list);
