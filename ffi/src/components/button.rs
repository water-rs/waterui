use core::ptr::null_mut;

use crate::WuiAnyView;
use crate::action::WuiAction;
use crate::reactive::WuiComputed;
use waterui::component::button::{ButtonConfig, ButtonStyle};
use waterui_text::styled::StyledStr;

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
    pub label: *mut WuiAnyView,
    pub action: *mut WuiAction,
    pub style: WuiButtonStyle,
    pub accessibility_label: *mut WuiComputed<StyledStr>,
}

impl crate::IntoFFI for ButtonConfig {
    type FFI = WuiButton;

    fn into_ffi(self) -> Self::FFI {
        WuiButton {
            label: self.label.into_ffi(),
            action: self.action.into_ffi(),
            style: self.style.into_ffi(),
            accessibility_label: self
                .accessibility_label
                .map_or_else(null_mut, crate::IntoFFI::into_ffi),
        }
    }
}

ffi_view!(ButtonConfig, WuiButton, button);
