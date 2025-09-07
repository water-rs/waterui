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
///
/// Warning: `T` must be FFI-safe. Using non-FFI-safe types may lead to undefined behavior.
#[repr(C)]
pub struct WuiArray<T> {
    head: *mut T,
    len: usize,
}

/// Frees a WuiArray without dropping its elements.
///
/// # Safety
///
/// The caller must ensure that:
/// - `arr` is a valid WuiArray that was previously created by Rust code
/// - The array elements are handled separately and not accessed after this call
/// - This function is only called once per array
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_free_anyview_array_without_free_elements(
    arr: WuiArray<*mut WuiAnyView>,
) {
    // Free only the array buffer allocated on the Rust side, without touching
    // element pointers. The element pointers remain owned by the caller.
    unsafe {
        let _ = Box::from_raw(slice_from_raw_parts_mut(arr.head, arr.len));
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
