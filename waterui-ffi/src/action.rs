use waterui::component::button::Action;

use crate::{waterui_env, IntoRust};

ffi_type!(waterui_action, Action, waterui_drop_action);

#[no_mangle]
pub unsafe extern "C" fn waterui_call_action(action: *mut waterui_action, env: *mut waterui_env) {
    (*action)(env.into_rust());
}
