use crate::WuiLabel;
use crate::action::WuiAction;
use waterui::component::button::{ButtonConfig, ButtonStyle};

into_ffi! {ButtonStyle, non_exhaustive,
    pub enum WuiButtonStyle {
        Automatic,
        Plain,
        Link,
        Borderless,
        Bordered,
        BorderedProminent,
    }
}

/// FFI representation of the `Button` component.
#[repr(C)]
#[derive(Debug)]
pub struct WuiButton {
    /// Semantic label slot. Carries the visual view, accessibility text, and
    /// visual mode in a single struct — see [`WuiLabel`].
    pub label: WuiLabel,
    /// The action invoked when the button is activated.
    pub action: *mut WuiAction,
    /// The visual presentation style for the button.
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
