//! JNI entry points for the drag-and-drop metadata pair.
//!
//! Kotlin holds a boxed `WuiDraggable` / `WuiDropDestination` pointer handed out
//! by `forceAsMetadataDraggable` / `forceAsMetadataDropDestination` and passes
//! it back here. A drag's value is an opaque payload handle: Kotlin reads a
//! draggable's payload when the drag starts, keeps the handle as the drag's
//! local state for in-process drops, builds one from `ClipData` for drags from
//! other applications, and releases it with `dropDragPayload`.

extern crate alloc;
extern crate std;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use jni::EnvUnowned;
use jni::objects::{JClass, JObject, JObjectArray, JString};
use jni::sys::{jboolean, jint, jlong, jobjectArray, jstring};
use jni::{JNIEnv, jni_str};

use super::convert::{jlong_to_ptr, jlong_to_ptr_mut, string_from_java};
use super::with_env;
use crate::drag_drop::{
    WuiDragPayload, WuiDraggable, WuiDropDestination,
    waterui_call_drop_enter_handler, waterui_call_drop_exit_handler, waterui_call_drop_handler,
    waterui_drag_payload_kind, waterui_drop_destination_accepts, waterui_draggable_payload,
};
use waterui::Url;
use waterui::drag_drop::{DragPayload, Files, PlatformRepresentation};

fn payload_handle(payload: DragPayload) -> jlong {
    Box::into_raw(Box::new(WuiDragPayload(payload))) as jlong
}

/// # Safety
///
/// `payload_ptr` must be a live payload handle handed out by this module.
unsafe fn borrow_payload<'a>(payload_ptr: jlong) -> &'a DragPayload {
    // SAFETY: the caller contract makes the handle live for the borrow.
    unsafe { &(*jlong_to_ptr::<WuiDragPayload>(payload_ptr)).0 }
}

fn parse_url(url: &str) -> Url {
    Url::parse(url).unwrap_or_else(|| panic!("drag payload URL does not parse: {url}"))
}

fn java_string<'local>(env: &mut JNIEnv<'local>, value: &str) -> JString<'local> {
    env.new_string(value)
        .expect("failed to create a drag payload Java string")
}

/// Reads the payload of a drag starting now from a `Metadata<Draggable>`.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_draggablePayload<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    draggable_ptr: jlong,
) -> jlong {
    with_env(&mut env, |_env| {
        // SAFETY: Kotlin passes back the boxed handle handed out by
        // `forceAsMetadataDraggable`, live until `dropDraggable`.
        unsafe { waterui_draggable_payload(jlong_to_ptr::<WuiDraggable>(draggable_ptr)) as jlong }
    })
}

/// The `WuiTransferKind` ordinal of a payload.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dragPayloadKind<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    payload_ptr: jlong,
) -> jint {
    with_env(&mut env, |_env| {
        // SAFETY: Kotlin passes back a live payload handle.
        unsafe { waterui_drag_payload_kind(jlong_to_ptr::<WuiDragPayload>(payload_ptr)) as jint }
    })
}

/// The text of a text payload.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dragPayloadText<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    payload_ptr: jlong,
) -> jstring {
    with_env(&mut env, |env| {
        // SAFETY: Kotlin passes back a live payload handle.
        let payload = unsafe { borrow_payload(payload_ptr) };
        match payload.platform_representation() {
            PlatformRepresentation::Text(text) => java_string(env, text).into_raw(),
            _ => panic!("{payload:?} is not a text payload"),
        }
    })
}

/// The URL of a URL payload.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dragPayloadUrl<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    payload_ptr: jlong,
) -> jstring {
    with_env(&mut env, |env| {
        // SAFETY: Kotlin passes back a live payload handle.
        let payload = unsafe { borrow_payload(payload_ptr) };
        match payload.platform_representation() {
            PlatformRepresentation::Url(url) => java_string(env, url.as_ref()).into_raw(),
            _ => panic!("{payload:?} is not a URL payload"),
        }
    })
}

