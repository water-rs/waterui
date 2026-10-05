//! A main-queue-owned payload shared into cross-thread callbacks.
//!
//! `dispatch2::MainThreadBound` makes a main-thread value `Send`/`Sync`
//! by gating access on `MainThreadMarker` — a type-level proof that can
//! only be produced on the main thread — and by running its inner drop
//! through `DispatchQueue::main().exec_sync`. That drop
//! contract deadlocks the moment the value's last reference releases on
//! a thread the main queue cannot serve — the cherenkov render thread
//! during `Engine::drop`'s join, a media command pump, an
//! effect-owned callback thread — because `exec_sync` parks the
//! background thread until the main queue runs its block while main is
//! itself waiting on that thread (#1776).
//!
//! [`MainQueueOwned`] keeps the same `new`/`get` contract but owns the
//! payload through the existing asynchronous main-queue drain: on main,
//! the payload drops directly; off-main, the *whole* bound moves into
//! `main_queue::enqueue`, so the third-party synchronous drop never runs
//! on a background thread and the payload's own teardown still lands on
//! its owning thread.

use std::sync::Arc;

use cocoa_ui::MainThreadMarker;
use dispatch2::MainThreadBound;

/// A `T` produced and consumed on the main thread, shared into a
/// callback that outlives it on arbitrary threads.
///
/// Wrap in `Arc` where the callback shares ownership, the same way
/// `MainThreadBound` was wrapped before.
pub struct MainQueueOwned<T: 'static>(Option<MainThreadBound<T>>);

impl<T: 'static> MainQueueOwned<T> {
    /// Takes ownership of `value`, which must already live on the main
    /// thread — the marker is the type-level proof of that, the same
    /// token `MainThreadBound::new` requires.
    pub const fn new(value: T, mtm: MainThreadMarker) -> Self {
        Self(Some(MainThreadBound::new(value, mtm)))
    }

    /// Reads the payload — only meaningful on the main thread, where
    /// every consumer's work item lands; the marker proves the caller
    /// is there.
    pub fn get(&self, mtm: MainThreadMarker) -> &T {
        self.0
            .as_ref()
            .map(|bound| bound.get(mtm))
            .expect("a MainQueueOwned always holds its payload until drop")
    }
}

impl<T: 'static> Drop for MainQueueOwned<T> {
    fn drop(&mut self) {
        let bound = self
            .0
            .take()
            .expect("a MainQueueOwned holds its payload until this drop releases it");
        if let Some(mtm) = MainThreadMarker::new() {
            // Already on the owning thread: extract and drop the payload
            // directly — no queue round-trip, no exec_sync.
            drop(bound.into_inner(mtm));
        } else {
            // The whole bound is `Send`, so it travels to the queue its
            // payload belongs to; `into_inner` extracts it under the
            // real marker the queue provides, and the payload's own
            // destructor runs there — the synchronous third-party drop
            // never blocks a background thread.
            cocoa_ui::main_queue::enqueue(move |mtm| {
                drop(bound.into_inner(mtm));
            });
        }
    }
}

/// `Arc<MainQueueOwned<T>>` is the sharing shape every callback uses.
/// The payload itself is reachable only through [`MainQueueOwned::get`]
/// on the main thread, so cross-thread access is still impossible — the
/// `Arc` shares only the owning handle.
pub type Shared<T> = Arc<MainQueueOwned<T>>;

/// Shares `value` for capture into a cross-thread callback.
pub fn shared<T: 'static>(value: T, mtm: MainThreadMarker) -> Shared<T> {
    Arc::new(MainQueueOwned::new(value, mtm))
}
