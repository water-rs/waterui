use crate::{binding::waterui_binding_bool, waterui_anyview, IntoFFI};

use waterui::component::toggle::ToggleConfig;

#[repr(C)]
pub struct waterui_toggle {
    label: *mut waterui_anyview,
    toggle: *const waterui_binding_bool,
}

impl IntoFFI for ToggleConfig {
    type FFI = waterui_toggle;
    fn into_ffi(self) -> Self::FFI {
        waterui_toggle {
            label: self.label.into_ffi(),
            toggle: self.toggle.into_ffi(),
        }
    }
}

native_view!(
    ToggleConfig,
    waterui_toggle,
    waterui_view_force_as_toggle,
    waterui_view_toggle_id
);
