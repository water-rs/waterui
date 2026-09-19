//! FFI bindings for the `Badge` component — a small indicator overlaid on
//! another view, typically a notification count on an icon or button.

use waterui::component::badge::BadgeConfig;

use crate::{IntoFFI, WuiAnyView, reactive::WuiComputed};

/// FFI representation of the `Badge` component.
#[repr(C)]
#[derive(Debug)]
pub struct WuiBadge {
    /// The numeric value shown inside the badge indicator.
    pub value: *mut WuiComputed<i32>,
    /// The view the badge is attached to.
    pub content: *mut WuiAnyView,
    /// The badge indicator color.
    pub color: *mut WuiComputed<waterui::Color>,
}

impl IntoFFI for BadgeConfig {
    type FFI = WuiBadge;

    fn into_ffi(self) -> Self::FFI {
        WuiBadge {
            value: self.value.into_ffi(),
            content: self.content.build().into_ffi(),
            color: self.color.into_ffi(),
        }
    }
}

ffi_view!(BadgeConfig, WuiBadge, badge);
