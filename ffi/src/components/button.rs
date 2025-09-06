use crate::action::WuiAction;
use crate::{WuiAnyView, ffi_struct, ffi_view};
use waterui::component::Native;
use waterui::component::button::ButtonConfig;

/// C representation of a WaterUI button for FFI purposes.
#[derive(Debug)]
#[repr(C)]
pub struct WuiButton {
    /// Pointer to the button's label view
    pub label: *mut WuiAnyView,
    /// Pointer to the button's action handler
    pub action: *mut WuiAction,
}

ffi_struct!(ButtonConfig, WuiButton, label, action);
ffi_view!(
    Native<ButtonConfig>,
    WuiButton,
    waterui_button_id,
    waterui_force_as_button
);
