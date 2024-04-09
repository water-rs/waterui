use std::{
    fmt::{Debug, Display},
    mem::replace,
    ops::{Deref, DerefMut},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{subscriber::SubscriberManager, Subscriber};

pub struct Binding<T> {
    inner: Arc<BindingInner<T>>,
}

impl<T: Debug> Debug for Binding<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.read().fmt(f)
    }
}

impl<T: Display> Display for Binding<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.read().fmt(f)
    }
}

impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Binding<Option<T>> {
    pub fn take(&self) -> Option<T> {
        let mut result = None;
        self.peek_mut(|v| result = v.take());
        result
    }
}

impl<T: Clone> Binding<T> {
    pub fn get(&self) -> T {
        self.inner.value.read().unwrap().clone()
    }
}

impl<T> Binding<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Arc::new(BindingInner {
                value: RwLock::new(value),
                subscribers: SubscriberManager::new(),
            }),
        }
    }

    pub fn bridge<F, Output>(&self, f: F) -> Binding<Output>
    where
        F: 'static + Send + Sync + Fn(&T) -> Output,
        Output: Send + Sync + 'static,
        T: Send + Sync + 'static,
    {
        let result = Binding::new(f(self.read().deref()));

        self.register_subscriber({
            let result = result.clone();
            let source = self.clone();

            move || result.set(f(source.read().deref()))
        });
        result
    }

    pub fn replace(&self, value: T) -> T {
        let result = replace(self.write().deref_mut(), value);
        self.notify();
        result
    }

    pub fn set(&self, value: T) {
        let _ = self.replace(value);
    }

    fn read(&self) -> RwLockReadGuard<'_, T> {
        self.inner.value.read().unwrap()
    }

    fn write(&self) -> RwLockWriteGuard<'_, T> {
        self.inner.value.write().unwrap()
    }

    fn notify(&self) {
        self.inner.subscribers.notify();
    }

    pub fn peek(&self, f: impl FnOnce(&T)) {
        f(self.read().deref());
    }

    pub fn peek_mut(&self, f: impl FnOnce(&mut T)) {
        f(self.write().deref_mut());
        self.notify();
    }

    pub fn register_subscriber(&self, subscriber: impl Into<Subscriber>) -> usize {
        self.inner.subscribers.register(subscriber.into())
    }

    pub fn cancel_subscriber(&self, id: usize) {
        self.inner.subscribers.cancel(id);
    }
}

struct BindingInner<T> {
    value: RwLock<T>,
    subscribers: SubscriberManager,
}
