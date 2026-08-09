use waterui::AnyView;
use waterui_core::id::Id;

use crate::{IntoFFI, IntoRust, WuiAnyView, ffi_binding};

/// FFI-compatible representation of [`waterui_core::id::Id`].
#[repr(C)]
#[derive(Debug, Default)]
pub struct WuiId {
    /// The inner integer value of the ID.
    pub inner: i32,
}

impl IntoFFI for waterui_core::id::Id {
    type FFI = WuiId;
    fn into_ffi(self) -> Self::FFI {
        WuiId {
            inner: i32::from(self),
        }
    }
}

impl IntoRust for WuiId {
    type Rust = waterui_core::id::Id;
    unsafe fn into_rust(self) -> Self::Rust {
        waterui_core::id::Id::try_from(self.inner).expect("failed to convert id")
    }
}

into_ffi! {
    waterui_core::id::TaggedView<Id, AnyView>,
    pub struct WuiTaggedView {
        tag: WuiId,
        content: *mut WuiAnyView,
    }
}

ffi_binding!(Id, WuiId, id);
#[cfg(feature = "c-api")]
crate::ffi_watcher!(Id, WuiId, id);
