use waterui::core::handler::BoxHandler;

use crate::waterui_env;

ffi_type!(WuiAction, BoxHandler<()>, waterui_drop_action);

/// Calls an action with the given environment.
///
/// # Safety
///
/// * `action` must be a valid pointer to a `waterui_action` struct.
/// * `env` must be a valid pointer to a `waterui_env` struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_action(action: *mut WuiAction, env: *const waterui_env) {
    unsafe {
        (*action).handle(&*env);
    }
}
