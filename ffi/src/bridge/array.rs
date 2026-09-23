use core::{
    ops::{Deref, DerefMut},
    ptr::{NonNull, slice_from_raw_parts, slice_from_raw_parts_mut},
};

use alloc::{boxed::Box, vec::Vec};

use crate::{IntoFFI, IntoRust};

/// A type alias representing binary data as a byte array.
pub type WuiData = WuiArray<u8>;

/// A generic array structure for FFI, representing a contiguous sequence of elements.
/// `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of WuiArray is bound to the caller's scope),
/// or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
/// For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
/// We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
#[repr(C)]
pub struct WuiArray<T: 'static> {
    // Store the original boxed data for proper dropping
    data: NonNull<()>,
    vtable: WuiArrayVTable<T>,
}

#[repr(C)]
pub struct WuiArrayVTable<T> {
    drop: unsafe extern "C" fn(*mut ()),
    slice: unsafe extern "C" fn(*const ()) -> WuiArraySlice<T>,
}

#[repr(C)]
pub struct WuiArraySlice<T> {
    head: *mut T,
    len: usize,
}

impl<T> WuiArrayVTable<T> {
    pub const fn from_raw(
        drop: unsafe extern "C" fn(*mut ()),
        slice: unsafe extern "C" fn(*const ()) -> WuiArraySlice<T>,
    ) -> Self {
        Self { drop, slice }
    }

    pub const fn new<U>() -> Self
    where
        U: AsRef<[T]> + 'static,
    {
        unsafe extern "C" fn drop<U2>(data: *mut ()) {
            unsafe {
                let _: Box<U2> = Box::from_raw(data as *mut U2);
            }
        }

        unsafe extern "C" fn slice<U2, T2>(data: *const ()) -> WuiArraySlice<T2>
        where
            U2: AsRef<[T2]>,
        {
            unsafe {
                let slice = &*data.cast::<U2>();
                let s = slice.as_ref();
                let len = s.len();
                let head = s.as_ptr() as *mut T2;
                WuiArraySlice { head, len }
            }
        }

        Self {
            drop: drop::<U>,
            slice: slice::<U, T>,
        }
    }
}

impl<T> WuiArray<T> {
    /// Creates a new `WuiArray` from a raw pointer and length.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the pointer is valid and points to an array of the specified length.
    /// The memory must remain valid for the lifetime of the `WuiArray`.
    pub const unsafe fn from_raw(data: *mut (), vtable: WuiArrayVTable<T>) -> Self {
        Self {
            data: unsafe { NonNull::new_unchecked(data) },
            vtable,
        }
    }

    pub fn new<U>(array: U) -> Self
    where
        U: AsRef<[T]> + 'static,
    {
        let boxed = Box::new(array);
        let data = Box::into_raw(boxed) as *mut ();
        let vtable = WuiArrayVTable::new::<U>();
        unsafe { Self::from_raw(data, vtable) }
    }

    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_slice(&self) -> &[T] {
        unsafe {
            let slice = (self.vtable.slice)(self.data.as_ptr());
            &*slice_from_raw_parts(slice.head, slice.len)
        }
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        unsafe {
            let slice = (self.vtable.slice)(self.data.as_ptr());
            &mut *slice_from_raw_parts_mut(slice.head, slice.len)
        }
    }
}

impl<T: IntoRust + Default> IntoIterator for WuiArray<T> {
    type Item = T::Rust;
    type IntoIter = alloc::vec::IntoIter<T::Rust>;
    fn into_iter(self) -> Self::IntoIter {
        unsafe { self.into_rust().into_iter() }
    }
}

impl<T> Deref for WuiArray<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<T> DerefMut for WuiArray<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut_slice()
    }
}

impl<T> WuiArray<T> {
    /// Consumes the array, running the destructor on each element, and then freeing the array buffer.
    ///
    /// This is necessary because `WuiArray` cannot implement `Drop` directly (see notes below),
    /// but sometimes we own the array and its elements (e.g. passed from foreign code by value)
    /// and need to clean them up.
    pub fn consume_and_drop_elements(mut self) {
        unsafe {
            // 1. Drop each element
            let slice = self.as_mut_slice();
            for item in slice {
                core::ptr::drop_in_place(item);
            }

            // 2. Free the array buffer using the vtable
            (self.vtable.drop)(self.data.as_ptr());
        }
    }

    /// Consumes the array and frees the array buffer WITHOUT dropping elements.
    /// Use this if the elements are POD or ownership has been moved elsewhere.
    pub fn consume(self) {
        unsafe {
            (self.vtable.drop)(self.data.as_ptr());
        }
    }
}

impl<T> AsRef<[T]> for WuiArray<T> {
    fn as_ref(&self) -> &[T] {
        self
    }
}

impl<T: IntoFFI> IntoFFI for Vec<T>
where
    <T as IntoFFI>::FFI: 'static,
{
    type FFI = WuiArray<T::FFI>;

    fn into_ffi(self) -> Self::FFI {
        WuiArray::new(
            self.into_iter()
                .map(|item| item.into_ffi())
                .collect::<Vec<_>>(),
        )
    }
}

impl<T: Default + IntoRust> IntoRust for WuiArray<T> {
    type Rust = Vec<T::Rust>;
    unsafe fn into_rust(mut self) -> Self::Rust {
        self.deref_mut()
            .iter_mut()
            .map(|item| unsafe { core::mem::take(item).into_rust() })
            .collect::<Vec<_>>()
    }
}

// NOTE: WuiArray does NOT implement Drop because:
// 1. It crosses FFI boundaries where Rust's Drop semantics don't apply
// 2. C/C++ code may copy the struct by value (memcpy), creating shared ownership
// 3. Cleanup must be done explicitly by calling vtable.drop() at the right places
// 4. For Rust-created arrays passed to native code, native code is responsible for cleanup
// 5. For native-created arrays passed to Rust, the data is consumed by into_rust()
