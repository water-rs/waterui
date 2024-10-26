use core::mem::transmute;

use waterui::{AnyView, Environment};

use crate::{IntoFFI, IntoRust};

#[repr(C)]
pub struct waterui_type_id {
    inner: [u64; 2],
}

impl IntoFFI for core::any::TypeId {
    type FFI = waterui_type_id;
    fn into_ffi(self) -> Self::FFI {
        unsafe {
            waterui_type_id {
                inner: transmute::<core::any::TypeId, [u64; 2]>(self),
            }
        }
    }
}

impl IntoRust for waterui_type_id {
    type Rust = core::any::TypeId;
    unsafe fn into_rust(self) -> Self::Rust {
        unsafe { transmute(self.inner) }
    }
}
ffi_type!(waterui_anyview, AnyView, waterui_drop_anyview);
ffi_type!(waterui_env, Environment, waterui_drop_env);

#[no_mangle]
unsafe extern "C" fn waterui_clone_env(env: *const waterui_env) -> *mut waterui_env {
    (*env).clone().into_ffi()
}
