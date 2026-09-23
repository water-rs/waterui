//! Function callback wrappers for C-API interoperability.
//!
//! This module provides structures and implementations for safely wrapping Rust
//! functions to be called from C and vice versa, with proper memory management.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;

/// Cloneable ownership for a repeatable callback invoked across the native ABI.
///
/// Callers clone this owner before invoking the callback so synchronously
/// disposing the native view cannot free the callback while it is executing.
#[doc(hidden)]
pub struct RetainedCallback<T>(Rc<RefCell<T>>);

impl<T> RetainedCallback<T> {
    pub(crate) fn new(callback: T) -> Self {
        Self(Rc::new(RefCell::new(callback)))
    }

    pub(crate) fn call<R>(&self, call: impl FnOnce(&mut T) -> R) -> R {
        call(&mut self.0.borrow_mut())
    }
}

impl<T> Clone for RetainedCallback<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

/// Native-owned callback context retained by a Rust environment service.
///
/// The owner invokes `drop` exactly once when the service leaves its
/// [`Environment`](waterui_core::Environment).
pub(crate) struct ForeignCallbackContext {
    data: *mut (),
    drop: unsafe extern "C" fn(*mut ()),
}

impl ForeignCallbackContext {
    /// Takes ownership of a native callback context.
    ///
    /// # Safety
    ///
    /// `data` must remain valid until `drop` releases it exactly once.
    pub(crate) unsafe fn new(data: *mut (), drop: unsafe extern "C" fn(*mut ())) -> Self {
        Self { data, drop }
    }

    pub(crate) const fn data(&self) -> *mut () {
        self.data
    }
}

impl Drop for ForeignCallbackContext {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.data) };
    }
}

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

impl<T, F> From<F> for WuiFn<T>
where
    F: Fn(T),
    T: 'static,
{
    fn from(value: F) -> Self {
        unsafe {
            let data = Box::into_raw(Box::new(Rc::new(value))) as *mut ();

            unsafe extern "C" fn call<F2, T2>(data: *const (), value: T2)
            where
                F2: Fn(T2),
            {
                let callback = unsafe { Rc::clone(&*(data as *const Rc<F2>)) };
                callback(value);
            }
            unsafe extern "C" fn drop<F2, T2>(data: *mut ())
            where
                F2: Fn(T2),
            {
                unsafe {
                    let _ = Box::from_raw(data as *mut Rc<F2>);
                }
            }
            Self::new(data, call::<F, T>, drop::<F, T>)
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::cell::Cell;
    use core::mem::ManuallyDrop;

    use super::*;

    #[test]
    fn callback_survives_reentrant_owner_drop_until_call_returns() {
        type DropCallback = unsafe extern "C" fn(*mut ());

        let data = Rc::new(Cell::new(core::ptr::null_mut()));
        let drop_callback = Rc::new(Cell::new(None::<DropCallback>));
        let callback_finished = Rc::new(Cell::new(false));
        let data_for_callback = Rc::clone(&data);
        let drop_for_callback = Rc::clone(&drop_callback);
        let callback_finished_for_callback = Rc::clone(&callback_finished);
        let callback = ManuallyDrop::new(WuiFn::from(move |()| {
            unsafe {
                drop_for_callback
                    .get()
                    .expect("drop callback was not installed")(
                    data_for_callback.get()
                );
            }
            callback_finished_for_callback.set(true);
        }));
        data.set(callback.data);
        drop_callback.set(Some(callback.drop));

        unsafe { (callback.call)(callback.data, ()) };

        assert!(callback_finished.get());
    }
}
