use waterui_core::handler::BoxHandler;

use crate::waterui_env;

ffi_type!(waterui_action, BoxHandler<()>, waterui_drop_action);

#[no_mangle]
pub unsafe extern "C" fn waterui_call_action(action: *mut waterui_action, env: *const waterui_env) {
    (*action).handle(&*env);
}
