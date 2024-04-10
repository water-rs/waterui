use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use crate::Compute;

pub type Subscriber = Box<dyn Fn() + Send + Sync>;
pub type SharedSubscriberManager = Arc<SubscriberManager>;
pub struct SubscriberManager {
    inner: RwLock<SubscriberManagerInner>,
}

impl Default for SubscriberManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriberManager {
    pub const fn new() -> Self {
        Self {
            inner: RwLock::new(SubscriberManagerInner::new()),
        }
    }
    pub fn register(&self, subscriber: Subscriber) -> usize {
        self.inner.write().unwrap().register(subscriber)
    }

    pub fn notify(&self) {
        self.inner.read().unwrap().notify()
    }

    pub fn cancel(&self, id: usize) {
        self.inner.write().unwrap().cancel(id)
    }
}
struct SubscriberManagerInner {
    id: usize,
    map: BTreeMap<usize, Subscriber>,
}

impl SubscriberManagerInner {
    pub const fn new() -> Self {
        Self {
            id: 0,
            map: BTreeMap::new(),
        }
    }
    pub fn register(&mut self, subscriber: Subscriber) -> usize {
        let id = self
            .id
            .checked_add(1)
            .expect("`id` grows beyond `usize::MAX`");

        self.map.insert(id, subscriber);
        id
    }

    pub fn notify(&self) {
        for (_, subscriber) in self.map.iter() {
            subscriber()
        }
    }

    pub fn cancel(&mut self, id: usize) {
        self.map.remove(&id);
    }
}

pub struct SubscribeGuard<'a, V: ?Sized>
where
    V: Compute,
{
    source: &'a V,
    id: usize,
}

impl<'a, V> SubscribeGuard<'a, V>
where
    V: Compute,
{
    pub fn new(source: &'a V, id: usize) -> Self {
        Self { source, id }
    }
}

impl<'a, V> Drop for SubscribeGuard<'a, V>
where
    V: Compute + ?Sized,
{
    fn drop(&mut self) {
        self.source.cancel_subscriber(self.id);
    }
}
