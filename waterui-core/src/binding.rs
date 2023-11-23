use std::{
    any::type_name,
    fmt::{Debug, Display},
    ops::{Deref, DerefMut},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

#[derive(Default)]
pub struct Binding<T: 'static> {
    inner: Arc<RawBinding<T>>,
}

impl<T> Debug for Binding<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<T> From<T> for Binding<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: Display> Display for Binding<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

impl<T: Iterator> Iterator for Binding<T> {
    type Item = T::Item;
    fn next(&mut self) -> Option<Self::Item> {
        self.get_mut().next()
    }
}

impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Default)]
struct RawBinding<T: 'static> {
    value: RwLock<T>,
    watchers: RwLock<Vec<BoxWatcher<T>>>,
}

pub trait Watcher<T>: 'static {
    fn call_watcher(&self, value: &T);
}

impl<T: 'static> Watcher<T> for BoxWatcher<T> {
    fn call_watcher(&self, value: &T) {
        self.deref().call_watcher(value)
    }
}

impl Subscriber for BoxSubscriber {
    fn call_subscriber(&self) {
        self.deref().call_subscriber()
    }
}

pub trait Subscriber: 'static {
    fn call_subscriber(&self);
}

impl<T: 'static, S> Watcher<T> for S
where
    S: Subscriber,
{
    fn call_watcher(&self, _value: &T) {
        self.call_subscriber();
    }
}

pub type BoxWatcher<T> = Box<dyn Watcher<T>>;
pub type BoxSubscriber = Box<dyn Subscriber>;

impl<T> RawBinding<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: RwLock::new(value),
            watchers: RwLock::new(Vec::new()),
        }
    }

    pub fn get(&self) -> RwLockReadGuard<T> {
        self.value.read().unwrap()
    }

    pub fn get_mut(&self) -> MutGuard<T> {
        MutGuard {
            guard: self.value.write().unwrap(),
            watchers: self.watchers.read().unwrap(),
        }
    }

    pub fn make_effect(&self) {
        for watcher in self.watchers.read().unwrap().deref() {
            watcher.call_watcher(self.get().deref());
        }
    }

    pub fn add_watcher(&self, watcher: BoxWatcher<T>) {
        self.watchers.write().unwrap().push(watcher)
    }

    pub fn add_subscriber(&self, subscriber: BoxSubscriber) {
        self.watchers.write().unwrap().push(Box::new(subscriber))
    }
}

impl<T> Binding<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Arc::new(RawBinding::new(value)),
        }
    }

    pub fn get(&self) -> RwLockReadGuard<T> {
        self.inner.get()
    }

    pub fn get_mut(&self) -> MutGuard<T> {
        self.inner.get_mut()
    }

    pub fn add_boxed_watcher(&self, watcher: BoxWatcher<T>) {
        self.inner.add_watcher(watcher)
    }

    pub fn add_boxed_subscriber(&self, subscriber: BoxSubscriber) {
        self.inner.add_subscriber(subscriber)
    }

    pub fn make_effect(&self) {
        self.inner.make_effect();
    }
}

impl Binding<String> {
    pub fn string(string: impl Into<String>) -> Self {
        Self::new(string.into())
    }
}

pub struct MutGuard<'a, T: 'static> {
    guard: RwLockWriteGuard<'a, T>,
    watchers: RwLockReadGuard<'a, Vec<BoxWatcher<T>>>,
}

impl<'a, T> Deref for MutGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.deref()
    }
}

impl<'a, T> DerefMut for MutGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.deref_mut()
    }
}

impl<'a, T> Drop for MutGuard<'a, T> {
    fn drop(&mut self) {
        for watcher in self.watchers.deref() {
            watcher.call_watcher(&self.guard)
        }
    }
}
