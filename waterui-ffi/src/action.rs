use waterui::component::button::BoxAction;

use crate::waterui_env;

ffi_type!(waterui_action, BoxAction, waterui_drop_action);

#[no_mangle]
pub unsafe extern "C" fn waterui_call_action(
    action: *const waterui_action,
    env: *const waterui_env,
) {
    (*action).call_action(&*env);
}
