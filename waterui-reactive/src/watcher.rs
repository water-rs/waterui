use alloc::{boxed::Box, collections::BTreeMap, rc::Rc};
use core::{
    any::{type_name, Any, TypeId},
    cell::RefCell,
    fmt::Debug,
    mem::forget,
    num::NonZeroUsize,
};

#[derive(Debug, Default, Clone)]
pub struct Metadata(Rc<MetadataBuilder>);

#[derive(Debug, Default)]
pub struct MetadataBuilder(BTreeMap<TypeId, Box<dyn Any>>);

impl MetadataBuilder {
    pub const fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn metadata<T: 'static>(mut self, value: T) -> Self {
        self.0.insert(TypeId::of::<T>(), Box::new(value));
        self
    }

    pub fn build(self) -> Metadata {
        Metadata(Rc::new(self))
    }
}

#[allow(clippy::type_complexity)]
pub struct Watcher<T>(Box<dyn Fn(T, &Metadata)>);

impl<T> Watcher<T> {
    pub fn new(f: impl Fn(T, &Metadata) + 'static) -> Self {
        Self(Box::new(f))
    }

    pub fn notify(&self, value: T) {
        self.notify_with_metadata(value, &Metadata::empty())
    }

    pub fn notify_with_metadata(&self, value: T, metadata: &Metadata) {
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
    pub fn empty() -> Self {
        MetadataBuilder::new().build()
    }

    pub const fn builder() -> MetadataBuilder {
        MetadataBuilder::new()
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.0
             .0
            .get(&TypeId::of::<T>())
            .map(|v| v.downcast_ref().unwrap())
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub struct WatcherId(NonZeroUsize);

#[derive(Debug)]
pub struct WatcherManager<T>(RefCell<WatcherManagerInner<T>>);

pub type SharedWatcherManager<T> = Rc<WatcherManager<T>>;

impl<T> Default for WatcherManager<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> WatcherManager<T> {
    pub const fn new() -> Self {
        Self(RefCell::new(WatcherManagerInner::new()))
    }

    pub fn register(&self, watcher: Watcher<T>) -> WatcherId {
        self.0.borrow_mut().register(watcher)
    }

    pub fn notify(&self, value: impl Fn() -> T) {
        self.notify_with_metadata(value, &Metadata::empty())
    }

    pub fn notify_with_metadata(&self, value: impl Fn() -> T, metadata: &Metadata) {
        self.0.borrow().notify_with_metadata(value, metadata)
    }

    pub fn cancel(&self, id: WatcherId) {
        self.0.borrow_mut().cancel(id)
    }
}

pub struct WatcherGuard(Option<Box<dyn FnOnce()>>);

impl WatcherGuard {
    pub fn new(f: impl FnOnce() + 'static) -> Self {
        Self(Some(Box::new(f)))
    }

    pub(crate) fn from_id<T: 'static>(watchers: &SharedWatcherManager<T>, id: WatcherId) -> Self {
        let weak = Rc::downgrade(watchers);
        Self::new(move || {
            if let Some(rc) = weak.upgrade() {
                rc.cancel(id)
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

impl<T> WatcherManagerInner<T> {
    pub const fn new() -> Self {
        Self {
            id: WatcherId(NonZeroUsize::MIN),
            map: BTreeMap::new(),
        }
    }

    fn assign(&mut self) -> WatcherId {
        let id = self.id;

        self.id
            .0
            .checked_add(1)
            .expect("`id` grows beyond `usize::MAX`");
        id
    }

    pub fn register(&mut self, watcher: Watcher<T>) -> WatcherId {
        let id = self.assign();
        self.map.insert(id, watcher);
        id
    }

    pub fn notify_with_metadata(&self, value: impl Fn() -> T, metadata: &Metadata) {
        for watcher in self.map.values() {
            watcher.notify_with_metadata(value(), metadata);
        }
    }

    pub fn cancel(&mut self, id: WatcherId) {
        self.map.remove(&id);
    }
}
