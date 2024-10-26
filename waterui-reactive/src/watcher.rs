use alloc::{boxed::Box, collections::BTreeMap, rc::Rc};
use core::{
    any::{type_name, Any, TypeId},
    cell::RefCell,
    fmt::Debug,
    mem::forget,
    num::NonZeroUsize,
};
use waterui_task::Throttle;

use crate::compute::ComputeResult;

#[derive(Debug, Default, Clone)]
pub struct Metadata(Box<MetadataInner>);

#[derive(Debug, Default, Clone)]
struct MetadataInner(BTreeMap<TypeId, Rc<dyn Any>>);

impl MetadataInner {
    pub fn try_get<T: 'static + Clone>(&self) -> Option<T> {
        self.0
            .get(&TypeId::of::<T>())
            .map(|v| v.downcast_ref::<T>().unwrap())
            .cloned()
    }

    pub fn insert<T: 'static + Clone>(&mut self, value: T) {
        self.0.insert(TypeId::of::<T>(), Rc::new(value));
    }
}

#[allow(clippy::type_complexity)]
pub struct Watcher<T>(Box<dyn Fn(T, Metadata)>);

impl<T> Watcher<T> {
    pub fn new(f: impl Fn(T, Metadata) + 'static) -> Self {
        Self(Box::new(f))
    }

    pub fn notify(&self, value: T) {
        self.notify_with_metadata(value, Metadata::new())
    }

    pub fn notify_with_metadata(&self, value: T, metadata: Metadata) {
        (self.0)(value, metadata);
    }
}

impl<F, T> From<F> for Watcher<T>
where
    F: Fn(T) + 'static,
{
    fn from(f: F) -> Self {
        Self::new(move |value, _| f(value))
    }
}

impl Metadata {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get<T: 'static + Clone>(&self) -> T {
        self.try_get().unwrap()
    }

    pub fn try_get<T: 'static + Clone>(&self) -> Option<T> {
        self.0.try_get()
    }

    pub fn with<T: 'static + Clone>(mut self, value: T) -> Self {
        self.0.insert(value);
        self
    }
}

pub(crate) type WatcherId = NonZeroUsize;

#[derive(Debug)]
pub struct WatcherManager<T> {
    inner: Rc<RefCell<WatcherManagerInner<T>>>,
    throttle: Throttle,
}

impl<T> Clone for WatcherManager<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            throttle: self.throttle.clone(),
        }
    }
}

impl<T: ComputeResult> Default for WatcherManager<T> {
    fn default() -> Self {
        Self {
            inner: Rc::default(),
            throttle: Throttle::default(),
        }
    }
}

impl<T: ComputeResult> WatcherManager<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    pub fn register(&self, watcher: Watcher<T>) -> WatcherId {
        self.inner.borrow_mut().register(watcher)
    }

    pub fn notify(&self, value: T) {
        self.notify_with_metadata(value, Metadata::new())
    }

    pub fn notify_with_metadata(&self, value: T, metadata: Metadata) {
        let this = Rc::downgrade(&self.inner);
        self.throttle.spawn(async move {
            if let Some(this) = this.upgrade() {
                this.borrow().notify_with_metadata(value, metadata);
            }
        });
    }

    pub fn cancel(&self, id: WatcherId) {
        self.inner.borrow_mut().cancel(id)
    }
}

#[must_use]
pub struct WatcherGuard(Option<Box<dyn FnOnce()>>);

impl Debug for WatcherGuard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl WatcherGuard {
    pub fn new(f: impl FnOnce() + 'static) -> Self {
        Self(Some(Box::new(f)))
    }

    pub fn from_id<T: ComputeResult>(watchers: &WatcherManager<T>, id: WatcherId) -> Self {
        let weak = Rc::downgrade(&watchers.inner);
        Self::new(move || {
            if let Some(rc) = weak.upgrade() {
                rc.borrow_mut().cancel(id)
            }
        })
    }

    pub fn leak(self) {
        forget(self);
    }
}

impl Drop for WatcherGuard {
    fn drop(&mut self) {
        self.0.take().unwrap()();
    }
}

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
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn assign(&mut self) -> WatcherId {
        let id = self.id;
        self.id = self
            .id
            .checked_add(1)
            .expect("`id` grows beyond `usize::MAX`");
        id
    }

    pub fn register(&mut self, watcher: Watcher<T>) -> WatcherId {
        let id = self.assign();
        self.map.insert(id, watcher);
        id
    }

    pub fn notify_with_metadata(&self, value: T, metadata: Metadata) {
        for watcher in self.map.values() {
            watcher.notify_with_metadata(value.clone(), metadata.clone());
        }
    }

    pub fn cancel(&mut self, id: WatcherId) {
        self.map.remove(&id);
    }
}
