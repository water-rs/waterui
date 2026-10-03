//! Work scheduled onto the main dispatch queue.
//!
//! The main queue is drained by the main thread's run loop, so work enqueued
//! here runs on the main thread, after the current event has been handled, in
//! the order it was enqueued. This is the primitive an executor for
//! main-thread tasks wakes itself with.

use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::MainThreadMarker;

use crate::callback::guarded;

/// Runs `work` on the main thread, asynchronously, from any thread.
///
/// `work` receives the [`MainThreadMarker`] proving where it runs.
///
/// # Panics
///
/// Nothing panics back to the caller. A panic in `work` aborts the process
/// (see the [crate documentation](crate)), and so does the main queue running
/// `work` off the main thread, which would mean the process never gave its
/// main thread to the main queue.
pub fn enqueue(work: impl FnOnce(MainThreadMarker) + Send + 'static) {
    DispatchQueue::main().exec_async(move || {
        guarded("main queue work", || {
            let mtm = MainThreadMarker::new()
                .expect("the main dispatch queue must run its work on the main thread");
            work(mtm);
        });
    });
}

/// Runs `work` on the main thread, asynchronously, from the main thread.
///
/// Unlike [`enqueue`], `work` need not be `Send`: it was created on the main
/// thread and only ever runs or drops there.
pub fn enqueue_local(mtm: MainThreadMarker, work: impl FnOnce(MainThreadMarker) + 'static) {
    let work = MainThreadBound::new(work, mtm);
    enqueue(move |mtm| work.into_inner(mtm)(mtm));
}
