use crate::{binding::waterui_binding_bool, ffi_view, waterui_anyview, IntoFFI};

use waterui::component::toggle::{RawToggle, ToggleStyle};

#[repr(C)]
pub enum waterui_style_toggle {
    Default,
    CheckBox,
    Switch,
}

impl IntoFFI for ToggleStyle {
    type FFI = waterui_style_toggle;
    fn into_ffi(self) -> Self::FFI {
        match self {
            ToggleStyle::CheckBox => waterui_style_toggle::CheckBox,
            ToggleStyle::Switch => waterui_style_toggle::Switch,
            _ => waterui_style_toggle::Default,
        }
    }
}

#[repr(C)]
pub struct waterui_toggle {
    label: *mut waterui_anyview,
    toggle: *const waterui_binding_bool,
    style: waterui_style_toggle,
}

impl IntoFFI for RawToggle {
    type FFI = waterui_toggle;
    fn into_ffi(self) -> Self::FFI {
        waterui_toggle {
            label: self._label.into_ffi(),
            toggle: self._toggle.into_ffi(),
            style: self._style.into_ffi(),
        }
    }
}

ffi_view!(
    RawToggle,
    waterui_toggle,
    waterui_view_force_as_toggle,
    waterui_view_toggle_id
);
