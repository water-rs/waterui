//! FFI bindings for event and lifecycle types.

use crate::IntoFFI;
use crate::bridge::closure::RetainedCallback;
use waterui_core::{
    event::{Event, HoverEvent, LifeCycle, LifeCycleHook, OnEvent},
    layout::Point,
};

// ============================================================================
// Lifecycle (one-time handlers for appear/disappear)
// ============================================================================

/// FFI lifecycle enum for one-time lifecycle events.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub enum WuiLifecycle {
    /// The component appeared (attached to the view hierarchy).
    Appear,
    /// The component disappeared (detached from the view hierarchy).
    Disappear,
}

impl IntoFFI for LifeCycle {
    type FFI = WuiLifecycle;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Appear => WuiLifecycle::Appear,
            Self::Disappear => WuiLifecycle::Disappear,
            _ => panic!("unsupported LifeCycle variant for FFI"),
        }
    }
}

/// Wrapper for `LifeCycleHook` to avoid orphan rule issues.
#[derive(Debug)]
pub struct WuiLifecycleHookHandler(pub LifeCycleHook);

/// FFI-safe representation of a lifecycle hook.
#[repr(C)]
#[derive(Debug)]
pub struct WuiLifecycleHook {
    /// The lifecycle event to listen for.
    pub lifecycle: WuiLifecycle,
    /// Opaque pointer to the lifecycle hook (owns the handler).
    pub handler: *mut WuiLifecycleHookHandler,
}

impl IntoFFI for LifeCycleHook {
    type FFI = WuiLifecycleHook;
    fn into_ffi(self) -> Self::FFI {
        let lifecycle = self.lifecycle().into_ffi();
        WuiLifecycleHook {
            lifecycle,
            handler: alloc::boxed::Box::into_raw(alloc::boxed::Box::new(WuiLifecycleHookHandler(
                self,
            ))),
        }
    }
}

/// Calls a lifecycle hook handler with the given environment.
///
/// # Safety
///
/// * `handler` must be a valid pointer to a `WuiLifecycleHookHandler`.
/// * `env` must be a valid pointer to a `WuiEnv`.
/// * This consumes the handler - it can only be called once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_lifecycle_hook(
    handler: *mut WuiLifecycleHookHandler,
    env: *const crate::WuiEnv,
) {
    let hook = unsafe { alloc::boxed::Box::from_raw(handler) };
    let env = unsafe { crate::borrow_ffi(env) }.0.clone();
    hook.0.handle(&env);
}

/// Drops a lifecycle hook handler without calling it.
///
/// # Safety
///
/// * `handler` must be a valid pointer to a `WuiLifecycleHookHandler`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_lifecycle_hook(handler: *mut WuiLifecycleHookHandler) {
    unsafe {
        drop(alloc::boxed::Box::from_raw(handler));
    }
}

// ============================================================================
// Event (repeatable handlers for interaction events like hover)
// ============================================================================

/// FFI event enum for repeatable interaction events.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub enum WuiEvent {
    /// The cursor entered a component's bounds.
    HoverEnter,
    /// Pointer motion within a component's bounds.
    HoverMove,
    /// The cursor exited a component's bounds.
    HoverExit,
}

impl IntoFFI for Event {
    type FFI = WuiEvent;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::HoverEnter => WuiEvent::HoverEnter,
            Self::HoverMove => WuiEvent::HoverMove,
            Self::HoverExit => WuiEvent::HoverExit,
            _ => panic!("unsupported Event variant for FFI"),
        }
    }
}

/// Wrapper for `OnEvent` to avoid orphan rule issues.
#[derive(Debug)]
pub struct WuiOnEventHandler(RetainedCallback<OnEvent>);

/// FFI-safe representation of an event handler.
#[repr(C)]
#[derive(Debug)]
pub struct WuiOnEvent {
    /// The event type to listen for.
    pub event: WuiEvent,
    /// Opaque pointer to the `OnEvent` (owns the handler).
    pub handler: *mut WuiOnEventHandler,
}

impl IntoFFI for OnEvent {
    type FFI = WuiOnEvent;
    fn into_ffi(self) -> Self::FFI {
        let event = self.event().into_ffi();
        WuiOnEvent {
            event,
            handler: alloc::boxed::Box::into_raw(alloc::boxed::Box::new(WuiOnEventHandler(
                RetainedCallback::new(self),
            ))),
        }
    }
}

/// Calls an `OnEvent` handler with the given environment.
/// This handler can be called multiple times (repeatable).
///
/// # Safety
///
/// * `handler` must be a valid pointer to a `WuiOnEventHandler`.
/// * `env` must be a valid pointer to a `WuiEnv`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_on_event(
    handler: *const WuiOnEventHandler,
    env: *const crate::WuiEnv,
) {
    let handler = unsafe { crate::borrow_ffi(handler) }.0.clone();
    let env = unsafe { crate::borrow_ffi(env) }.0.clone();
    handler.call(|handler| handler.handle(&env));
}

/// Calls an `OnEvent` hover-move handler with the given local pointer position.
///
/// # Safety
///
/// * `handler` must be a valid pointer to a `WuiOnEventHandler`.
/// * `env` must be a valid pointer to a `WuiEnv`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_on_hover_event(
    handler: *const WuiOnEventHandler,
    env: *const crate::WuiEnv,
    x: f32,
    y: f32,
) {
    let handler = unsafe { crate::borrow_ffi(handler) }.0.clone();
    let env = unsafe { crate::borrow_ffi(env) }.0.clone();
    let env_with_hover = env.extending(HoverEvent::new(Point::new(x, y)));
    handler.call(|handler| handler.handle(&env_with_hover));
}

/// Drops an `OnEvent` handler.
///
/// # Safety
///
/// * `handler` must be a valid pointer to a `WuiOnEventHandler`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_on_event(handler: *mut WuiOnEventHandler) {
    unsafe {
        drop(alloc::boxed::Box::from_raw(handler));
    }
}

#[cfg(all(test, feature = "c-api"))]
mod tests {
    use alloc::rc::Rc;
    use core::cell::Cell;

    use super::*;

    #[test]
    fn on_event_survives_reentrant_native_drop_until_callback_returns() {
        let handler_ptr = Rc::new(Cell::new(core::ptr::null_mut::<WuiOnEventHandler>()));
        let callback_finished = Rc::new(Cell::new(false));
        let handler_ptr_for_callback = Rc::clone(&handler_ptr);
        let callback_finished_for_callback = Rc::clone(&callback_finished);
        let event = OnEvent::new(Event::HoverEnter, move || {
            unsafe { waterui_drop_on_event(handler_ptr_for_callback.get()) };
            callback_finished_for_callback.set(true);
        })
        .into_ffi();
        handler_ptr.set(event.handler);
        let env = crate::WuiEnv(waterui::Environment::new());

        unsafe { waterui_call_on_event(event.handler, &raw const env) };

        assert!(callback_finished.get());
    }
}
