use alloc::boxed::Box;
use core::fmt;
use waterui::component::list::Move;
use waterui_core::Environment;
use waterui_core::handler::BoxedAction;

use crate::bridge::closure::RetainedCallback;
use crate::{IntoFFI, WuiEnv};

opaque!(WuiAction, RetainedCallback<BoxedAction<()>>, action);

impl IntoFFI for BoxedAction<()> {
    type FFI = *mut WuiAction;

    fn into_ffi(self) -> Self::FFI {
        RetainedCallback::new(self).into_ffi()
    }
}

/// Calls an action with the given environment.
///
/// # Safety
///
/// * `action` must be a valid pointer to a `waterui_action` struct.
/// * `env` must be a valid pointer to a `waterui_env` struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_action(action: *const WuiAction, env: *const WuiEnv) {
    // SAFETY: the caller contract requires `action` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let action = unsafe { crate::borrow_ffi(action) }.0.clone();
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) }.0.clone();
    action.call(|action| action(&env));
}

// ============================================================================
// Indexed Actions - for list delete callbacks
// ============================================================================

type IndexCallback = Box<dyn Fn(&Environment, usize)>;

/// Handler that takes an index parameter (used for delete callbacks).
pub struct IndexHandler(pub IndexCallback);

impl fmt::Debug for IndexHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The wrapped `IndexCallback` is a boxed closure with no Debug
        // representation, so only the handler's identity is reported.
        f.debug_struct("IndexHandler").finish_non_exhaustive()
    }
}

opaque!(WuiIndexAction, RetainedCallback<IndexHandler>, index_action);

impl crate::IntoNullableFFI for waterui::component::list::OnDelete {
    type FFI = *mut WuiIndexAction;

    fn into_ffi(self) -> Self::FFI {
        RetainedCallback::new(IndexHandler(self)).into_ffi()
    }

    fn null() -> Self::FFI {
        core::ptr::null_mut()
    }
}

/// Calls an index action with the given environment and index.
///
/// # Safety
///
/// * `action` must be a valid pointer to a `WuiIndexAction` struct.
/// * `env` must be a valid pointer to a `WuiEnv` struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_index_action(
    action: *const WuiIndexAction,
    env: *const WuiEnv,
    index: usize,
) {
    // SAFETY: the caller contract requires `action` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let action = unsafe { crate::borrow_ffi(action) }.0.clone();
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) }.0.clone();
    action.call(|action| (action.0)(&env, index));
}

// ============================================================================
// Move Actions - for list move callbacks
// ============================================================================

type MoveCallback = Box<dyn Fn(&Environment, Move)>;

/// Handler that takes a list move operation (used for move callbacks).
pub struct MoveHandler(pub MoveCallback);

impl fmt::Debug for MoveHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The wrapped `MoveCallback` is a boxed closure with no Debug
        // representation, so only the handler's identity is reported.
        f.debug_struct("MoveHandler").finish_non_exhaustive()
    }
}

opaque!(WuiMoveAction, RetainedCallback<MoveHandler>, move_action);

impl crate::IntoNullableFFI for waterui::component::list::OnMove {
    type FFI = *mut WuiMoveAction;

    fn into_ffi(self) -> Self::FFI {
        RetainedCallback::new(MoveHandler(self)).into_ffi()
    }

    fn null() -> Self::FFI {
        core::ptr::null_mut()
    }
}

/// Calls a move action with the given environment and from/to indices.
///
/// # Safety
///
/// * `action` must be a valid pointer to a `WuiMoveAction` struct.
/// * `env` must be a valid pointer to a `WuiEnv` struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_move_action(
    action: *const WuiMoveAction,
    env: *const WuiEnv,
    from_index: usize,
    to_index: usize,
) {
    // SAFETY: the caller contract requires `action` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let action = unsafe { crate::borrow_ffi(action) }.0.clone();
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) }.0.clone();
    action.call(|action| (action.0)(&env, Move::new(from_index, to_index)));
}

#[cfg(all(test, feature = "c-api"))]
mod tests {
    use alloc::rc::Rc;
    use alloc::sync::Arc;
    use core::cell::Cell;
    use core::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::WuiEnv;

    fn test_env() -> *mut WuiEnv {
        waterui::Environment::new().into_ffi()
    }

    #[test]
    fn call_action_executes_handler() {
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_action = Arc::clone(&hits);
        let action: BoxedAction<()> = Box::new(move |_| {
            hits_for_action.fetch_add(1, Ordering::SeqCst);
        });
        let action_ptr = action.into_ffi();
        let env_ptr = test_env();

        // SAFETY: both pointers are the live handles this test created above; the
        // action is invoked twice deliberately, which the contract allows.
        unsafe {
            waterui_call_action(action_ptr, env_ptr);
            waterui_call_action(action_ptr, env_ptr);
        }

        assert_eq!(hits.load(Ordering::SeqCst), 2);

        // SAFETY: both handles are still live and each is released once here.
        unsafe {
            waterui_drop_action(action_ptr);
            let _ = Box::from_raw(env_ptr.cast::<waterui::Environment>());
        }
    }

    #[test]
    fn action_survives_reentrant_native_drop_until_callback_returns() {
        let action_ptr = Rc::new(Cell::new(core::ptr::null_mut::<WuiAction>()));
        let callback_finished = Rc::new(Cell::new(false));
        let action_ptr_for_callback = Rc::clone(&action_ptr);
        let callback_finished_for_callback = Rc::clone(&callback_finished);
        let action: BoxedAction<()> = Box::new(move |_| {
            // SAFETY: the cell holds the action handle installed before this callback
            // runs, and the callback runs once.
            unsafe { waterui_drop_action(action_ptr_for_callback.get()) };
            callback_finished_for_callback.set(true);
        });
        let action = action.into_ffi();
        action_ptr.set(action);
        let env = test_env();

        // SAFETY: both are the live handles built above in this test.
        unsafe { waterui_call_action(action, env) };

        assert!(callback_finished.get());
        // SAFETY: `env` is the boxed environment this test leaked above, freed once.
        unsafe {
            let _ = Box::from_raw(env.cast::<waterui::Environment>());
        }
    }
}
