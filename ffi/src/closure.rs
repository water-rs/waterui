//! Function callback wrappers for C-API interoperability.
//!
//! This module provides structures and implementations for safely wrapping Rust
//! functions to be called from C and vice versa, with proper memory management.

use core::mem::ManuallyDrop;

use alloc::boxed::Box;

use super::IntoRust;

/// A C-compatible function wrapper that can be called multiple times.
///
/// This structure wraps a Rust `Fn` closure to allow it to be passed across
/// the FFI boundary while maintaining proper memory management.
#[repr(C)]
pub struct WuiFn<T> {
    data: *mut (),
    call: unsafe extern "C" fn(*const (), T),
    drop: unsafe extern "C" fn(*mut ()),
}

impl<T: 'static> IntoRust for WuiFn<T> {
    type Rust = Box<dyn Fn(T)>;
    unsafe fn into_rust(self) -> Self::Rust {
        let this = ManuallyDrop::new(self);
        unsafe { Box::new(move |v| (this.call)(this.data, v)) }
    }
}

impl<T: 'static> WuiFn<T> {
    /// Creates a new `WuiFn` with the given data pointer and functions.
    ///
    /// # Safety
    ///
    /// - `data` must be a valid pointer to appropriate data for the provided call and drop functions.
    /// - `call` must be a valid function that can safely be called with the provided data pointer.
    /// - `drop` must be a valid function that can safely free or clean up the provided data pointer.
    pub unsafe fn new(
        data: *mut (),
        call: unsafe extern "C" fn(*const (), T),
        drop: unsafe extern "C" fn(*mut ()),
    ) -> Self {
        Self { data, call, drop }
    }
    pub fn call(&self, value: T) {
        unsafe { (self.call)(self.data, value) }
    }
}

impl<T> Drop for WuiFn<T> {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.data) }
    }
}

/// A C-compatible function wrapper that can be called only once.
///
/// This structure wraps a Rust `FnOnce` closure to allow it to be passed across
/// the FFI boundary while maintaining proper memory management.
#[repr(C)]
pub struct WuiFnOnce<T> {
    data: *mut (),
    call: unsafe extern "C" fn(*mut (), T),
}

impl<T: 'static> IntoRust for WuiFnOnce<T> {
    type Rust = Box<dyn FnOnce(T)>;
    unsafe fn into_rust(self) -> Self::Rust {
        Box::new(move |v| self.call(v))
    }
}

impl<T> WuiFnOnce<T> {
    /// Creates a new `WuiFnOnce` with the given data pointer and call function.
    ///
    /// # Safety
    ///
    /// - `data` must be a valid pointer to appropriate data for the provided call function.
    /// - `call` must be a valid function that can safely be called with the provided data pointer.
    pub unsafe fn new(data: *mut (), call: unsafe extern "C" fn(*mut (), T)) -> Self {
        Self { data, call }
    }

    pub fn call(self, value: T) {
        unsafe { (self.call)(self.data, value) }
    }
}

impl<T, F> From<F> for WuiFn<T>
where
    F: Fn(T),
    T: 'static,
{
    fn from(value: F) -> Self {
        unsafe {
            let data = Box::into_raw(Box::new(value)) as *mut ();

            unsafe extern "C" fn call<F2, T2>(data: *const (), value: T2)
            where
                F2: Fn(T2),
            {
                unsafe {
                    let f: &F2 = &*(data as *const F2);
                    f(value);
                }
            }
            unsafe extern "C" fn drop<F2, T2>(data: *mut ())
            where
                F2: Fn(T2),
            {
                unsafe {
                    let _ = Box::from_raw(data as *mut F2);
                }
            }
            Self::new(data, call::<F, T>, drop::<F, T>)
        }
    }
}

impl<T, F> From<F> for WuiFnOnce<T>
where
    F: FnOnce(T),
{
    fn from(value: F) -> Self {
        unsafe {
            let data = Box::into_raw(Box::new(value)) as *mut ();

            unsafe extern "C" fn call<F2, T2>(data: *mut (), value: T2)
            where
                F2: FnOnce(T2),
            {
                unsafe {
                    let f = Box::from_raw(data as *mut F2);
                    f(value);
                }
            }
            Self::new(data, call::<F, T>)
        }
    }
}
