//! The `main_value` module provides safe handling of values on the main thread.
//!
//! This module allows you to store values that must be accessed or dropped on the main thread,
//! providing safe cross-thread access patterns through asynchronous APIs.
#![allow(clippy::non_send_fields_in_send_ty)]
use core::{mem::ManuallyDrop, ptr::from_ref};

use crate::{Task, exec_main};

/// A container for values that must be accessed and dropped on the main thread.
///
/// `MainValue<T>` ensures that the wrapped value is only ever accessed on the main thread
/// and properly dropped there as well, regardless of where the `MainValue` itself is dropped.
///
/// # Safety
///
/// This type implements `Send` and `Sync` unconditionally to allow it to be moved between threads,
/// but it ensures all actual operations on the inner value happen on the main thread.
#[derive(Debug)]
pub struct MainValue<T>(ManuallyDrop<T>);

/// Private wrapper to allow safe cross-thread references.
///
/// This wrapper is used internally to safely transport references across thread boundaries.
struct Wrapper<T>(T);

// Safe because we only use this wrapper to move references to the main thread
// and all actual operations on the referenced data occur only on the main thread.
unsafe impl<T> Send for Wrapper<T> {}

impl<T> Drop for MainValue<T> {
    /// Drops the inner value on the main thread.
    ///
    /// This ensures that even if `MainValue` is dropped on a non-main thread,
    /// the wrapped value itself is always dropped on the main thread.
    fn drop(&mut self) {
        let this = unsafe { Wrapper(ManuallyDrop::take(&mut self.0)) };
        exec_main(move || {
            let _ = this.0;
        });
    }
}

impl<T: Clone + 'static> MainValue<T> {
    /// Clones the inner value on the main thread and returns a new `MainValue`.
    ///
    /// # Returns
    ///
    /// A new `MainValue` containing a clone of the inner value.
    pub async fn clone(&self) -> Self {
        Self::new(self.handle(|v: &T| Wrapper(v.clone())).await.0)
    }
}

// These impls allow MainValue to be shared between threads safely
unsafe impl<T> Send for MainValue<T> {}
unsafe impl<T> Sync for MainValue<T> {}

static MAIN_THREAD_ID: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();

impl<T: 'static> MainValue<T> {
    /// Creates a new `MainValue` containing the provided value.
    ///
    /// # Parameters
    ///
    /// * `value` - The value to be wrapped in a `MainValue`.
    pub const fn new(value: T) -> Self {
        Self(ManuallyDrop::new(value))
    }

    /// Executes a function on the inner value on the main thread.
    ///
    /// This method safely allows accessing the inner value on the main thread
    /// by scheduling the provided closure to run there.
    ///
    /// # Parameters
    ///
    /// * `f` - The function to execute on the inner value. Must be `Send + 'static`.
    ///
    /// # Returns
    ///
    /// The result of executing the function on the inner value.
    pub fn handle<F, R>(&self, f: F) -> Task<R>
    where
        F: FnOnce(&T) -> R + Send + 'static,
        R: Send + 'static,
    {
        let ptr = Wrapper(from_ref(&self.0));
        if let Some(id) = MAIN_THREAD_ID.get()
            && *id == std::thread::current().id()
        {
            // on main thread, execute it directly
            let ptr = unsafe { &*(ptr.0) };

            let result = f(ptr);
            return Task::new(async move { result });
        }

        Task::on_main(async move {
            let _ = MAIN_THREAD_ID.get_or_init(|| std::thread::current().id());
            let ptr = ptr;
            let ptr = unsafe { &*(ptr.0) };
            f(ptr)
        })
    }
}
