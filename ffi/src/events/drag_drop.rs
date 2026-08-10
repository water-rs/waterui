//! FFI bindings for drag and drop types.

use alloc::boxed::Box;
use nami::Signal;
use waterui::drag_drop::{DragData, Draggable, DropDestination};
use waterui_str::Str;

use crate::bridge::closure::RetainedCallback;
use crate::{IntoFFI, WuiEnv, WuiStr};
use core::ptr;

// ============================================================================
// DragData FFI
// ============================================================================

/// FFI-safe representation of a drag data type tag.
#[repr(C)]
#[derive(Debug)]
pub enum WuiDragDataTag {
    /// Plain text content.
    Text = 0,
    /// A URL string.
    Url = 1,
}

/// FFI-safe representation of drag data.
#[repr(C)]
#[derive(Debug)]
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
            Self::Text(s) => WuiDragData {
                tag: WuiDragDataTag::Text,
                value: s.into_ffi(),
            },
            Self::Url(s) => WuiDragData {
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
#[derive(Debug)]
pub struct WuiDraggableWrapper(pub Draggable);

/// FFI-safe representation of a draggable metadata.
#[repr(C)]
#[derive(Debug)]
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
/// * `draggable` must be a valid pointer to a `WuiDraggable`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_draggable_get_data(draggable: *const WuiDraggable) -> WuiDragData {
    // SAFETY: the caller contract requires `draggable` to be a valid handle alive for
    // this call, and a live draggable holds a valid inner handle; both are borrowed.
    unsafe {
        let draggable = crate::borrow_ffi(draggable);
        let wrapper = crate::borrow_ffi(draggable.inner);
        wrapper.0.data.get().into_ffi()
    }
}

/// Drops a draggable.
///
/// # Safety
///
/// * `draggable` must be a valid pointer to a `WuiDraggable`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_draggable(draggable: *mut WuiDraggable) {
    // SAFETY: the caller contract requires `draggable` to be a valid handle that no
    // one else is borrowing for this call.
    unsafe {
        let draggable = crate::borrow_ffi_mut(draggable);
        let inner = draggable.inner;
        drop(Box::from_raw(inner));
        draggable.inner = ptr::null_mut();
    }
}

// ============================================================================
// DropDestination FFI
// ============================================================================

/// Wrapper for `DropDestination` to avoid orphan rule issues.
#[derive(Debug)]
pub struct WuiDropHandler(pub RetainedCallback<DropDestination>);

/// FFI-safe representation of a drop destination metadata.
#[repr(C)]
#[derive(Debug)]
pub struct WuiDropDestination {
    /// Opaque pointer to the drop handler.
    pub handler: *mut WuiDropHandler,
}

impl IntoFFI for DropDestination {
    type FFI = WuiDropDestination;
    fn into_ffi(self) -> Self::FFI {
        WuiDropDestination {
            handler: Box::into_raw(Box::new(WuiDropHandler(RetainedCallback::new(self)))),
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
/// * `handler` must be a valid pointer to a `WuiDropDestination`.
/// * `env` must be a valid pointer to a `WuiEnv`.
/// * `data_tag` must be a valid `WuiDragDataTag` value.
/// * `data_value` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_drop_handler(
    dest: *const WuiDropDestination,
    env: *const WuiEnv,
    data_tag: WuiDragDataTag,
    data_value: *const core::ffi::c_char,
) {
    // SAFETY: the caller contract requires `dest` to be a valid handle alive for this
    // call, and a live destination holds a valid handler handle; both are borrowed.
    unsafe {
        let dest = crate::borrow_ffi(dest);
        let handler = crate::borrow_ffi(dest.handler).0.clone();
        let env = crate::borrow_ffi(env).0.clone();
        let data_value = crate::borrow_ffi(data_value);

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
        let mut env_with_data = env;
        env_with_data.insert(data);

        handler.call(|handler| (handler.on_drop)(&env_with_data));
    }
}

/// Calls the enter handler if set.
///
/// # Safety
///
/// * `dest` must be a valid pointer to a `WuiDropDestination`.
/// * `env` must be a valid pointer to a `WuiEnv`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_drop_enter_handler(
    dest: *const WuiDropDestination,
    env: *const WuiEnv,
) {
    // SAFETY: the caller contract requires `dest` to be a valid handle alive for this
    // call, and a live destination holds a valid handler handle; both are borrowed.
    unsafe {
        let dest = crate::borrow_ffi(dest);
        let handler = crate::borrow_ffi(dest.handler).0.clone();
        let env = crate::borrow_ffi(env).0.clone();
        handler.call(|handler| {
            if let Some(on_enter) = &mut handler.on_enter {
                on_enter(&env);
            }
        });
    }
}

/// Calls the exit handler if set.
///
/// # Safety
///
/// * `dest` must be a valid pointer to a `WuiDropDestination`.
/// * `env` must be a valid pointer to a `WuiEnv`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_drop_exit_handler(
    dest: *const WuiDropDestination,
    env: *const WuiEnv,
) {
    // SAFETY: the caller contract requires `dest` to be a valid handle alive for this
    // call, and a live destination holds a valid handler handle; both are borrowed.
    unsafe {
        let dest = crate::borrow_ffi(dest);
        let handler = crate::borrow_ffi(dest.handler).0.clone();
        let env = crate::borrow_ffi(env).0.clone();
        handler.call(|handler| {
            if let Some(on_exit) = &mut handler.on_exit {
                on_exit(&env);
            }
        });
    }
}

/// Drops a drop destination handler.
///
/// # Safety
///
/// * `dest` must be a valid pointer to a `WuiDropDestination`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_drop_destination(dest: *mut WuiDropDestination) {
    // SAFETY: the caller contract requires `dest` to be a valid handle that no one
    // else is borrowing for this call.
    unsafe {
        let dest = crate::borrow_ffi_mut(dest);
        let handler = dest.handler;
        drop(Box::from_raw(handler));
        dest.handler = ptr::null_mut();
    }
}
