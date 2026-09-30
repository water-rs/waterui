//! JNI entry points for the drag-and-drop metadata pair.
//!
//! Kotlin holds a boxed `WuiDraggable` / `WuiDropDestination` pointer handed out
//! by `forceAsMetadataDraggable` / `forceAsMetadataDropDestination` and passes
//! it back here to read the drag payload, deliver drop events, or release the
//! handle.

extern crate alloc;
extern crate std;

use alloc::boxed::Box;
use std::ffi::CString;

use jni::EnvUnowned;
use jni::objects::{JClass, JString};
use jni::sys::{jint, jlong, jobject};

use super::convert::{jlong_to_ptr, jlong_to_ptr_mut, string_from_java, struct_to_java};
use super::with_env;
use crate::drag_drop::{WuiDraggable, WuiDropDestination};

/// Reads the drag payload a `Metadata<Draggable>` is carrying.
///
/// Kotlin calls this once when the drag gesture starts; the returned
/// `DragDataStruct` carries the resolved tag and string.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_draggableGetData<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    draggable_ptr: jlong,
) -> jobject {
    with_env(&mut env, |env| {
        // SAFETY: Kotlin passes back the boxed handle handed out by
        // `forceAsMetadataDraggable`, live until `dropDraggable`.
        let draggable = unsafe { jlong_to_ptr::<WuiDraggable>(draggable_ptr) };
        // SAFETY: the pointer above is a live `WuiDraggable` for this call.
        let data = unsafe { crate::drag_drop::waterui_draggable_get_data(draggable) };
        struct_to_java(env, data).into_raw()
    })
}

/// Releases a `Metadata<Draggable>` value handle.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dropDraggable<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    draggable_ptr: jlong,
) {
    with_env(&mut env, |_env| {
        // SAFETY: Kotlin passes back the boxed handle exactly once at dispose.
        let draggable = unsafe { jlong_to_ptr_mut::<WuiDraggable>(draggable_ptr) };
        // SAFETY: the pointer is the owning handle Kotlin was given.
        unsafe {
            crate::drag_drop::waterui_drop_draggable(draggable);
            drop(Box::from_raw(draggable));
        }
    });
}

/// Delivers an `ACTION_DROP` payload to a `Metadata<DropDestination>` handler.
///
/// `data_tag` mirrors `WuiDragDataTag` (0 = text, 1 = URL) and `data_value` is
/// the string payload extracted from the `ClipData`.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dropDestinationOnDrop<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    destination_ptr: jlong,
    env_ptr: jlong,
    data_tag: jint,
    data_value: JString<'local>,
) {
    with_env(&mut env, |env| {
        let value = string_from_java(env, &data_value);
        // SAFETY: `value` contains no interior NULs, so this cannot fail.
        let c_value = CString::new(value).expect("drop payload contained a NUL byte");
        // SAFETY: Kotlin passes back the live handles it was handed; both are
        // borrowed for this call only.
        unsafe {
            crate::drag_drop::waterui_call_drop_handler(
                jlong_to_ptr::<WuiDropDestination>(destination_ptr),
                jlong_to_ptr::<crate::WuiEnv>(env_ptr),
                match data_tag {
                    1 => crate::drag_drop::WuiDragDataTag::Url,
                    _ => crate::drag_drop::WuiDragDataTag::Text,
                },
                c_value.as_ptr(),
            );
        }
    });
}

/// Delivers `ACTION_DRAG_ENTERED` to a `Metadata<DropDestination>` handler.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dropDestinationOnEnter<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    destination_ptr: jlong,
    env_ptr: jlong,
) {
    with_env(&mut env, |_env| {
        // SAFETY: Kotlin passes back the live handles it was handed; both are
        // borrowed for this call only.
        unsafe {
            crate::drag_drop::waterui_call_drop_enter_handler(
                jlong_to_ptr::<WuiDropDestination>(destination_ptr),
                jlong_to_ptr::<crate::WuiEnv>(env_ptr),
            );
        }
    });
}

/// Delivers `ACTION_DRAG_EXITED` to a `Metadata<DropDestination>` handler.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dropDestinationOnExit<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    destination_ptr: jlong,
    env_ptr: jlong,
) {
    with_env(&mut env, |_env| {
        // SAFETY: Kotlin passes back the live handles it was handed; both are
        // borrowed for this call only.
        unsafe {
            crate::drag_drop::waterui_call_drop_exit_handler(
                jlong_to_ptr::<WuiDropDestination>(destination_ptr),
                jlong_to_ptr::<crate::WuiEnv>(env_ptr),
            );
        }
    });
}

/// Releases a `Metadata<DropDestination>` value handle.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dropDropDestination<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    destination_ptr: jlong,
) {
    with_env(&mut env, |_env| {
        // SAFETY: Kotlin passes back the boxed handle exactly once at dispose.
        let destination = unsafe { jlong_to_ptr_mut::<WuiDropDestination>(destination_ptr) };
        // SAFETY: the pointer is the owning handle Kotlin was given.
        unsafe {
            crate::drag_drop::waterui_drop_drop_destination(destination);
            drop(Box::from_raw(destination));
        }
    });
}
