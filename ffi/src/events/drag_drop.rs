//! FFI bindings for drag and drop types.
//!
//! A drag's value crosses the boundary as an opaque [`WuiDragPayload`] handle.
//! A native backend reads a drag source's payload when the drag begins, writes
//! its platform representation (text, URL or files) to the pasteboard, and
//! keeps the handle itself for the in-process leg of the drag. A drag arriving
//! from another application becomes a payload through the `from_*`
//! constructors. [`waterui_drop_destination_accepts`] decides highlighting and
//! delivery; [`waterui_call_drop_handler`] delivers.

use alloc::boxed::Box;
use alloc::vec::Vec;
use waterui::Url;
use waterui::drag_drop::{
    DragPayload, Draggable, DropDestination, Files, PlatformRepresentation, TransferKind,
};

use crate::bridge::closure::RetainedCallback;
use crate::{IntoFFI, IntoRust, WuiArray, WuiEnv, WuiStr};
use core::ptr;

// ============================================================================
// Payload FFI
// ============================================================================

/// The kind of value a drag carries or a drop destination accepts.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiTransferKind {
    /// Plain text.
    Text = 0,
    /// A URL.
    Url = 1,
    /// A list of file URLs.
    Files = 2,
    /// An application value that stays in the process; it has no pasteboard
    /// representation.
    InProcess = 3,
}

impl From<TransferKind> for WuiTransferKind {
    fn from(kind: TransferKind) -> Self {
        match kind {
            TransferKind::Text => Self::Text,
            TransferKind::Url => Self::Url,
            TransferKind::Files => Self::Files,
            TransferKind::InProcess(_) => Self::InProcess,
        }
    }
}

/// Opaque handle to the value one drag carries.
#[derive(Debug)]
pub struct WuiDragPayload(pub(crate) DragPayload);

fn into_payload_handle(payload: DragPayload) -> *mut WuiDragPayload {
    Box::into_raw(Box::new(WuiDragPayload(payload)))
}

fn parse_url(value: WuiStr) -> Url {
    // SAFETY: the caller contract makes `value` an owning handle from the
    // matching FFI constructor; it is consumed here and not observed again.
    let value = unsafe { value.into_rust() };
    Url::parse(&value).unwrap_or_else(|| panic!("drag payload URL does not parse: {value}"))
}

/// The kind of value `payload` carries.
///
/// # Safety
///
/// * `payload` must be a valid pointer to a live `WuiDragPayload`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drag_payload_kind(
    payload: *const WuiDragPayload,
) -> WuiTransferKind {
    // SAFETY: the caller contract requires `payload` to be a live handle.
    unsafe { crate::borrow_ffi(payload) }.0.kind().into()
}

/// The text a [`WuiTransferKind::Text`] payload carries.
///
/// # Safety
///
/// * `payload` must be a valid pointer to a live `WuiDragPayload`.
///
/// # Panics
///
/// Panics if the payload is not text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drag_payload_text(payload: *const WuiDragPayload) -> WuiStr {
    // SAFETY: the caller contract requires `payload` to be a live handle.
    let payload = unsafe { crate::borrow_ffi(payload) };
    match payload.0.platform_representation() {
        PlatformRepresentation::Text(text) => text.clone().into_ffi(),
        _ => panic!("{:?} is not a text payload", payload.0),
    }
}

/// The URL a [`WuiTransferKind::Url`] payload carries, as a string.
///
/// # Safety
///
/// * `payload` must be a valid pointer to a live `WuiDragPayload`.
///
/// # Panics
///
/// Panics if the payload is not a URL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drag_payload_url(payload: *const WuiDragPayload) -> WuiStr {
    // SAFETY: the caller contract requires `payload` to be a live handle.
    let payload = unsafe { crate::borrow_ffi(payload) };
    match payload.0.platform_representation() {
        PlatformRepresentation::Url(url) => waterui::Str::from(url.clone()).into_ffi(),
        _ => panic!("{:?} is not a URL payload", payload.0),
    }
}

/// The file URLs a [`WuiTransferKind::Files`] payload carries.
///
/// # Safety
///
/// * `payload` must be a valid pointer to a live `WuiDragPayload`.
///
/// # Panics
///
/// Panics if the payload is not a file list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drag_payload_files(
    payload: *const WuiDragPayload,
) -> WuiArray<WuiStr> {
    // SAFETY: the caller contract requires `payload` to be a live handle.
    let payload = unsafe { crate::borrow_ffi(payload) };
    match payload.0.platform_representation() {
        PlatformRepresentation::Files(files) => files
            .urls()
            .iter()
            .map(|url| waterui::Str::from(url.clone()))
            .collect::<Vec<_>>()
            .into_ffi(),
        _ => panic!("{:?} is not a file payload", payload.0),
    }
}

/// Creates a text payload for a drag arriving from another application.
///
/// # Safety
///
/// * `text` must be an owning `WuiStr`; it is consumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drag_payload_from_text(text: WuiStr) -> *mut WuiDragPayload {
    // SAFETY: the caller contract makes `text` an owning handle; it is consumed here.
    let text = unsafe { text.into_rust() };
    into_payload_handle(DragPayload::new(text))
}

