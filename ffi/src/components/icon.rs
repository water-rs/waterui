//! FFI bindings for the SystemIcon component.

use crate::{IntoFFI, WuiStr};
use waterui_icon::SystemIcon;

/// FFI representation of the SystemIcon component.
///
/// Native backends render this as platform-native icons:
/// - Apple: SF Symbols
/// - Android: Material Icons (with placeholder fallback)
#[repr(C)]
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

ffi_view!(SystemIcon, WuiSystemIcon, system_icon);
