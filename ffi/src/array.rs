use core::{
    mem::ManuallyDrop,
    ops::Deref,
    ptr::{slice_from_raw_parts, slice_from_raw_parts_mut},
};

use alloc::{boxed::Box, vec::Vec};

use crate::WuiAnyView;

use super::{IntoFFI, IntoRust};

/// A type alias representing binary data as a byte array.
pub type WuiData = WuiArray<u8>;

/// A C-compatible array structure that wraps a pointer and length.
///
/// This type is used as an FFI-compatible representation of Rust collections.
#[repr(C)]
pub struct WuiArray<T> {
    head: *mut T,
    len: usize,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_free_anyview_array_without_free_elements(
    arr: WuiArray<*mut WuiAnyView>,
) {
    unsafe {
        let mut arr = arr.into_rust();
        arr.set_len(0); // Prevent dropping the elements
    }
}

impl<T> Deref for WuiArray<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        unsafe { &*slice_from_raw_parts(self.head, self.len) }
    }
}

impl<T: IntoFFI> IntoFFI for Vec<T> {
    type FFI = WuiArray<T::FFI>;

    fn into_ffi(self) -> Self::FFI {
        let boxed = self
            .into_iter()
            .map(IntoFFI::into_ffi)
            .collect::<Vec<_>>()
            .into_boxed_slice();

        let mut this = ManuallyDrop::new(boxed);
        let len = this.len();
        let head = this.as_mut_ptr();

        WuiArray { head, len }
    }
}

impl<T: IntoRust> IntoRust for WuiArray<T> {
    type Rust = Vec<T::Rust>;
    unsafe fn into_rust(self) -> Self::Rust {
        unsafe {
            let vec = Box::from_raw(slice_from_raw_parts_mut(self.head, self.len)).into_vec();
            vec.into_iter().map(|v| v.into_rust()).collect()
        }
    }
}
