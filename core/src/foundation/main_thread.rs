//! [`MainThreadBound`]: a main-thread-confinement wrapper.
//!
//! The wrapper makes a `!Send`/`!Sync` value satisfy `Send + Sync` at the type
//! level while asserting at runtime that the value is only ever accessed on the
//! thread that created it (the main / UI thread).
//!
//! This is the primitive that lets the parallel layout engine measure children
//! across worker threads. A child whose measurement must stay on the main thread
//! (see [`SubView::require_main_thread`](crate::layout::SubView::require_main_thread))
//! keeps its `!Send` measurement state inside a `MainThreadBound`; the layout
//! executor guarantees such children are only ever measured on the main thread, so
//! the runtime check never fires in correct operation — it is the fail-fast safety
//! net for a [`require_main_thread`](crate::layout::SubView::require_main_thread)
//! misclassification, not a hot-path guard.

use core::fmt;
use core::mem::ManuallyDrop;
use core::ops::Deref;

/// Wraps a `!Send`/`!Sync` value so it satisfies `Send + Sync`, while enforcing at
/// runtime that it is only accessed on the thread that constructed it.
///
/// # Safety model
///
/// The [`Send`] and [`Sync`] impls are `unsafe`: they are sound only because every
/// access goes through [`get`](Self::get) / [`get_mut`](Self::get_mut) /
/// [`Deref`], each of which asserts the caller is on the owning thread and panics
/// otherwise, so the inner `!Send` value never actually crosses a thread boundary.
/// If the wrapper is dropped on a non-owning thread the inner value is **leaked**
/// rather than dropped, since running a `!Send` destructor off-thread would be
/// undefined behavior.
///
/// On `no_std` (single-threaded embedded) targets there is no other thread to
/// violate the confinement, so the checks compile to no-ops.
pub struct MainThreadBound<T> {
    #[cfg(feature = "std")]
    owner: std::thread::ThreadId,
    value: ManuallyDrop<T>,
}

#[allow(
    clippy::non_send_fields_in_send_ty,
    reason = "`MainThreadBound` deliberately carries non-`Send` data; the `Send` impl is sound because access is confined to the owning thread by `assert_owner`"
)]
// SAFETY: the inner value is only ever accessed on the owning thread (enforced at
// runtime by `assert_owner`), and is leaked rather than dropped off-thread, so it
// never crosses a thread boundary in practice.
unsafe impl<T> Send for MainThreadBound<T> {}
// SAFETY: see the `Send` impl — shared access is likewise confined to the owning
// thread by `assert_owner`.
unsafe impl<T> Sync for MainThreadBound<T> {}

impl<T> MainThreadBound<T> {
    /// Bind `value` to the current (main) thread.
    ///
    /// Recording the owning thread needs `std`; without it there is no thread to
    /// record and the binding is a plain wrapper, so that build gets a `const`
    /// constructor.
    #[cfg(feature = "std")]
    #[must_use]
    pub fn new(value: T) -> Self {
        Self {
            owner: std::thread::current().id(),
            value: ManuallyDrop::new(value),
        }
    }

    /// Bind `value` to the current (main) thread.
    #[cfg(not(feature = "std"))]
    #[must_use]
    pub const fn new(value: T) -> Self {
        Self {
            value: ManuallyDrop::new(value),
        }
    }

    #[cfg(feature = "std")]
    #[inline]
    fn is_owner_thread(&self) -> bool {
        std::thread::current().id() == self.owner
    }

    #[cfg(not(feature = "std"))]
    #[inline]
    #[allow(clippy::unused_self)]
    const fn is_owner_thread(&self) -> bool {
        // Single-threaded (no_std/embedded) targets have no other thread to violate
        // the confinement, so the owner check is unconditionally satisfied.
        true
    }

    #[inline]
    fn assert_owner(&self) {
        assert!(
            self.is_owner_thread(),
            "MainThreadBound accessed off the main thread: a view requiring \
             main-thread measurement was scheduled on a worker. This is a \
             SubView::require_main_thread() misclassification."
        );
    }

    /// Consume the wrapper and return the inner value, asserting the caller is on
    /// the owning thread.
    ///
    /// # Panics
    ///
    /// Panics if called from a thread other than the one that constructed the value.
    #[inline]
    #[must_use]
    pub fn into_inner(self) -> T {
        self.assert_owner();
        let mut me = ManuallyDrop::new(self);
        // SAFETY: `me`'s Drop is suppressed by ManuallyDrop and `me` is never used
        // again, so taking the inner value exactly once is sound.
        unsafe { ManuallyDrop::take(&mut me.value) }
    }
}

// Access is exposed only through `Deref`/`DerefMut` (not inherent `get`/`get_mut`)
// so the wrapped type's own methods — `Signal::get`, `RefCell::get`, etc. — remain
// reachable transparently via auto-deref. Both directions assert main-thread access.
impl<T> Deref for MainThreadBound<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.assert_owner();
        &self.value
    }
}

impl<T> core::ops::DerefMut for MainThreadBound<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.assert_owner();
        &mut self.value
    }
}

impl<T> Drop for MainThreadBound<T> {
    fn drop(&mut self) {
        if self.is_owner_thread() {
            // SAFETY: on the owning thread the inner value has not been taken and is
            // safe to drop here; `value` is not used again after this.
            unsafe { ManuallyDrop::drop(&mut self.value) }
        } else {
            // Dropping a `!Send` value off-thread would run its destructor on the
            // wrong thread (undefined behavior). Leak it instead to stay sound, and
            // flag the misuse loudly in debug builds.
            debug_assert!(
                false,
                "MainThreadBound dropped off the main thread; leaking the inner value \
                 to stay sound"
            );
        }
    }
}

impl<T> fmt::Debug for MainThreadBound<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Do not access the value here: keep `Debug` usable from any thread for
        // diagnostics, and avoid requiring `T: Debug` on wrapping structs.
        f.debug_struct("MainThreadBound").finish_non_exhaustive()
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::MainThreadBound;
    use alloc::rc::Rc;

    #[test]
    fn access_on_owner_thread_succeeds() {
        let bound = MainThreadBound::new(Rc::new(7u32));
        assert_eq!(**bound, 7);
        assert_eq!(*bound.into_inner(), 7);
    }

    #[test]
    fn is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        // `Rc` is neither Send nor Sync; the wrapper makes it both.
        assert_send_sync::<MainThreadBound<Rc<u32>>>();
    }

    #[test]
    fn access_off_owner_thread_panics() {
        // Move the wrapper to another thread and access it there: the owner-thread
        // assertion must fire (the fail-fast safety net), not silently succeed.
        let bound = MainThreadBound::new(Rc::new(1u32));
        let handle = std::thread::spawn(move || {
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = &*bound;
            }));
            // Leak the wrapper on this non-owning thread so its `Rc` is not dropped
            // here; `MainThreadBound`'s Drop would leak it anyway, but be explicit.
            core::mem::forget(bound);
            caught.is_err()
        });
        assert!(
            handle.join().expect("spawned thread panicked unexpectedly"),
            "accessing MainThreadBound off the owner thread must panic"
        );
    }
}
