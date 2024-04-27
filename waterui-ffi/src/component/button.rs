use crate::{ffi_view, waterui_anyview, IntoFFI};

use waterui::component::button::RawButton;

#[repr(C)]
pub struct waterui_button {
    label: *mut waterui_anyview,
}

impl IntoFFI for RawButton {
    type FFI = waterui_button;

    fn into_ffi(self) -> Self::FFI {
        waterui_button {
            label: self._label.into_ffi(),
        }
    }
}

ffi_view!(
    RawButton,
    waterui_button,
    waterui_view_force_as_button,
    waterui_view_button_id
);
