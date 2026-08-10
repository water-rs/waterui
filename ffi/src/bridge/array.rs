use core::{
    ops::{Deref, DerefMut},
    ptr::{NonNull, slice_from_raw_parts, slice_from_raw_parts_mut},
};

use alloc::{boxed::Box, vec::Vec};

use crate::{IntoFFI, IntoRust};

/// A type alias representing binary data as a byte array.
pub type WuiData = WuiArray<u8>;

/// A generic array structure for FFI, representing a contiguous sequence of elements.
///
/// `WuiArray` can represent multiple types of arrays, for instance, a `&[T]` (in this case, the lifetime of `WuiArray` is bound to the caller's scope),
/// or a value type having a static lifetime like `Vec<T>`, `Box<[T]>`, `Bytes`, or even a foreign allocated array.
/// For a value type, `WuiArray` contains a destructor function pointer to free the array buffer, whatever it is allocated by Rust side or foreign side.
/// We assume `T` does not contain any non-trivial drop logic, and `WuiArray` will not call `drop` on each element when it is dropped.
#[repr(C)]
#[derive(Debug)]
pub struct WuiArray<T: 'static> {
    // Store the original boxed data for proper dropping
    data: NonNull<()>,
    vtable: WuiArrayVTable<T>,
}

/// The pair of function pointers `WuiArray` uses to view and free its backing storage.
///
/// `drop` releases the boxed container referenced by [`WuiArray::data`](WuiArray),
/// and `slice` exposes that container's elements as a raw [`WuiArraySlice`].
#[repr(C)]
#[derive(Debug)]
pub struct WuiArrayVTable<T> {
    drop: unsafe extern "C" fn(*mut ()),
    slice: unsafe extern "C" fn(*const ()) -> WuiArraySlice<T>,
}

/// A raw, borrowed view of a `WuiArray`'s elements as a pointer and length.
#[repr(C)]
#[derive(Debug)]
pub struct WuiArraySlice<T> {
    head: *mut T,
    len: usize,
}

impl<T> Default for WuiArray<T> {
    fn default() -> Self {
        Self::new(Vec::<T>::new())
    }
}

impl<T> WuiArrayVTable<T> {
    /// Assembles a `WuiArrayVTable` from an explicit drop function and slice accessor.
    ///
    /// Use this when the backing storage's drop/slice behavior is supplied directly
    /// (for instance, by native code) rather than generated for a known Rust type `U`.
    pub const fn from_raw(
        drop: unsafe extern "C" fn(*mut ()),
        slice: unsafe extern "C" fn(*const ()) -> WuiArraySlice<T>,
    ) -> Self {
        Self { drop, slice }
    }

    /// Builds a vtable whose drop/slice functions are specialized for the concrete
    /// backing type `U`, so the correct destructor and element view are used
    /// regardless of what `T` the resulting `WuiArray<T>` is declared to hold.
    #[must_use]
    pub const fn new<U>() -> Self
    where
        U: AsRef<[T]> + 'static,
    {
        unsafe extern "C" fn drop<U2>(data: *mut ()) {
            // SAFETY: this vtable entry is only reachable through a `WuiArray` built by
            // `from_boxed`, which boxed a `U` and paired it with `drop::<U>`, so `U2` is
            // that same `U` and `data` is its `Box::into_raw` pointer. `consume` invokes
            // it once and then gives up the pointer.
            unsafe {
                let _: Box<U2> = Box::from_raw(data.cast::<U2>());
            }
        }

        unsafe extern "C" fn slice<U2, T2>(data: *const ()) -> WuiArraySlice<T2>
        where
            U2: AsRef<[T2]>,
        {
            // SAFETY: as for `drop`, `data` is the live `Box<U2>` allocation this
            // vtable was built for. The borrow lasts only for this call, which the
            // callers (`as_slice` / `as_mut_slice`) hold behind a borrow of the array.
            unsafe {
                let slice = &*data.cast::<U2>();
                let s = slice.as_ref();
                let len = s.len();
                let head = s.as_ptr().cast_mut();
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
            // SAFETY: the caller contract above requires `data` to be a valid pointer,
            // and a valid pointer is never null.
            data: unsafe { NonNull::new_unchecked(data) },
            vtable,
        }
    }

    /// Creates a new `WuiArray` that takes ownership of `array` and exposes its
    /// elements through a vtable generated for `U`'s concrete type.
    pub fn new<U>(array: U) -> Self
    where
        U: AsRef<[T]> + 'static,
    {
        let boxed = Box::new(array);
        let data = Box::into_raw(boxed).cast::<()>();
        let vtable = WuiArrayVTable::new::<U>();
        // SAFETY: `data` is the pointer `Box::into_raw` just produced for this `U`, and
        // `vtable` is the one built for that same `U`, so the array owns a live buffer
        // its vtable can describe and free.
        unsafe { Self::from_raw(data, vtable) }
    }

    /// Returns the number of elements in the array.
    #[must_use]
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Returns `true` if the array has no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrows the array's elements as a Rust slice.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: the vtable reports the head and length of the buffer this array owns,
        // which stays live and unmoved for as long as the array does; the returned slice
        // borrows from `&self`, so it cannot outlive it.
        unsafe {
            let slice = (self.vtable.slice)(self.data.as_ptr());
            &*slice_from_raw_parts(slice.head, slice.len)
        }
    }

    /// Mutably borrows the array's elements as a Rust slice.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: as for `as_slice`, and the `&mut self` borrow makes this the only live
        // reference to the buffer.
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
        // SAFETY: `into_rust` requires ownership of a live array, which `self` is.
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
    /// Consumes the array and frees the array buffer WITHOUT dropping elements.
    /// Use this if the elements are POD or ownership has been moved elsewhere.
    pub fn consume(self) {
        // SAFETY: the vtable's `drop` is paired with the allocation `data` points at,
        // and taking `self` by value means it runs exactly once and the pointer is never
        // observed again.
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
                .map(super::super::IntoFFI::into_ffi)
                .collect::<Vec<_>>(),
        )
    }
}

impl<T: Default + IntoRust> IntoRust for WuiArray<T> {
    type Rust = Vec<T::Rust>;
    unsafe fn into_rust(mut self) -> Self::Rust {
        let values = self
            .deref_mut()
            .iter_mut()
            // SAFETY: each slot holds a live `T` this array owns, and `mem::take`
            // replaces it with `T::default()` so the buffer stays valid for the
            // `consume` below — no element is converted twice.
            .map(|item| unsafe { core::mem::take(item).into_rust() })
            .collect::<Vec<_>>();
        self.consume();
        values
    }
}

// NOTE: WuiArray does NOT implement Drop because:
// 1. It crosses FFI boundaries where Rust's Drop semantics don't apply
// 2. C/C++ code may copy the struct by value (memcpy), creating shared ownership
// 3. Cleanup must be done explicitly by calling vtable.drop() at the right places
// 4. For Rust-created arrays passed to native code, native code is responsible for cleanup
// 5. For native-created arrays passed to Rust, the data is consumed by into_rust()
