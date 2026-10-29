//! Work scheduled onto the main dispatch queue.
//!
//! The main queue is drained by the main thread's run loop, so work enqueued
//! here runs on the main thread, after the current event has been handled, in
//! the order it was enqueued. This is the primitive an executor for
//! main-thread tasks wakes itself with.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub use dispatch2::MainThreadBound;
use dispatch2::{DispatchQueue, DispatchTime};
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

/// Runs `work` on the main thread once `delay` has elapsed, from the main
/// thread.
///
/// The delay is measured on the monotonic clock, so a fire is never early,
/// and `work` need not be `Send` for the reason [`enqueue_local`] gives.
///
/// # Panics
///
/// If `delay` is too long for the dispatch clock to represent (centuries).
pub fn enqueue_after_local(
    mtm: MainThreadMarker,
    delay: Duration,
    work: impl FnOnce(MainThreadMarker) + 'static,
) {
    let when = DispatchTime::try_from(delay)
        .expect("a delay of centuries is not a deadline the dispatch clock can hold");
    let work = MainThreadBound::new(work, mtm);
    DispatchQueue::main()
        .after(when, move || {
            guarded("main queue delayed work", || {
                let mtm = MainThreadMarker::new()
                    .expect("the main dispatch queue must run its work on the main thread");
                work.into_inner(mtm)(mtm);
            });
        })
        .expect("the main queue accepts delayed work");
}

/// A wake any thread can fire to run one main-thread callback.
///
/// Wakes coalesce: however many arrive before the main queue runs the
/// callback, it runs once, and a wake fired while the callback runs queues
/// the next run. This is what an executor for main-thread tasks signals the
/// run loop with when work is queued from another thread.
///
/// Cloning shares the wake. The callback is installed after construction
/// because the thing it drains is usually built from the wake.
#[derive(Clone)]
pub struct MainQueueWake {
    shared: Arc<WakeShared>,
}

/// The callback slot a [`MainQueueWake`] runs.
type WakeCallback = RefCell<Option<Rc<dyn Fn()>>>;

struct WakeShared {
    /// Set while a run is queued on the main queue.
    queued: AtomicBool,
    /// The callback a queued run executes; main-thread only.
    callback: MainThreadBound<WakeCallback>,
}

impl fmt::Debug for MainQueueWake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MainQueueWake")
            .field("queued", &self.shared.queued.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl MainQueueWake {
    /// A wake with no callback yet; wakes fired before
    /// [`set_callback`](Self::set_callback) find nothing to run.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            shared: Arc::new(WakeShared {
                queued: AtomicBool::new(false),
                callback: MainThreadBound::new(RefCell::new(None), mtm),
            }),
        }
    }

    /// Installs the callback a wake runs on the main thread, replacing any
    /// previous one.
    pub fn set_callback(&self, mtm: MainThreadMarker, callback: impl Fn() + 'static) {
        self.shared
            .callback
            .get(mtm)
            .replace(Some(Rc::new(callback)));
    }

    /// Asks the main queue to run the callback, from any thread.
    pub fn wake(&self) {
        if self.shared.queued.swap(true, Ordering::AcqRel) {
            return;
        }
        let shared = Arc::clone(&self.shared);
        enqueue(move |mtm| {
            // Cleared before the callback runs: a wake the callback itself
            // raises must queue another run.
            shared.queued.store(false, Ordering::Release);
            let callback = shared.callback.get(mtm).borrow().clone();
            if let Some(callback) = callback {
                callback();
            }
        });
    }
}
