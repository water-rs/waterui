use alloc::boxed::Box;
use waterui::component::list::Move;
use waterui_core::Environment;
use waterui_core::handler::BoxedAction;

use crate::{IntoFFI, WuiEnv};

opaque!(WuiAction, BoxedAction<()>, action);

/// Calls an action with the given environment.
///
/// # Safety
///
/// * `action` must be a valid pointer to a `waterui_action` struct.
/// * `env` must be a valid pointer to a `waterui_env` struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_action(action: *mut WuiAction, env: *const WuiEnv) {
    let action = unsafe { crate::expect_non_null_mut(action, "waterui_call_action", "action") };
    let env = unsafe { crate::expect_non_null(env, "waterui_call_action", "env") };
    let _ = crate::ffi_boundary("waterui_call_action", || {
        (action.0)(env);
    });
}

// ============================================================================
// Indexed Actions - for list delete callbacks
// ============================================================================

/// Handler that takes an index parameter (used for delete callbacks).
pub struct IndexHandler(pub Box<dyn Fn(&Environment, usize)>);

opaque!(WuiIndexAction, IndexHandler, index_action);

impl crate::IntoNullableFFI for waterui::component::list::OnDelete {
    type FFI = *mut WuiIndexAction;

    fn into_ffi(self) -> Self::FFI {
        IndexHandler(self).into_ffi()
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
    action: *mut WuiIndexAction,
    env: *const WuiEnv,
    index: usize,
) {
    let action =
        unsafe { crate::expect_non_null_mut(action, "waterui_call_index_action", "action") };
    let env = unsafe { crate::expect_non_null(env, "waterui_call_index_action", "env") };
    let _ = crate::ffi_boundary("waterui_call_index_action", || {
        (action.0.0)(env, index);
    });
}

// ============================================================================
// Move Actions - for list move callbacks
// ============================================================================

/// Handler that takes a list move operation (used for move callbacks).
pub struct MoveHandler(pub Box<dyn Fn(&Environment, Move)>);

opaque!(WuiMoveAction, MoveHandler, move_action);

impl crate::IntoNullableFFI for waterui::component::list::OnMove {
    type FFI = *mut WuiMoveAction;

    fn into_ffi(self) -> Self::FFI {
        MoveHandler(self).into_ffi()
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
    action: *mut WuiMoveAction,
    env: *const WuiEnv,
    from_index: usize,
    to_index: usize,
) {
    let action =
        unsafe { crate::expect_non_null_mut(action, "waterui_call_move_action", "action") };
    let env = unsafe { crate::expect_non_null(env, "waterui_call_move_action", "env") };
    let _ = crate::ffi_boundary("waterui_call_move_action", || {
        (action.0.0)(env, Move::new(from_index, to_index));
    });
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
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

        unsafe {
            waterui_call_action(action_ptr, env_ptr);
            waterui_call_action(action_ptr, env_ptr);
        }

        assert_eq!(hits.load(Ordering::SeqCst), 2);

        unsafe {
            waterui_drop_action(action_ptr);
            let _ = Box::from_raw(env_ptr as *mut waterui::Environment);
        }
    }
}
