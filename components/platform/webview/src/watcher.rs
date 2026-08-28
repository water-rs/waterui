//! A removable watcher registry shared by every `WebView` backend.
//!
//! Each backend used to keep its own `Vec` of event callbacks with no way to
//! remove one, so a watcher registered by a short-lived view stayed registered
//! for the lifetime of the web view. This registry owns that bookkeeping once:
//! registration hands back a guard, and dropping the guard unregisters.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

type Watcher<T> = Rc<dyn Fn(T)>;

struct Registry<T> {
    /// Watchers paired with the id they were registered under. A `Vec` keeps
    /// registration order, which backends rely on to deliver events in the
    /// order they were subscribed.
    watchers: RefCell<Vec<(u64, Watcher<T>)>>,
    next_id: RefCell<u64>,
}

/// A set of event watchers that can each be removed independently.
pub struct WatcherSet<T> {
    registry: Rc<Registry<T>>,
}

impl<T> Clone for WatcherSet<T> {
    fn clone(&self) -> Self {
        Self {
            registry: Rc::clone(&self.registry),
        }
    }
}

impl<T: Clone + 'static> Default for WatcherSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> core::fmt::Debug for WatcherSet<T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WatcherSet")
            .field("len", &self.registry.watchers.borrow().len())
            .finish()
    }
}

impl<T: Clone + 'static> WatcherSet<T> {
    /// Creates an empty set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            registry: Rc::new(Registry {
                watchers: RefCell::new(Vec::new()),
                next_id: RefCell::new(0),
            }),
        }
    }

    /// Registers `watcher` and returns the guard that keeps it subscribed.
    pub fn insert(&self, watcher: impl Fn(T) + 'static) -> WatcherGuard {
        let id = {
            let mut next_id = self.registry.next_id.borrow_mut();
            let id = *next_id;
            *next_id = next_id.wrapping_add(1);
            id
        };
        self.registry
            .watchers
            .borrow_mut()
            .push((id, Rc::new(watcher)));
        WatcherGuard {
            remove: Box::new(RegistryHandle {
                registry: Rc::downgrade(&self.registry),
                id,
            }),
        }
    }

    /// Delivers `event` to every registered watcher, in registration order.
    pub fn emit(&self, event: &T) {
        // Snapshot first: a watcher is allowed to register or drop watchers,
        // including its own guard, while it runs.
        let snapshot: Vec<Watcher<T>> = self
            .registry
            .watchers
            .borrow()
            .iter()
            .map(|(_, watcher)| Rc::clone(watcher))
            .collect();
        for watcher in snapshot {
            watcher(event.clone());
        }
    }

    /// Returns the number of live watchers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.registry.watchers.borrow().len()
    }

    /// Returns whether no watcher is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Removes one watcher when dropped.
trait Unregister {
    fn unregister(&self);
}

struct RegistryHandle<T> {
    registry: Weak<Registry<T>>,
    id: u64,
}

impl<T> Unregister for RegistryHandle<T> {
    fn unregister(&self) {
        let Some(registry) = self.registry.upgrade() else {
            return;
        };
        // `retain` may run while `emit` holds only a snapshot, never a borrow.
        registry
            .watchers
            .borrow_mut()
            .retain(|(id, _)| *id != self.id);
    }
}

/// Keeps a watcher subscribed. Dropping it unregisters the watcher.
#[must_use = "dropping the guard immediately unregisters the watcher"]
pub struct WatcherGuard {
    remove: Box<dyn Unregister>,
}

impl core::fmt::Debug for WatcherGuard {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("WatcherGuard").finish()
    }
}

impl WatcherGuard {
    /// Keeps the watcher registered for as long as the web view lives.
    ///
    /// Use this only when the watcher genuinely has the same lifetime as the web
    /// view; otherwise hold the guard.
    pub const fn forget(self) {
        core::mem::forget(self);
    }
}

impl Drop for WatcherGuard {
    fn drop(&mut self) {
        self.remove.unregister();
    }
}

#[cfg(test)]
mod tests {
    use super::WatcherSet;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn watchers_receive_events_in_registration_order() {
        let set = WatcherSet::<u32>::new();
        let seen = Rc::new(RefCell::new(Vec::new()));

        let _first = set.insert({
            let seen = Rc::clone(&seen);
            move |value| seen.borrow_mut().push(("first", value))
        });
        let _second = set.insert({
            let seen = Rc::clone(&seen);
            move |value| seen.borrow_mut().push(("second", value))
        });

        set.emit(&7);

        assert_eq!(*seen.borrow(), vec![("first", 7), ("second", 7)]);
    }

    #[test]
    fn dropping_a_guard_unregisters_only_that_watcher() {
        let set = WatcherSet::<u32>::new();
        let seen = Rc::new(RefCell::new(Vec::new()));

        let first = set.insert({
            let seen = Rc::clone(&seen);
            move |value| seen.borrow_mut().push(("first", value))
        });
        let _second = set.insert({
            let seen = Rc::clone(&seen);
            move |value| seen.borrow_mut().push(("second", value))
        });

        drop(first);
        set.emit(&1);

        assert_eq!(set.len(), 1);
        assert_eq!(*seen.borrow(), vec![("second", 1)]);
    }

    /// A watcher may drop its own guard, or register another, while it runs.
    #[test]
    fn a_watcher_may_mutate_the_set_while_it_is_emitting() {
        let set = WatcherSet::<u32>::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let guard = Rc::new(RefCell::new(None));

        *guard.borrow_mut() = Some(set.insert({
            let seen = Rc::clone(&seen);
            let guard = Rc::clone(&guard);
            let set = set.clone();
            move |value| {
                seen.borrow_mut().push(value);
                // Unsubscribe from inside the callback, and add a new watcher.
                guard.borrow_mut().take();
                set.insert(|_| {}).forget();
            }
        }));

        set.emit(&1);
        set.emit(&2);

        assert_eq!(*seen.borrow(), vec![1]);
    }

    #[test]
    fn forgetting_a_guard_keeps_the_watcher_registered() {
        let set = WatcherSet::<u32>::new();
        let seen = Rc::new(RefCell::new(Vec::new()));

        set.insert({
            let seen = Rc::clone(&seen);
            move |value| seen.borrow_mut().push(value)
        })
        .forget();

        set.emit(&5);

        assert_eq!(*seen.borrow(), vec![5]);
    }

    #[test]
    fn a_guard_outliving_its_set_does_not_panic() {
        let set = WatcherSet::<u32>::new();
        let guard = set.insert(|_| {});
        drop(set);
        drop(guard);
    }
}
