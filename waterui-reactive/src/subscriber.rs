use alloc::{boxed::Box, collections::BTreeMap, rc::Rc};
use core::{cell::RefCell, num::NonZeroUsize};

pub type Subscriber = Box<dyn Fn()>;
pub type SharedSubscriberManager = Rc<SubscriberManager>;
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
    pub fn register(&self, subscriber: Subscriber) -> NonZeroUsize {
        self.inner.borrow_mut().register(subscriber)
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
    map: BTreeMap<NonZeroUsize, Subscriber>,
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
    pub fn register(&mut self, subscriber: Subscriber) -> NonZeroUsize {
        let id = self.id;
        self.map.insert(id, subscriber);

        self.id = self
            .id
            .checked_add(1)
            .expect("`id` grows beyond `usize::MAX`");

        id
    }

    pub fn notify(&self) {
        for subscriber in self.map.values() {
            subscriber()
        }
    }

    pub fn cancel(&mut self, id: NonZeroUsize) {
        self.map.remove(&id);
    }
}
