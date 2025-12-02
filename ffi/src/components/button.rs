use crate::WuiAnyView;
use crate::action::WuiAction;
use waterui::component::button::ButtonConfig;

into_ffi! {
    ButtonConfig,
    pub struct WuiButton {
        label: *mut WuiAnyView,
        action: *mut WuiAction,
    }
}

ffi_view!(ButtonConfig, WuiButton, button);
