use waterui::component::button::Action;

ffi_type!(waterui_action, Action, waterui_drop_action);

#[no_mangle]
pub unsafe extern "C" fn waterui_call_action(action: *const waterui_action) {
    (*action)();
}
