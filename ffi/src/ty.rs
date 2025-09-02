use core::mem::transmute;

use waterui::core::handler::BoxHandler;

use crate::OpaqueType;

use super::{IntoFFI, IntoRust};

/// A C-compatible representation of Rust's `core::any::TypeId`.
///
/// This struct is used for passing TypeId across FFI boundaries.
#[repr(C)]
pub struct WuiTypeId {
    inner: [u64; 2],
}

impl IntoFFI for core::any::TypeId {
    type FFI = WuiTypeId;
    fn into_ffi(self) -> Self::FFI {
        unsafe {
            WuiTypeId {
                inner: transmute::<core::any::TypeId, [u64; 2]>(self),
            }
        }
    }
}

impl IntoRust for WuiTypeId {
    type Rust = core::any::TypeId;
    unsafe fn into_rust(self) -> Self::Rust {
        unsafe { transmute(self.inner) }
    }
}

impl OpaqueType for waterui::AnyView {}
