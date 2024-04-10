use std::{collections::BTreeMap, marker::PhantomData, sync::RwLock};

use crate::Compute;

pub type Subscriber = Box<dyn Fn() + Send + Sync>;

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

#[doc(hidden)]
pub trait SubscribeManage<T, const TLEN: usize> {
    fn register_subscriber(&self, subscriber: impl Fn() -> Subscriber) -> [usize; TLEN];
    fn cancel_subscriber(&self, guard: [usize; TLEN]);
}

impl<V1, V2, T1, T2> SubscribeManage<(T1, T2), 2> for (V1, V2)
where
    V1: Compute<T1>,
    V2: Compute<T2>,
{
    fn register_subscriber(&self, subscriber: impl Fn() -> Subscriber) -> [usize; 2] {
        [
            self.0.register_subscriber(subscriber()),
            self.1.register_subscriber(subscriber()),
        ]
    }
    fn cancel_subscriber(&self, guard: [usize; 2]) {
        self.0.cancel_subscriber(guard[0]);
        self.1.cancel_subscriber(guard[1]);
    }
}

pub struct SubscribeGuard<'a, V, T>
where
    V: Compute<T>,
{
    source: &'a V,
    id: usize,
    _marker: PhantomData<T>,
}

impl<'a, V, T> SubscribeGuard<'a, V, T>
where
    V: Compute<T>,
{
    pub fn new(source: &'a V, id: usize) -> Self {
        Self {
            source,
            id,
            _marker: PhantomData,
        }
    }
}

impl<'a, V, T> Drop for SubscribeGuard<'a, V, T>
where
    V: Compute<T>,
{
    fn drop(&mut self) {
        self.source.cancel_subscriber(self.id);
    }
}
