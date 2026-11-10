//! A value the canvas sets and its recordings observe.

use alloc::rc::Rc;
use core::cell::RefCell;

use nami::Signal;
use nami::watcher::{Context, WatcherManager, WatcherManagerGuard};

struct Inner<T> {
    value: RefCell<T>,
    watchers: WatcherManager<T>,
}

/// A signal whose value the canvas sets, passing the change's metadata on —
/// so an item updated under an animation animates the operands bound to it.
pub struct SignalCell<T: 'static>(Rc<Inner<T>>);

impl<T: 'static> Clone for SignalCell<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: Clone + 'static> SignalCell<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(Rc::new(Inner {
            value: RefCell::new(value),
            watchers: WatcherManager::new(),
        }))
    }

    /// Sets the value and notifies every watcher with `change`'s metadata.
    pub(crate) fn set(&self, change: &Context<T>) {
        *self.0.value.borrow_mut() = change.value().clone();
        self.0.watchers.notify(change);
    }

    /// Whether anything currently observes the value.
    pub(crate) fn observed(&self) -> bool {
        !self.0.watchers.is_empty()
    }
}

impl<T: Clone + 'static> Signal for SignalCell<T> {
    type Output = T;
    type Guard = WatcherManagerGuard<T>;

    fn snapshot(&self) -> T {
        self.0.value.borrow().clone()
    }

    fn watch(&self, watcher: impl Fn(Context<T>) + 'static) -> Self::Guard {
        self.0.watchers.register_as_guard(watcher)
    }
}
