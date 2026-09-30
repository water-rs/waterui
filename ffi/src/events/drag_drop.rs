//! FFI bindings for drag and drop types.

use alloc::boxed::Box;
use suiteki::Str;
use waterui::Url;
use waterui::drag_drop::{DragPayload, Draggable, DropDestination, PlatformRepresentation};

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

impl IntoFFI for DragPayload {
    type FFI = WuiDragData;
    fn into_ffi(self) -> Self::FFI {
        match self.platform_representation() {
            PlatformRepresentation::Text(s) => WuiDragData {
                tag: WuiDragDataTag::Text,
                value: s.clone().into_ffi(),
            },
            PlatformRepresentation::Url(url) => WuiDragData {
                tag: WuiDragDataTag::Url,
                value: url.inner().into_ffi(),
            },
            PlatformRepresentation::Files(_) | PlatformRepresentation::InProcess => {
                panic!("waterui drag/drop FFI does not support this DragPayload kind")
            }
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
        wrapper.0.payload().into_ffi()
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
///
/// # Panics
///
/// Panics if `data_value` is not a parseable URL while `data_tag` is
/// `WuiDragDataTag::Url`: the backend contract requires delivering a valid
/// URL payload.
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

        // Build the payload from the tag and value, then deliver it to the
        // destination — it routes the typed value into the handler the app
        // registered.
        let payload = match data_tag {
            WuiDragDataTag::Text => DragPayload::new(value),
            WuiDragDataTag::Url => DragPayload::new(
                value
                    .parse::<Url>()
                    .expect("backend delivered an invalid drop URL"),
            ),
        };

        handler.call(|handler| handler.deliver(payload, &env));
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
        handler.call(|handler| handler.enter(&env));
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
        handler.call(|handler| handler.exit(&env));
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
