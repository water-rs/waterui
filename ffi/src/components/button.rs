use crate::action::WuiAction;
use crate::{WuiAnyView, ffi_view};
use waterui::component::button::ButtonConfig;
use waterui_core::Native;

into_ffi! {
    ButtonConfig,
    pub struct WuiButton {
        label: *mut WuiAnyView,
        action: *mut WuiAction,
    }
}

ffi_view!(Native<ButtonConfig>, WuiButton, button);
