use core::{
    mem::ManuallyDrop,
    ops::Deref,
    ptr::{slice_from_raw_parts, slice_from_raw_parts_mut},
};

use ::waterui_str::Str;
use alloc::{boxed::Box, vec::Vec};

use crate::{IntoFFI, IntoRust};

pub type waterui_data = waterui_array<u8>;

#[repr(C)]
pub struct waterui_array<T> {
    head: *mut T,
    len: usize,
}

impl<T> Deref for waterui_array<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        unsafe { &*slice_from_raw_parts(self.head, self.len) }
    }
}

impl<T: IntoFFI> IntoFFI for Vec<T> {
    type FFI = waterui_array<T::FFI>;

    fn into_ffi(self) -> Self::FFI {
        let boxed = self
            .into_iter()
            .map(IntoFFI::into_ffi)
            .collect::<Vec<_>>()
            .into_boxed_slice();

        let mut this = ManuallyDrop::new(boxed);
        let len = this.len();
        let head = this.as_mut_ptr();

        waterui_array { head, len }
    }
}

impl<T: IntoRust> IntoRust for waterui_array<T> {
    type Rust = Vec<T::Rust>;
    unsafe fn into_rust(self) -> Self::Rust {
        let vec = Box::from_raw(slice_from_raw_parts_mut(self.head, self.len)).into_vec();
        vec.into_iter().map(|v| v.into_rust()).collect()
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct waterui_str {
    ptr: *const (),
    len: usize,
}

#[no_mangle]
unsafe extern "C" fn waterui_str_get_head(s: waterui_str) -> *const u8 {
    let str = ManuallyDrop::new(Str::from_raw_parts(s.ptr, s.len));
    str.as_ptr()
}

impl IntoFFI for Str {
    type FFI = waterui_str;
    fn into_ffi(self) -> Self::FFI {
        let (ptr, len) = self.into_raw_parts();
        Self::FFI { ptr, len }
    }
}

impl IntoRust for waterui_str {
    type Rust = Str;
    unsafe fn into_rust(self) -> Self::Rust {
        Str::from_raw_parts(self.ptr, self.len)
    }
}

#[no_mangle]
pub unsafe extern "C" fn waterui_new_str(head: *const u8, len: usize) -> waterui_str {
    let vec = core::slice::from_raw_parts(head, len).to_vec();
    Str::from_utf8_unchecked(vec).into_ffi()
}

#[no_mangle]
pub unsafe extern "C" fn waterui_free_str(s: waterui_str) {
    let _ = s.into_rust();
}

#[no_mangle]
pub unsafe extern "C" fn waterui_free_array(ptr: *mut u8, size: usize) {
    let _ = Vec::from_raw_parts(ptr, 0, size);
}
