use alloc::boxed::Box;
use waterui_core::handler::BoxHandler;
use waterui_core::Environment;

use crate::{IntoFFI, WuiEnv};

opaque!(WuiAction, BoxHandler<()>, action);

/// Calls an action with the given environment.
///
/// # Safety
///
/// * `action` must be a valid pointer to a `waterui_action` struct.
/// * `env` must be a valid pointer to a `waterui_env` struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_action(action: *mut WuiAction, env: *const WuiEnv) {
    unsafe {
        (*action).handle(&*env);
    }
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
    unsafe {
        ((*action).0.0)(&*env, index);
    }
}

// ============================================================================
// Move Actions - for list move callbacks
// ============================================================================

/// Handler that takes from/to indices (used for move callbacks).
pub struct MoveHandler(pub Box<dyn Fn(&Environment, usize, usize)>);

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
    unsafe {
        ((*action).0.0)(&*env, from_index, to_index);
    }
}
