//! FFI bindings for drag and drop types.

use alloc::boxed::Box;
use waterui::drag_drop::{DragData, Draggable, DropDestination};
use waterui_str::Str;

use crate::{IntoFFI, WuiEnv, WuiStr};
use core::ptr;

// ============================================================================
// DragData FFI
// ============================================================================

/// FFI-safe representation of a drag data type tag.
#[repr(C)]
pub enum WuiDragDataTag {
    /// Plain text content.
    Text = 0,
    /// A URL string.
    Url = 1,
}

/// FFI-safe representation of drag data.
#[repr(C)]
pub struct WuiDragData {
    /// The type of data.
    pub tag: WuiDragDataTag,
    /// The content (text or URL string).
    pub value: WuiStr,
}

impl IntoFFI for DragData {
    type FFI = WuiDragData;
    fn into_ffi(self) -> Self::FFI {
        match self {
            DragData::Text(s) => WuiDragData {
                tag: WuiDragDataTag::Text,
                value: s.into_ffi(),
            },
            DragData::Url(s) => WuiDragData {
                tag: WuiDragDataTag::Url,
                value: s.into_ffi(),
            },
            _ => panic!("waterui drag/drop FFI does not support this DragData variant"),
        }
    }
}

// ============================================================================
// Draggable FFI
// ============================================================================

/// Opaque wrapper for Draggable.
pub struct WuiDraggableWrapper(pub Draggable);

/// FFI-safe representation of a draggable metadata.
#[repr(C)]
pub struct WuiDraggable {
    /// Opaque pointer to the Draggable wrapper.
    pub inner: *mut WuiDraggableWrapper,
}

impl IntoFFI for Draggable {
    type FFI = WuiDraggable;
    fn into_ffi(self) -> Self::FFI {
        WuiDraggable {
            inner: Box::into_raw(Box::new(WuiDraggableWrapper(self))),
        }
    }
}

/// Gets the current drag data value from a draggable.
///
/// # Safety
///
/// * `draggable` must be a valid pointer to a WuiDraggable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_draggable_get_data(draggable: *const WuiDraggable) -> WuiDragData {
    unsafe {
        let draggable =
            crate::expect_non_null(draggable, "waterui_draggable_get_data", "draggable");
        let wrapper = crate::expect_non_null(
            draggable.inner,
            "waterui_draggable_get_data",
            "draggable.inner",
        );
        use nami::Signal;
        wrapper.0.data.get().into_ffi()
    }
}

/// Drops a draggable.
///
/// # Safety
///
/// * `draggable` must be a valid pointer to a WuiDraggable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_draggable(draggable: *mut WuiDraggable) {
    unsafe {
        let draggable =
            crate::expect_non_null_mut(draggable, "waterui_drop_draggable", "draggable");
        let inner = draggable.inner;
        crate::expect_non_null_mut(inner, "waterui_drop_draggable", "draggable.inner");
        drop(Box::from_raw(inner));
        draggable.inner = ptr::null_mut();
    }
}

// ============================================================================
// DropDestination FFI
// ============================================================================

/// Wrapper for DropDestination to avoid orphan rule issues.
pub struct WuiDropHandler(pub DropDestination);

/// FFI-safe representation of a drop destination metadata.
#[repr(C)]
pub struct WuiDropDestination {
    /// Opaque pointer to the drop handler.
    pub handler: *mut WuiDropHandler,
}

impl IntoFFI for DropDestination {
    type FFI = WuiDropDestination;
    fn into_ffi(self) -> Self::FFI {
        WuiDropDestination {
            handler: Box::into_raw(Box::new(WuiDropHandler(self))),
        }
    }
}

// ============================================================================
// FFI Functions
// ============================================================================

/// Calls the drop handler with the given data.
///
/// # Safety
///
/// * `handler` must be a valid pointer to a WuiDropDestination.
/// * `env` must be a valid pointer to a WuiEnv.
/// * `data_tag` must be a valid WuiDragDataTag value.
/// * `data_value` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_drop_handler(
    dest: *const WuiDropDestination,
    env: *const WuiEnv,
    data_tag: WuiDragDataTag,
    data_value: *const core::ffi::c_char,
) {
    unsafe {
        let dest = crate::expect_non_null(dest, "waterui_call_drop_handler", "dest");
        let env = crate::expect_non_null(env, "waterui_call_drop_handler", "env");
        let handler =
            crate::expect_non_null_mut(dest.handler, "waterui_call_drop_handler", "dest.handler");
        let data_value =
            crate::expect_non_null(data_value, "waterui_call_drop_handler", "data_value");

        // Convert C string to Rust String
        let c_str = core::ffi::CStr::from_ptr(data_value);
        let value = Str::from(core::str::from_utf8_unchecked(c_str.to_bytes()).to_owned());

        // Create DragData from the tag and value
        let data = match data_tag {
            WuiDragDataTag::Text => DragData::Text(value),
            WuiDragDataTag::Url => DragData::Url(value),
        };

        // Clone the environment and insert the DragData
        // This allows handlers to extract DragData using Use<DragData>
        let mut env_with_data = env.0.clone();
        env_with_data.insert(data);

        // Get the handler and call it
        (handler.0.on_drop)(&env_with_data);
    }
}

/// Calls the enter handler if set.
///
/// # Safety
///
/// * `dest` must be a valid pointer to a WuiDropDestination.
/// * `env` must be a valid pointer to a WuiEnv.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_drop_enter_handler(
    dest: *const WuiDropDestination,
    env: *const WuiEnv,
) {
    unsafe {
        let dest = crate::expect_non_null(dest, "waterui_call_drop_enter_handler", "dest");
        let env = crate::expect_non_null(env, "waterui_call_drop_enter_handler", "env");
        let drop_handler = crate::expect_non_null_mut(
            dest.handler,
            "waterui_call_drop_enter_handler",
            "dest.handler",
        );
        if let Some(ref mut on_enter) = drop_handler.0.on_enter {
            (on_enter)(env);
        }
    }
}

/// Calls the exit handler if set.
///
/// # Safety
///
/// * `dest` must be a valid pointer to a WuiDropDestination.
/// * `env` must be a valid pointer to a WuiEnv.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_drop_exit_handler(
    dest: *const WuiDropDestination,
    env: *const WuiEnv,
) {
    unsafe {
        let dest = crate::expect_non_null(dest, "waterui_call_drop_exit_handler", "dest");
        let env = crate::expect_non_null(env, "waterui_call_drop_exit_handler", "env");
        let drop_handler = crate::expect_non_null_mut(
            dest.handler,
            "waterui_call_drop_exit_handler",
            "dest.handler",
        );
        if let Some(ref mut on_exit) = drop_handler.0.on_exit {
            (on_exit)(env);
        }
    }
}

/// Drops a drop destination handler.
///
/// # Safety
///
/// * `dest` must be a valid pointer to a WuiDropDestination.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_drop_destination(dest: *mut WuiDropDestination) {
    unsafe {
        let dest = crate::expect_non_null_mut(dest, "waterui_drop_drop_destination", "dest");
        let handler = dest.handler;
        crate::expect_non_null_mut(handler, "waterui_drop_drop_destination", "dest.handler");
        drop(Box::from_raw(handler));
        dest.handler = ptr::null_mut();
    }
}
