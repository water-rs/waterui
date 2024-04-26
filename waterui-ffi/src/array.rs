use core::{
    ops::Deref,
    ptr::{slice_from_raw_parts, slice_from_raw_parts_mut},
    str,
};

use alloc::{borrow::Cow, boxed::Box, string::String, vec::Vec};

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

impl<T> IntoFFI for Vec<T> {
    type FFI = waterui_array<T>;

    fn into_ffi(mut self) -> Self::FFI {
        let len = self.len();
        let head = self.as_mut_ptr();
        core::mem::forget(self);

        waterui_array { head, len }
    }
}

impl<T> IntoRust for waterui_array<T> {
    type Rust = Vec<T>;
    unsafe fn into_rust(self) -> Self::Rust {
        Box::from_raw(slice_from_raw_parts_mut(self.head, self.len)).into_vec()
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct waterui_str {
    head: *mut u8,
    len: usize,
}

impl Deref for waterui_str {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        unsafe { core::str::from_utf8_unchecked(&*slice_from_raw_parts(self.head, self.len)) }
    }
}

impl IntoFFI for String {
    type FFI = waterui_str;

    fn into_ffi(mut self) -> Self::FFI {
        let len = self.len();
        let head = self.as_mut_ptr();
        core::mem::forget(self);

        waterui_str { head, len }
    }
}

impl IntoFFI for Cow<'static, str> {
    type FFI = waterui_str;
    fn into_ffi(self) -> Self::FFI {
        self.into_owned().into_ffi()
    }
}

impl IntoRust for waterui_str {
    type Rust = String;
    unsafe fn into_rust(self) -> Self::Rust {
        String::from_utf8_unchecked(
            Box::from_raw(slice_from_raw_parts_mut(self.head, self.len)).into_vec(),
        )
    }
}
