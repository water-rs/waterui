use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

#[repr(C)]
#[derive(Debug)]
pub struct Subscriber {
    state: *mut (),
    subscriber: unsafe extern "C" fn(*mut ()),
}

impl<F> From<F> for Subscriber
where
    F: Fn() + Send + Sync,
{
    fn from(value: F) -> Self {
        Self::new(value)
    }
}

unsafe impl Send for Subscriber {}
unsafe impl Sync for Subscriber {}

impl Drop for Subscriber {
    fn drop(&mut self) {
        unsafe { drop(Box::from_raw(self.state)) }
    }
}

impl Subscriber {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn() + Send + Sync,
    {
        let boxed: Box<Box<dyn Fn()>> = Box::new(Box::new(f));
        let state = Box::into_raw(boxed) as *mut ();
        extern "C" fn from_fn_impl(state: *mut ()) {
            let boxed = state as *mut Box<dyn Fn()>;
            unsafe {
                let f = &*boxed;
                (f)()
            }
        }
        unsafe { Self::from_raw(state, from_fn_impl) }
    }

    pub fn notify(&self) {
        unsafe { (self.subscriber)(self.state) }
    }

    /// Constructs subscribers from raw state and subscriber.
    /// # Safety
    /// The subscriber function is marked as unsafe because it requires a raw pointer.
    /// You must make sure the state and subscriber function is implemented correctly.
    pub unsafe fn from_raw(state: *mut (), subscriber: unsafe extern "C" fn(*mut ())) -> Self {
        Self { state, subscriber }
    }
}

pub struct SubscriberManager {
    id: usize,
    map: BTreeMap<usize, Subscriber>,
}

impl SubscriberManager {
    pub const fn new() -> Self {
        Self {
            id: 0,
            map: BTreeMap::new(),
        }
    }
    pub fn subscribe(&mut self, subscriber: Subscriber) -> usize {
        let id = self
            .id
            .checked_add(1)
            .expect("`id` grows beyond `usize::MAX`");

        self.map.insert(id, subscriber);
        id
    }

    pub fn notify(&self) {
        for (_, subscriber) in self.map.iter() {
            subscriber.notify()
        }
    }

    pub fn unsubscribe(&mut self, id: usize) {
        self.map.remove(&id);
    }
}

#[derive(Clone)]
pub struct SharedSubscriberManager {
    inner: Arc<RwLock<SubscriberManager>>,
}

impl SharedSubscriberManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(SubscriberManager::new())),
        }
    }
    pub fn subscribe(&self, subscriber: Subscriber) -> usize {
        self.inner.write().unwrap().subscribe(subscriber)
    }

    pub fn notify(&self) {
        self.inner.read().unwrap().notify()
    }

    pub fn unsubscribe(&self, id: usize) {
        self.inner.write().unwrap().unsubscribe(id)
    }
}
