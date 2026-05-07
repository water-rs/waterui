use crate::WuiLabel;
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

#[repr(C)]
pub struct WuiButton {
    /// Semantic label slot. Carries the visual view, accessibility text, and
    /// visual mode in a single struct — see [`WuiLabel`].
    pub label: WuiLabel,
    pub action: *mut WuiAction,
    pub style: WuiButtonStyle,
}

impl crate::IntoFFI for ButtonConfig {
    type FFI = WuiButton;

    fn into_ffi(self) -> Self::FFI {
        WuiButton {
            label: self.label.into_ffi(),
            action: self.action.into_ffi(),
            style: self.style.into_ffi(),
        }
    }
}

ffi_view!(ButtonConfig, WuiButton, button);
