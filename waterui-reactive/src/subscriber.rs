use alloc::{boxed::Box, collections::BTreeMap, rc::Rc};
use core::{cell::RefCell, fmt::Debug, num::NonZeroUsize};

use crate::Reactive;

pub type BoxSubscriber = Box<dyn Subscriber>;

pub trait Subscriber {
    fn call_subscriber(&self);
}

impl<F> Subscriber for F
where
    F: Fn(),
{
    fn call_subscriber(&self) {
        (self)()
    }
}

pub struct FnSubscriber<State, F, F2>
where
    F2: Fn(&State),
{
    state: State,
    f: F,
    drop: F2,
}

impl<State, F, F2> FnSubscriber<State, F, F2>
where
    F: Fn(&State),
    F2: Fn(&State),
{
    pub fn new(state: State, f: F, drop: F2) -> Self {
        Self { state, f, drop }
    }
}

impl<State, F, F2> Drop for FnSubscriber<State, F, F2>
where
    F2: Fn(&State),
{
    fn drop(&mut self) {
        (self.drop)(&self.state)
    }
}

impl<State, F, F2> Subscriber for FnSubscriber<State, F, F2>
where
    F: Fn(&State),
    F2: Fn(&State),
{
    fn call_subscriber(&self) {
        (self.f)(&self.state);
    }
}

pub type SharedSubscriberManager = Rc<SubscriberManager>;
#[derive(Debug)]
pub struct SubscriberManager {
    inner: RefCell<SubscriberManagerInner>,
}

impl Default for SubscriberManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriberManager {
    pub const fn new() -> Self {
        Self {
            inner: RefCell::new(SubscriberManagerInner::new()),
        }
    }

    pub fn preassign(&self) -> NonZeroUsize {
        self.inner.borrow_mut().preassign()
    }

    pub fn register(&self, subscriber: BoxSubscriber) -> NonZeroUsize {
        self.inner.borrow_mut().register(subscriber)
    }

    pub fn register_with_id(&self, id: NonZeroUsize, subscriber: BoxSubscriber) {
        self.inner.borrow_mut().register_with_id(id, subscriber);
    }

    pub fn notify(&self) {
        self.inner.borrow().notify()
    }

    pub fn cancel(&self, id: NonZeroUsize) {
        self.inner.borrow_mut().cancel(id)
    }
}

struct SubscriberManagerInner {
    id: NonZeroUsize,
    map: BTreeMap<NonZeroUsize, BoxSubscriber>,
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
                id: NonZeroUsize::new_unchecked(1),
                map: BTreeMap::new(),
            }
        }
    }

    pub fn preassign(&mut self) -> NonZeroUsize {
        let id = self.id;

        self.id
            .checked_add(1)
            .expect("`id` grows beyond `usize::MAX`");
        id
    }

    pub fn register(&mut self, subscriber: BoxSubscriber) -> NonZeroUsize {
        let id = self.preassign();
        self.register_with_id(id, subscriber);
        id
    }

    pub fn register_with_id(&mut self, id: NonZeroUsize, subscriber: BoxSubscriber) {
        let result = self.map.insert(id, subscriber);
        assert!(result.is_none());
    }

    pub fn notify(&self) {
        for subscriber in self.map.values() {
            subscriber.call_subscriber();
        }
    }

    pub fn cancel(&mut self, id: NonZeroUsize) {
        self.map.remove(&id);
    }
}

#[must_use]
pub struct SubscribeGuard<'a, V>
where
    V: Reactive + ?Sized,
{
    source: &'a V,
    id: Option<NonZeroUsize>,
}

impl<'a, V> SubscribeGuard<'a, V>
where
    V: Reactive,
{
    pub fn new(source: &'a V, id: impl Into<Option<NonZeroUsize>>) -> Self {
        Self {
            source,
            id: id.into(),
        }
    }

    pub fn id(&self) -> Option<NonZeroUsize> {
        self.id
    }
}

impl<'a, V> Drop for SubscribeGuard<'a, V>
where
    V: Reactive + ?Sized,
{
    fn drop(&mut self) {
        self.id.inspect(|id| self.source.cancel_subscriber(*id));
    }
}