/// The file URIs of a file payload, as a `String[]`.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dragPayloadFiles<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    payload_ptr: jlong,
) -> jobjectArray {
    with_env(&mut env, |env| {
        // SAFETY: Kotlin passes back a live payload handle.
        let payload = unsafe { borrow_payload(payload_ptr) };
        let PlatformRepresentation::Files(files) = payload.platform_representation() else {
            panic!("{payload:?} is not a file payload");
        };
        let string_class = env
            .find_class(jni_str!("java/lang/String"))
            .expect("java.lang.String class not found");
        let array = env
            .new_object_array(
                super::array_len(files.urls().len()),
                &string_class,
                JObject::null(),
            )
            .expect("failed to create the dropped-file URI array");
        for (index, url) in files.urls().iter().enumerate() {
            let uri = JObject::from(java_string(env, url.as_ref()));
            array
                .set_element(env, index, uri)
                .expect("failed to set a dropped-file URI");
        }
        array.into_raw()
    })
}

/// Creates a text payload for a drag from another application.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dragPayloadFromText<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
) -> jlong {
    with_env(&mut env, |env| {
        let text = string_from_java(env, &text);
        payload_handle(DragPayload::new(waterui::Str::from(text)))
    })
}

/// Creates a URL payload for a drag from another application.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dragPayloadFromUrl<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    url: JString<'local>,
) -> jlong {
    with_env(&mut env, |env| {
        let url = string_from_java(env, &url);
        payload_handle(DragPayload::new(parse_url(&url)))
    })
}

/// Creates a file payload from `content://` or `file://` URIs for a drag from
/// another application.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dragPayloadFromFiles<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    uris: jobjectArray,
) -> jlong {
    with_env(&mut env, |env| {
        // SAFETY: the JVM keeps the `String[]` local reference valid for this call.
        let uris = unsafe { JObjectArray::<JObject>::from_raw(env, uris) };
        let length = uris.len(env).expect("failed to read the URI array length");
        let uris: Vec<String> = (0..length)
            .map(|index| {
                let element = uris
                    .get_element(env, index)
                    .expect("failed to read a dropped-file URI");
                let uri = env
                    .cast_local::<JString>(element)
                    .expect("dropped-file URI array holds a non-String element");
                string_from_java(env, &uri)
            })
            .collect();
        payload_handle(DragPayload::new(Files::new(
            uris.iter().map(|uri| parse_url(uri)),
        )))
    })
}

/// Releases a payload handle.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dropDragPayload<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    payload_ptr: jlong,
) {
    with_env(&mut env, |_env| {
        // SAFETY: Kotlin passes back the owning handle exactly once.
        drop(unsafe { Box::from_raw(jlong_to_ptr_mut::<WuiDragPayload>(payload_ptr)) });
    });
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

/// Whether a `Metadata<DropDestination>` accepts a payload. Kotlin highlights
/// and delivers only accepted drags.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dropDestinationAccepts<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    destination_ptr: jlong,
    payload_ptr: jlong,
) -> jboolean {
    with_env(&mut env, |_env| {
        // SAFETY: Kotlin passes back live destination and payload handles.
        unsafe {
            waterui_drop_destination_accepts(
                jlong_to_ptr::<WuiDropDestination>(destination_ptr),
                jlong_to_ptr::<WuiDragPayload>(payload_ptr),
            )
        }
    })
}

/// Delivers an `ACTION_DROP` payload to a `Metadata<DropDestination>` handler.
/// The payload handle stays owned by Kotlin.
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_dropDestinationOnDrop<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    destination_ptr: jlong,
    env_ptr: jlong,
    payload_ptr: jlong,
) {
    with_env(&mut env, |_env| {
        // SAFETY: Kotlin passes back the live handles it was handed; all are
        // borrowed for this call only.
        unsafe {
            waterui_call_drop_handler(
                jlong_to_ptr::<WuiDropDestination>(destination_ptr),
                jlong_to_ptr::<crate::WuiEnv>(env_ptr),
                jlong_to_ptr::<WuiDragPayload>(payload_ptr),
            );
        }
    });
}

/// Delivers `ACTION_DRAG_ENTERED` of an accepted drag to a
/// `Metadata<DropDestination>` handler.
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
            waterui_call_drop_enter_handler(
                jlong_to_ptr::<WuiDropDestination>(destination_ptr),
                jlong_to_ptr::<crate::WuiEnv>(env_ptr),
            );
        }
    });
}

/// Delivers `ACTION_DRAG_EXITED` of an accepted drag to a
/// `Metadata<DropDestination>` handler.
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
            waterui_call_drop_exit_handler(
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

