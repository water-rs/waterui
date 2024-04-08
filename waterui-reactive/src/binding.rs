use std::sync::{Arc, RwLock};

use crate::{subscriber::SubscriberManager, Subscriber};

pub struct Binding<T> {
    inner: Arc<BindingInner<T>>,
}

impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Binding<T> {
    pub fn get(&self) -> T {
        self.inner.inner.get()
    }

    pub fn set(&self, value: T) {
        self.inner.inner.set(value);
        self.inner.subscribers.read().unwrap().notify();
    }

    pub fn subscribe(&self, subscriber: Subscriber) -> usize {
        self.inner
            .subscribers
            .write()
            .unwrap()
            .subscribe(subscriber)
    }

    pub fn unsubscribe(&self, id: usize) {
        self.inner.subscribers.write().unwrap().unsubscribe(id);
    }
}

struct BindingInner<T> {
    subscribers: RwLock<SubscriberManager>,
    inner: dyn BindingImpl<T>,
}

trait BindingImpl<T> {
    fn get(&self) -> T;
    fn set(&self, value: T);
}

struct BindingContainer<T> {
    value: RwLock<T>,
}

struct CustomBinding<Getter, Setter> {
    getter: Getter,
    setter: Setter,
}

impl<T: Clone> BindingImpl<T> for BindingContainer<T> {
    fn get(&self) -> T {
        self.value.read().unwrap().clone()
    }

    fn set(&self, value: T) {
        *self.value.write().unwrap() = value;
    }
}

impl<T, Getter, Setter> BindingImpl<T> for CustomBinding<Getter, Setter>
where
    Getter: Fn() -> T,
    Setter: Fn(T),
{
    fn get(&self) -> T {
        (self.getter)()
    }

    fn set(&self, value: T) {
        (self.setter)(value)
    }
}
