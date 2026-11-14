//! Serial delivery of reactive notifications.
//!
//! A notification delivered while a handler still runs — a handler that writes
//! the signal it watches, or a watcher that emits an event of its own — lands
//! re-entrantly inside the delivering call. Callers that hold the handler in a
//! `RefCell` then panic on the nested borrow. These primitives run the nested
//! delivery *after* the in-flight one returns instead.
//!
//! - [`SerialDispatch`] keeps every queued item and drains it in FIFO order;
//!   use it for ordered work such as transactions and events.
//! - [`LatestDispatch`] collapses queued items into the newest one; use it for
//!   value watchers, where a value superseded before it can run is meaningless.
//!
//! Neither primitive spawns or defers onto another task: queued work runs on
//! the calling thread, inside the outer `deliver`.

use alloc::collections::VecDeque;
use core::cell::{Cell, RefCell};
use core::fmt;

/// Serializes deliveries of `T`, keeping every item in FIFO order.
///
/// `deliver` runs `run` immediately when no delivery is in flight. A `deliver`
/// that arrives while `run` still executes queues the item and returns; the
/// delivery already in flight drains the queue, one item at a time, after `run`
/// returns, so `run` is never called re-entrantly.
pub struct SerialDispatch<T> {
    pending: RefCell<VecDeque<T>>,
    dispatching: Cell<bool>,
}

impl<T> SerialDispatch<T> {
    /// Creates an empty dispatch queue.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: RefCell::new(VecDeque::new()),
            dispatching: Cell::new(false),
        }
    }

    /// Delivers `item` through `run`.
    ///
    /// Runs `run` now when idle; when a `run` is already in flight, queues
    /// `item` and returns, and the in-flight delivery drains it later.
    pub fn deliver(&self, item: T, mut run: impl FnMut(T)) {
        self.pending.borrow_mut().push_back(item);
        if self.dispatching.replace(true) {
            return;
        }
        loop {
            let next = self.pending.borrow_mut().pop_front();
            let Some(item) = next else {
                break;
            };
            run(item);
        }
        self.dispatching.set(false);
    }
}

impl<T> Default for SerialDispatch<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> fmt::Debug for SerialDispatch<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SerialDispatch").finish_non_exhaustive()
    }
}

/// Serializes deliveries of `T`, keeping only the latest queued item.
///
/// Behaves like [`SerialDispatch`], except items queued while `run` executes
/// collapse into the newest one: the drained delivery sees the latest value
/// exactly once.
pub struct LatestDispatch<T> {
    pending: RefCell<Option<T>>,
    dispatching: Cell<bool>,
}

impl<T> LatestDispatch<T> {
    /// Creates an empty dispatch slot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: RefCell::new(None),
            dispatching: Cell::new(false),
        }
    }

    /// Delivers `item` through `run`.
    ///
    /// Runs `run` now when idle; when a `run` is already in flight, stores
    /// `item` as the latest pending value — superseding any earlier pending
    /// one — and returns, and the in-flight delivery drains it later.
    pub fn deliver(&self, item: T, mut run: impl FnMut(T)) {
        *self.pending.borrow_mut() = Some(item);
        if self.dispatching.replace(true) {
            return;
        }
        loop {
            let next = self.pending.borrow_mut().take();
            let Some(item) = next else {
                break;
            };
            run(item);
        }
        self.dispatching.set(false);
    }
}

impl<T> Default for LatestDispatch<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> fmt::Debug for LatestDispatch<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LatestDispatch").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{rc::Rc, vec, vec::Vec};

    #[test]
    fn a_nested_deliver_runs_after_the_in_flight_one() {
        let serial = Rc::new(SerialDispatch::new());
        let seen = Rc::new(RefCell::new(Vec::new()));
        let once = Cell::new(false);

        serial.deliver(1, |item| {
            seen.borrow_mut().push(item);
            if !once.replace(true) {
                serial.deliver(2, |item| seen.borrow_mut().push(item));
            }
        });

        assert_eq!(*seen.borrow(), vec![1, 2]);
    }

    #[test]
    fn queued_items_are_delivered_in_order() {
        let serial = Rc::new(SerialDispatch::new());
        let seen = Rc::new(RefCell::new(Vec::new()));
        let once = Cell::new(false);

        serial.deliver(0, |item| {
            seen.borrow_mut().push(item);
            if !once.replace(true) {
                // Queue two more items while the first is still running; both
                // must arrive after it returns, in FIFO order.
                serial.deliver(1, |item| seen.borrow_mut().push(item));
                serial.deliver(2, |item| seen.borrow_mut().push(item));
            }
        });

        assert_eq!(*seen.borrow(), vec![0, 1, 2]);
    }

    #[test]
    fn latest_dispatch_collapses_queued_items_to_the_newest() {
        let latest = Rc::new(LatestDispatch::new());
        let seen = Rc::new(RefCell::new(Vec::new()));
        let once = Cell::new(false);

        latest.deliver(0, |item| {
            seen.borrow_mut().push(item);
            if !once.replace(true) {
                latest.deliver(1, |item| seen.borrow_mut().push(item));
                latest.deliver(2, |item| seen.borrow_mut().push(item));
            }
        });

        assert_eq!(*seen.borrow(), vec![0, 2]);
    }
}
