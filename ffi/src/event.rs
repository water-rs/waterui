//! FFI bindings for event types.

use crate::IntoFFI;
use waterui_core::event::{Event, OnEvent};

/// FFI event enum.
#[repr(C)]
pub enum WuiEvent {
    Appear,
    Disappear,
}

impl IntoFFI for Event {
    type FFI = WuiEvent;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Event::Appear => WuiEvent::Appear,
            Event::Disappear => WuiEvent::Disappear,
            // Handle any future event variants
            _ => WuiEvent::Appear,
        }
    }
}

/// Wrapper for OnEvent to avoid orphan rule issues.
pub struct WuiOnEventHandler(pub OnEvent);

/// FFI-safe representation of an event handler.
#[repr(C)]
pub struct WuiOnEvent {
    /// The event type to listen for.
    pub event: WuiEvent,
    /// Opaque pointer to the OnEvent (owns the handler).
    pub handler: *mut WuiOnEventHandler,
}

impl IntoFFI for OnEvent {
    type FFI = WuiOnEvent;
    fn into_ffi(self) -> Self::FFI {
        let event = self.event().into_ffi();
        WuiOnEvent {
            event,
            handler: alloc::boxed::Box::into_raw(alloc::boxed::Box::new(WuiOnEventHandler(self))),
        }
    }
}

/// Calls an OnEvent handler with the given environment.
///
/// # Safety
///
/// * `handler` must be a valid pointer to a WuiOnEventHandler.
/// * `env` must be a valid pointer to a WuiEnv.
/// * This consumes the handler - it can only be called once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_on_event(
    handler: *mut WuiOnEventHandler,
    env: *const crate::WuiEnv,
) {
    unsafe {
        let on_event = alloc::boxed::Box::from_raw(handler);
        on_event.0.handle(&*env);
    }
}

/// Drops an OnEvent handler without calling it.
///
/// # Safety
///
/// * `handler` must be a valid pointer to a WuiOnEventHandler.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_on_event(handler: *mut WuiOnEventHandler) {
    unsafe {
        drop(alloc::boxed::Box::from_raw(handler));
    }
}
