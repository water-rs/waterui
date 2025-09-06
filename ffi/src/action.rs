use waterui::core::handler::BoxHandler;

use crate::WuiEnv;

ffi_type!(WuiAction, BoxHandler<()>, waterui_drop_action);

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
