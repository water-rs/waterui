use crate::WuiAnyView;
use crate::action::WuiAction;
use waterui::component::button::{ButtonConfig, ButtonStyle};

into_ffi! {ButtonStyle, Automatic,
    pub enum WuiButtonStyle {
        Automatic,
        Plain,
        Link,
        Borderless,
        Bordered,
        BorderedProminent,
    }
}

into_ffi! {
    ButtonConfig,
    pub struct WuiButton {
        label: *mut WuiAnyView,
        action: *mut WuiAction,
        style: WuiButtonStyle,
    }
}

ffi_view!(ButtonConfig, WuiButton, button);
