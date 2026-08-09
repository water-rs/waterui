//! FFI bindings for the `SystemIcon` component.

use crate::{IntoFFI, WuiStr};
use waterui_icon::SystemIcon;

/// FFI representation of the `SystemIcon` component.
///
/// Native backends render this as platform-native icons when supported.
///
/// Apple currently maps this to SF Symbols. Other backends may omit `SystemIcon` support and should use
/// cross-platform icon-pack views instead.
#[repr(C)]
#[derive(Debug)]
pub struct WuiSystemIcon {
    /// The name of the system icon.
    pub name: WuiStr,
}

impl IntoFFI for SystemIcon {
    type FFI = WuiSystemIcon;
    fn into_ffi(self) -> Self::FFI {
        WuiSystemIcon {
            name: self.name.into_ffi(),
        }
    }
}

#[cfg(feature = "c-api")]
ffi_view!(SystemIcon, WuiSystemIcon, system_icon);
