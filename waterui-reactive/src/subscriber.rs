use alloc::{boxed::Box, collections::BTreeMap, rc::Rc};
use core::{
    any::{Any, TypeId},
    cell::RefCell,
    fmt::Debug,
    mem::forget,
    num::NonZeroUsize,
};

use crate::Reactive;

#[derive(Debug, Default)]
pub struct Metadata {
    map: BTreeMap<TypeId, Box<dyn Any>>,
}

impl Metadata {
    pub const fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .map(|v| v.downcast_ref().unwrap())
    }

    pub fn insert<T: 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(value));
    }
}

pub struct Subscriber(Box<dyn Fn(&Metadata)>);

impl Subscriber {
    pub fn new(value: impl Fn(&Metadata) + 'static) -> Self {
        Self(Box::new(value))
    }

    pub fn notify(&self, metadata: &Metadata) {
        (self.0)(metadata);
    }
}

impl<F> From<F> for Subscriber
where
    F: Fn() + 'static,
{
    fn from(value: F) -> Self {
        Subscriber::new(move |_| value())
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub struct SubscriberId(NonZeroUsize);

impl SubscriberId {
    pub fn new(raw: NonZeroUsize) -> Self {
        Self(raw)
    }
    pub fn into_inner(self) -> usize {
        self.0.get()
    }
}

pub type SharedSubscriberManager = Rc<SubscriberManager>;

#[derive(Debug)]
pub struct SubscriberManager(RefCell<SubscriberManagerInner>);

impl Default for SubscriberManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriberManager {
    pub const fn new() -> Self {
        Self(RefCell::new(SubscriberManagerInner::new()))
    }

    pub fn register(&self, subscriber: Subscriber) -> SubscriberId {
        self.0.borrow_mut().register(subscriber)
    }

    pub fn notify(&self, metadata: &Metadata) {
        self.0.borrow().notify(metadata);
    }

    pub fn cancel(&self, id: SubscriberId) {
        self.0.borrow_mut().cancel(id)
    }
}

struct SubscriberManagerInner {
    id: SubscriberId,
    map: BTreeMap<SubscriberId, Subscriber>,
}

impl Debug for SubscriberManagerInner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SubscriberManagerInner")
    }
}

impl SubscriberManagerInner {
    pub const fn new() -> Self {
        unsafe {
            Self {
                id: SubscriberId(NonZeroUsize::new_unchecked(1)),
                map: BTreeMap::new(),
            }
        }
    }

    fn assign(&mut self) -> SubscriberId {
        let id = self.id;

        self.id
            .0
            .checked_add(1)
            .expect("`id` grows beyond `usize::MAX`");
        id
    }

    pub fn register(&mut self, subscriber: Subscriber) -> SubscriberId {
        let id = self.assign();
        self.map.insert(id, subscriber);
        id
    }

    pub fn notify(&self, metadata: &Metadata) {
        for subscriber in self.map.values() {
            subscriber.notify(metadata);
        }
    }

    pub fn cancel(&mut self, id: SubscriberId) {
        self.map.remove(&id);
    }
}

#[derive(Debug)]
#[must_use]
pub struct SubscribeGuard<V>
where
    V: Reactive,
{
    source: V,
    id: Option<SubscriberId>,
}

impl<V> SubscribeGuard<V>
where
    V: Reactive,
{
    pub fn new(source: V, id: Option<SubscriberId>) -> Self {
        Self { source, id }
    }

    pub fn into_raw(self) -> Option<SubscriberId> {
        self.id
    }

    pub fn leak(self) {
        forget(self);
    }
}

impl<V: Reactive + Clone> SubscribeGuard<&V> {
    pub fn cloned(self) -> SubscribeGuard<V> {
        SubscribeGuard {
            source: self.source.clone(),
            id: self.id,
        }
    }
}

impl<V: Reactive + Clone> SubscribeGuard<&mut V> {
    pub fn cloned(self) -> SubscribeGuard<V> {
        SubscribeGuard {
            source: self.source.clone(),
            id: self.id,
        }
    }
}

impl<V> Drop for SubscribeGuard<V>
where
    V: Reactive,
{
    fn drop(&mut self) {
        self.id.inspect(|id| self.source.cancel_subscriber(*id));
    }
}
