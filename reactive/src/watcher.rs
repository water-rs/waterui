use alloc::{boxed::Box, collections::BTreeMap, rc::Rc};
use core::{
    any::{Any, TypeId, type_name},
    cell::RefCell,
    fmt::Debug,
    mem::forget,
    num::NonZeroUsize,
};

use crate::{Compute, compute::ComputeResult};

/// A type-erased container for metadata that can be associated with computation results.
///
/// `Metadata` allows attaching arbitrary typed information to computation results
/// and passing it through the computation pipeline.
#[derive(Debug, Default, Clone)]
pub struct Metadata(Box<MetadataInner>);

/// Internal implementation of the metadata storage system.
///
/// Uses a `BTreeMap` with `TypeId` as keys to store type-erased values.
#[derive(Debug, Default, Clone)]
struct MetadataInner(BTreeMap<TypeId, Rc<dyn Any>>);

impl MetadataInner {
    /// Attempts to retrieve a value of type `T` from the metadata store.
    ///
    /// Returns `None` if no value of the requested type is present.
    pub fn try_get<T: 'static + Clone>(&self) -> Option<T> {
        self.0
            .get(&TypeId::of::<T>())
            .map(|v| v.downcast_ref::<T>().unwrap())
            .cloned()
    }

    /// Inserts a value of type `T` into the metadata store.
    ///
    /// If a value of the same type already exists, it will be replaced.
    pub fn insert<T: 'static + Clone>(&mut self, value: T) {
        self.0.insert(TypeId::of::<T>(), Rc::new(value));
    }
}

/// A callback that can be registered to observe computation results.
///
/// Watchers receive both the value of a computation and any associated metadata.
#[allow(clippy::type_complexity)]
pub struct Watcher<T>(Box<dyn Fn(T, Metadata)>);

impl<T> Watcher<T> {
    /// Creates a new watcher with the given callback function.
    ///
    /// The callback will receive both the computation result and associated metadata.
    pub fn new(f: impl Fn(T, Metadata) + 'static) -> Self {
        Self(Box::new(f))
    }

    /// Notifies the watcher with a value and empty metadata.
    pub fn notify(&self, value: T) {
        self.notify_with_metadata(value, Metadata::new())
    }

    /// Notifies the watcher with a value and specific metadata.
    pub fn notify_with_metadata(&self, value: T, metadata: Metadata) {
        (self.0)(value, metadata);
    }
}

/// Allows creating a watcher from a simple callback that only cares about the value.
impl<F, T> From<F> for Watcher<T>
where
    F: Fn(T) + 'static,
{
    fn from(f: F) -> Self {
        Self::new(move |value, _| f(value))
    }
}

impl Metadata {
    /// Creates a new, empty metadata container.
    pub fn new() -> Self {
        Self::default()
    }

    /// Gets a value of type `T` from the metadata.
    ///
    /// # Panics
    ///
    /// Panics if no value of type `T` is present in the metadata.
    pub fn get<T: 'static + Clone>(&self) -> T {
        self.try_get().unwrap()
    }

    /// Attempts to get a value of type `T` from the metadata.
    ///
    /// Returns `None` if no value of the requested type is present.
    pub fn try_get<T: 'static + Clone>(&self) -> Option<T> {
        self.0.try_get()
    }

    /// Adds a value to the metadata and returns the updated metadata.
    ///
    /// This method is chainable for fluent API usage.
    pub fn with<T: 'static + Clone>(mut self, value: T) -> Self {
        self.0.insert(value);
        self
    }
}

/// A unique identifier for registered watchers.
pub(crate) type WatcherId = NonZeroUsize;

/// Manages a collection of watchers for a specific computation type.
///
/// Provides functionality to register, notify, and cancel watchers.
#[derive(Debug)]
pub struct WatcherManager<T> {
    inner: Rc<RefCell<WatcherManagerInner<T>>>,
}

impl<T> Clone for WatcherManager<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: ComputeResult> Default for WatcherManager<T> {
    fn default() -> Self {
        Self {
            inner: Rc::default(),
        }
    }
}