/// Creates a URL payload for a drag arriving from another application.
///
/// # Safety
///
/// * `url` must be an owning `WuiStr` holding a valid URL; it is consumed.
///
/// # Panics
///
/// Panics if `url` does not parse.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drag_payload_from_url(url: WuiStr) -> *mut WuiDragPayload {
    into_payload_handle(DragPayload::new(parse_url(url)))
}

/// Creates a file-list payload from file URLs for a drag arriving from another
/// application.
///
/// # Safety
///
/// * `urls` must be an owning `WuiArray` of owning `WuiStr` URLs; it is consumed.
///
/// # Panics
///
/// Panics if any URL does not parse.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drag_payload_from_files(
    urls: WuiArray<WuiStr>,
) -> *mut WuiDragPayload {
    // SAFETY: the caller contract makes `urls` an owning array; it is consumed here.
    let urls = unsafe { urls.into_rust() };
    let files = Files::new(urls.into_iter().map(|url| {
        Url::parse(&url).unwrap_or_else(|| panic!("dropped file URL does not parse: {url}"))
    }));
    into_payload_handle(DragPayload::new(files))
}

/// Releases a payload handle.
///
/// # Safety
///
/// * `payload` must be an owning pointer returned by this module and not
///   released before.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_drag_payload(payload: *mut WuiDragPayload) {
    // SAFETY: the caller contract makes `payload` the owning handle, released once.
    drop(unsafe { Box::from_raw(payload) });
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

/// Reads the payload of a drag starting now from a draggable.
///
/// The returned handle is owned by the caller and released with
/// [`waterui_drop_drag_payload`].
///
/// # Safety
///
/// * `draggable` must be a valid pointer to a `WuiDraggable`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_draggable_payload(
    draggable: *const WuiDraggable,
) -> *mut WuiDragPayload {
    // SAFETY: the caller contract requires `draggable` to be a valid handle alive for
    // this call, and a live draggable holds a valid inner handle; both are borrowed.
    let wrapper = unsafe { crate::borrow_ffi(crate::borrow_ffi(draggable).inner) };
    into_payload_handle(wrapper.0.payload())
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
    /// The kind of payload the destination accepts. A backend registers for the
    /// matching pasteboard type; `InProcess` destinations register for none.
    pub accepted_kind: WuiTransferKind,
}

impl IntoFFI for DropDestination {
    type FFI = WuiDropDestination;
    fn into_ffi(self) -> Self::FFI {
        let accepted_kind = self.accepted_kind().into();
        WuiDropDestination {
            handler: Box::into_raw(Box::new(WuiDropHandler(RetainedCallback::new(self)))),
            accepted_kind,
        }
    }
}

// ============================================================================
// FFI Functions
// ============================================================================

/// Returns `true` if `dest` accepts `payload`: the payload has the type the
/// destination's handler takes. A backend highlights and delivers only
/// accepted drags.
///
/// # Safety
///
/// * `dest` must be a valid pointer to a `WuiDropDestination`.
/// * `payload` must be a valid pointer to a live `WuiDragPayload`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_destination_accepts(
    dest: *const WuiDropDestination,
    payload: *const WuiDragPayload,
) -> bool {
    // SAFETY: the caller contract requires both handles to be live for this call,
    // and a live destination holds a valid handler handle; all are borrowed.
    unsafe {
        let handler = &crate::borrow_ffi(crate::borrow_ffi(dest).handler).0;
        let payload = &crate::borrow_ffi(payload).0;
        handler.call(|destination| destination.accepts(payload))
    }
}

/// Delivers a dropped payload to the destination's handler. The payload stays
/// owned by the caller.
///
/// # Safety
///
/// * `dest` must be a valid pointer to a `WuiDropDestination`.
/// * `env` must be a valid pointer to a `WuiEnv`.
/// * `payload` must be a valid pointer to a live `WuiDragPayload`.
///
/// # Panics
///
/// Panics if the destination does not accept the payload.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_drop_handler(
    dest: *const WuiDropDestination,
    env: *const WuiEnv,
    payload: *const WuiDragPayload,
) {
    // SAFETY: the caller contract requires all three handles to be live for this
    // call, and a live destination holds a valid handler handle; all are borrowed.
    unsafe {
        let handler = crate::borrow_ffi(crate::borrow_ffi(dest).handler).0.clone();
        let env = crate::borrow_ffi(env).0.clone();
        let payload = crate::borrow_ffi(payload).0.clone();
        handler.call(|destination| destination.deliver(payload, &env));
    }
}

/// Reports that an accepted drag entered the destination.
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
        let handler = crate::borrow_ffi(crate::borrow_ffi(dest).handler).0.clone();
        let env = crate::borrow_ffi(env).0.clone();
        handler.call(|destination| destination.enter(&env));
    }
}

/// Reports that an accepted drag left the destination without dropping.
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
        let handler = crate::borrow_ffi(crate::borrow_ffi(dest).handler).0.clone();
        let env = crate::borrow_ffi(env).0.clone();
        handler.call(|destination| destination.exit(&env));
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
