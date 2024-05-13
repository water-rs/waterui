use crate::{action::waterui_action, ffi_view, waterui_anyview, IntoFFI};

use waterui::component::Button;

#[repr(C)]
pub struct waterui_button {
    label: *mut waterui_anyview,
    action: *mut waterui_action,
}

impl IntoFFI for Button {
    type FFI = waterui_button;

    fn into_ffi(self) -> Self::FFI {
        waterui_button {
            label: self._label.into_ffi(),
            action: self._action.into_ffi(),
        }
    }
}

ffi_view!(
    Button,
    waterui_button,
    waterui_view_force_as_button,
    waterui_view_button_id
);
