use crate::action::{WuiIndexAction, WuiMoveAction};
use crate::reactive::WuiComputed;
use crate::views::WuiAnyViews;
use crate::{IntoFFI, WuiAnyView};
use core::fmt;
use nami::SignalExt;
use waterui::component::list::{ListConfig, ListItem, ListSection};
use waterui::text::styled::StyledStr;
use waterui::views::ViewsExt;

/// FFI representation of the section break a list item may carry.
///
/// A section header is semantic text, not a frozen string: it localizes, and
/// an app can drive it from a signal (`"3 unread"`). Both slots therefore
/// cross as reactive styled-text signals the backend watches like any other
/// text, rather than as a snapshot taken when the list was built. Paragraph
/// alignment is deliberately not carried — section chrome is aligned by the
/// platform, matching [`WuiNavigationSearch::prompt`].
///
/// `has_value` distinguishes "this item opens a section" from "this item
/// continues the previous one": a section that opens with neither a header nor
/// a footer is a pure visual divider, and both of its signals are then null.
/// A null signal must not be read or dropped.
///
/// [`WuiNavigationSearch::prompt`]: crate::components::navigation::WuiNavigationSearch::prompt
#[repr(C)]
#[derive(Debug)]
pub struct WuiListSection {
    /// `true` when this item starts a new logical section.
    pub has_value: bool,
    /// Reactive header text shown above the section, or null when it has none.
    pub label: *mut WuiComputed<StyledStr>,
    /// Reactive footer text shown below the section, or null when it has none.
    pub footer: *mut WuiComputed<StyledStr>,
}

/// FFI representation of a list item.
///
/// `section` is owned by the consumer and passed by value, so ownership of the
/// reactive text handles it carries transfers cleanly to the backend; the
/// backend releases each non-null handle when it is done with the item.
#[repr(C)]
pub struct WuiListItem {
    /// The content view for this item.
    pub content: *mut WuiAnyView,
    /// Read-only signal indicating whether this item can be deleted.
    pub deletable: *mut WuiComputed<bool>,
    /// Read-only signal marking this item as the current selection; the
    /// backend draws its platform's selection chrome while it is true.
    pub selected: *mut WuiComputed<bool>,
    /// Section break carried by this item — see [`WuiListSection`].
    pub section: WuiListSection,
}

impl fmt::Debug for WuiListItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WuiListItem")
            .field("content", &self.content)
            .field("deletable", &self.deletable)
            .field("section", &self.section)
            .finish_non_exhaustive()
    }
}

impl IntoFFI for Option<ListSection> {
    type FFI = WuiListSection;

    fn into_ffi(self) -> Self::FFI {
        let Some(ListSection { label, footer }) = self else {
            return WuiListSection {
                has_value: false,
                label: core::ptr::null_mut(),
                footer: core::ptr::null_mut(),
            };
        };
        let into_signal = |text: waterui::text::Text| text.into_config_without_env().content;
        WuiListSection {
            has_value: true,
            label: label.map_or_else(core::ptr::null_mut, |label| into_signal(label).into_ffi()),
            footer: footer
                .map_or_else(core::ptr::null_mut, |footer| into_signal(footer).into_ffi()),
        }
    }
}

impl IntoFFI for ListItem {
    type FFI = WuiListItem;

    fn into_ffi(self) -> Self::FFI {
        WuiListItem {
            content: self.content.into_ffi(),
            deletable: self.deletable.into_ffi(),
            selected: self.selected.into_ffi(),
            section: self.section.into_ffi(),
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
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_listItemId<'local>(
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
unsafe extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_forceAsListItem<'local>(
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
