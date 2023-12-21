use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::subscriber::Subscriber;

pub struct Binding<T> {
    inner: Arc<BindingInner<T>>,
}

struct BindingInner<T> {
    value: RwLock<T>,
    subscribers: RwLock<Vec<Subscriber>>,
}

pub struct BindingReadGuard<'a, T> {
    guard: RwLockReadGuard<'a, T>,
}

impl<T> Deref for BindingReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.deref()
    }
}

pub struct BindingWriteGuard<'a, T> {
    guard: Option<RwLockWriteGuard<'a, T>>,
    subscribers: &'a RwLock<Vec<Subscriber>>,
}

impl<T> Deref for BindingWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.as_deref().unwrap()
    }
}

impl<T> DerefMut for BindingWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_deref_mut().unwrap()
    }
}

impl<T> Drop for BindingWriteGuard<'_, T> {
    fn drop(&mut self) {
        let _ = self.guard.take();
        let _ = self
            .subscribers
            .read()
            .unwrap()
            .iter()
            .map(Subscriber::call);
    }
}

impl<T> BindingInner<T> {
    pub fn get(&self) -> BindingReadGuard<T> {
        BindingReadGuard {
            guard: self.value.read().unwrap(),
        }
    }

    pub fn get_mut(&self) -> BindingWriteGuard<T> {
        BindingWriteGuard {
            guard: Some(self.value.write().unwrap()),
            subscribers: &self.subscribers,
        }
    }

    pub fn subscribe(&self, subscriber: Subscriber) {
        self.subscribers.write().unwrap().push(subscriber)
    }
}

impl<T> Binding<T> {
    pub fn new(value: impl Into<T>) -> Self {
        Self::from(value.into())
    }

    pub fn get(&self) -> BindingReadGuard<T> {
        self.inner.get()
    }

    pub fn get_mut(&self) -> BindingWriteGuard<T> {
        self.inner.get_mut()
    }

    pub fn subscribe(&self, subscriber: impl Into<Subscriber>) {
        self.inner.subscribe(subscriber.into())
    }

    pub fn make_effect(&self) {
        let _ = self
            .inner
            .subscribers
            .read()
            .unwrap()
            .iter()
            .map(Subscriber::call);
    }

    pub fn set(&self, value: impl Into<T>) {
        *self.get_mut() = value.into();
    }
}

impl<T> From<T> for Binding<T> {
    fn from(value: T) -> Self {
        Self {
            inner: Arc::new(BindingInner::from(value)),
        }
    }
}

impl<T> From<T> for BindingInner<T> {
    fn from(value: T) -> Self {
        Self {
            value: RwLock::new(value),
            subscribers: RwLock::new(Vec::new()),
        }
    }
}
