//! FFI bindings for event and lifecycle types.

use crate::IntoFFI;
use waterui_core::event::{Event, LifeCycle, LifeCycleHook, OnEvent};

// ============================================================================
// LifeCycle (one-time handlers for appear/disappear)
// ============================================================================

/// FFI lifecycle enum for one-time lifecycle events.
#[derive(Clone, Copy)]
#[repr(C)]
pub enum WuiLifeCycle {
    Appear,
    Disappear,
}

impl IntoFFI for LifeCycle {
    type FFI = WuiLifeCycle;
    fn into_ffi(self) -> Self::FFI {
        match self {
            LifeCycle::Appear => WuiLifeCycle::Appear,
            LifeCycle::Disappear => WuiLifeCycle::Disappear,
            _ => panic!("unsupported LifeCycle variant for FFI"),
        }
    }
}

/// Wrapper for LifeCycleHook to avoid orphan rule issues.
pub struct WuiLifeCycleHookHandler(pub LifeCycleHook);

/// FFI-safe representation of a lifecycle hook.
#[repr(C)]
pub struct WuiLifeCycleHook {
    /// The lifecycle event to listen for.
    pub lifecycle: WuiLifeCycle,
    /// Opaque pointer to the LifeCycleHook (owns the handler).
    pub handler: *mut WuiLifeCycleHookHandler,
}

impl IntoFFI for LifeCycleHook {
    type FFI = WuiLifeCycleHook;
    fn into_ffi(self) -> Self::FFI {
        let lifecycle = self.lifecycle().into_ffi();
        WuiLifeCycleHook {
            lifecycle,
            handler: alloc::boxed::Box::into_raw(alloc::boxed::Box::new(WuiLifeCycleHookHandler(
                self,
            ))),
        }
    }
}

/// Calls a LifeCycleHook handler with the given environment.
///
/// # Safety
///
/// * `handler` must be a valid pointer to a WuiLifeCycleHookHandler.
/// * `env` must be a valid pointer to a WuiEnv.
/// * This consumes the handler - it can only be called once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_lifecycle_hook(
    handler: *mut WuiLifeCycleHookHandler,
    env: *const crate::WuiEnv,
) {
    let handler =
        unsafe { crate::expect_non_null_mut(handler, "waterui_call_lifecycle_hook", "handler") };
    let env = unsafe { crate::expect_non_null(env, "waterui_call_lifecycle_hook", "env") };
    let _ = crate::ffi_boundary("waterui_call_lifecycle_hook", || unsafe {
        let hook = alloc::boxed::Box::from_raw(handler);
        hook.0.handle(env);
    });
}

/// Drops a LifeCycleHook handler without calling it.
///
/// # Safety
///
/// * `handler` must be a valid pointer to a WuiLifeCycleHookHandler.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_lifecycle_hook(handler: *mut WuiLifeCycleHookHandler) {
    unsafe {
        crate::expect_non_null_mut(handler, "waterui_drop_lifecycle_hook", "handler");
    }
    unsafe {
        drop(alloc::boxed::Box::from_raw(handler));
    }
}

// ============================================================================
// Event (repeatable handlers for interaction events like hover)
// ============================================================================

/// FFI event enum for repeatable interaction events.
#[derive(Clone, Copy)]
#[repr(C)]
pub enum WuiEvent {
    HoverEnter,
    HoverExit,
}

impl IntoFFI for Event {
    type FFI = WuiEvent;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Event::HoverEnter => WuiEvent::HoverEnter,
            Event::HoverExit => WuiEvent::HoverExit,
            _ => panic!("unsupported Event variant for FFI"),
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
/// This handler can be called multiple times (repeatable).
///
/// # Safety
///
/// * `handler` must be a valid pointer to a WuiOnEventHandler.
/// * `env` must be a valid pointer to a WuiEnv.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_on_event(
    handler: *mut WuiOnEventHandler,
    env: *const crate::WuiEnv,
) {
    let handler =
        unsafe { crate::expect_non_null_mut(handler, "waterui_call_on_event", "handler") };
    let env = unsafe { crate::expect_non_null(env, "waterui_call_on_event", "env") };
    let _ = crate::ffi_boundary("waterui_call_on_event", || {
        handler.0.handle(env);
    });
}

/// Drops an OnEvent handler.
///
/// # Safety
///
/// * `handler` must be a valid pointer to a WuiOnEventHandler.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_on_event(handler: *mut WuiOnEventHandler) {
    unsafe {
        crate::expect_non_null_mut(handler, "waterui_drop_on_event", "handler");
    }
    unsafe {
        drop(alloc::boxed::Box::from_raw(handler));
    }
}