impl<T: ComputeResult> WatcherManager<T> {
    /// Creates a new, empty watcher manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Checks if the manager has any registered watchers.
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    /// Registers a new watcher and returns its unique identifier.
    pub fn register(&self, watcher: Watcher<T>) -> WatcherId {
        self.inner.borrow_mut().register(watcher)
    }

    /// Notifies all registered watchers with a value and empty metadata.
    pub fn notify(&self, value: T) {
        self.notify_with_metadata(value, Metadata::new())
    }

    /// Notifies all registered watchers with a value and specific metadata.
    ///
    /// Uses weak references to avoid memory leaks if notification happens during drop.
    pub fn notify_with_metadata(&self, value: T, metadata: Metadata) {
        let this = Rc::downgrade(&self.inner);
        if let Some(this) = this.upgrade() {
            this.borrow().notify_with_metadata(value, metadata);
        }
    }

    /// Cancels a previously registered watcher by its identifier.
    pub fn cancel(&self, id: WatcherId) {
        self.inner.borrow_mut().cancel(id)
    }
}

/// A RAII guard that automatically cancels a watcher registration when dropped.
///
/// This makes it easy to tie the lifetime of a watcher to a specific scope.
#[must_use]
pub struct WatcherGuard(Option<Box<dyn FnOnce()>>);

impl Debug for WatcherGuard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl WatcherGuard {
    /// Creates a new guard that will execute the given function when dropped.
    pub fn new(f: impl FnOnce() + 'static) -> Self {
        Self(Some(Box::new(f)))
    }

    /// Creates a guard that will cancel a watcher registration when dropped.
    pub fn from_id<T: ComputeResult>(watchers: &WatcherManager<T>, id: WatcherId) -> Self {
        let weak = Rc::downgrade(&watchers.inner);
        Self::new(move || {
            if let Some(rc) = weak.upgrade() {
                rc.borrow_mut().cancel(id)
            }
        })
    }

    /// Prevents the guard from executing its cleanup function when dropped.
    ///
    /// This method is useful when you want to transfer responsibility for cleanup
    /// to another entity.
    pub fn leak(self) {
        forget(self);
    }
}

impl Drop for WatcherGuard {
    fn drop(&mut self) {
        self.0.take().unwrap()();
    }
}

/// Internal implementation of the watcher manager.
///
/// Maintains the collection of watchers and handles identifier assignment.
struct WatcherManagerInner<T> {
    id: WatcherId,
    map: BTreeMap<WatcherId, Watcher<T>>,
}

impl<T> Debug for WatcherManagerInner<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<T> Default for WatcherManagerInner<T> {
    fn default() -> Self {
        Self {
            id: WatcherId::MIN,
            map: BTreeMap::new(),
        }
    }
}

impl<T: ComputeResult> WatcherManagerInner<T> {
    /// Checks if there are any registered watchers.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Assigns a new unique identifier for a watcher.
    fn assign(&mut self) -> WatcherId {
        let id = self.id;
        self.id = self
            .id
            .checked_add(1)
            .expect("`id` grows beyond `usize::MAX`");
        id
    }

    /// Registers a watcher and returns its unique identifier.
    pub fn register(&mut self, watcher: Watcher<T>) -> WatcherId {
        let id = self.assign();
        self.map.insert(id, watcher);
        id
    }

    /// Notifies all registered watchers with a value and metadata.
    pub fn notify_with_metadata(&self, value: T, metadata: Metadata) {
        for watcher in self.map.values() {
            watcher.notify_with_metadata(value.clone(), metadata.clone());
        }
    }

    /// Cancels a watcher registration by its identifier.
    pub fn cancel(&mut self, id: WatcherId) {
        self.map.remove(&id);
    }
}

/// Convenience function to watch a computable value with automatic cleanup.
///
/// Returns a guard that will automatically deregister the watcher when dropped.
pub fn watch<C: Compute>(source: &C, watcher: impl Into<Watcher<C::Output>>) -> WatcherGuard {
    source.add_watcher(watcher.into())
}
