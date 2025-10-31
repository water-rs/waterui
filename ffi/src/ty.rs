use super::{IntoFFI, IntoRust};

#[repr(C)]
pub struct WuiId {
    inner: i32,
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
