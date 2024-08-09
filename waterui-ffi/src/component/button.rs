use crate::{action::waterui_action, waterui_anyview, IntoFFI};
use waterui::component::button::ButtonConfig;

#[repr(C)]
pub struct waterui_button {
    label: *mut waterui_anyview,
    action: *mut waterui_action,
}

impl IntoFFI for ButtonConfig {
    type FFI = waterui_button;

    fn into_ffi(self) -> Self::FFI {
        waterui_button {
            label: self.label.into_ffi(),
            action: self.action.into_ffi(),
        }
    }
}

native_view!(
    ButtonConfig,
    waterui_button,
    waterui_view_force_as_button,
    waterui_view_button_id
);
