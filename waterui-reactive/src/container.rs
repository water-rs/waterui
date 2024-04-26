use core::{
    cell::{Ref, RefCell, RefMut},
    ops::{Deref, DerefMut, Index},
};

use alloc::rc::Rc;

use crate::{binding::BindingImpl, subscriber::SubscriberManager, Compute, Reactive};

pub struct Container<T> {
    inner: Rc<ContainerInner<T>>,
}

impl<T> Container<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Rc::new(ContainerInner {
                data: RefCell::new(value),
                subscribers: SubscriberManager::new(),
            }),
        }
    }
}

impl<T> Clone for Container<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

struct ContainerInner<T> {
    data: RefCell<T>,
    subscribers: SubscriberManager,
}

pub struct ContainerGuard<'a, T> {
    data: Ref<'a, T>,
}

impl<T> Deref for ContainerGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.data.deref()
    }
}

impl<T> Deref for ContainerMutGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.data.deref()
    }
}

impl<T> DerefMut for ContainerMutGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data.deref_mut()
    }
}

pub struct ContainerMutGuard<'a, T> {
    data: RefMut<'a, T>,
    subscribers: &'a SubscriberManager,
}

impl<T> Drop for ContainerMutGuard<'_, T> {
    fn drop(&mut self) {
        self.subscribers.notify();
    }
}

impl<T> Container<T> {
    pub fn get(&self) -> ContainerGuard<'_, T> {
        ContainerGuard {
            data: self.inner.data.borrow(),
        }
    }

    pub fn get_mut(&self) -> ContainerMutGuard<'_, T> {
        ContainerMutGuard {
            data: self.inner.data.borrow_mut(),
            subscribers: &self.inner.subscribers,
        }
    }

    pub fn set(&self, value: T) {
        *self.get_mut() = value;
    }
}

impl<T> Reactive for Container<T> {
    fn register_subscriber(
        &self,
        subscriber: crate::Subscriber,
    ) -> Option<core::num::NonZeroUsize> {
        Some(self.inner.subscribers.register(subscriber))
    }

    fn cancel_subscriber(&self, id: core::num::NonZeroUsize) {
        self.inner.subscribers.cancel(id);
    }
    fn notify(&self) {
        self.inner.subscribers.notify();
    }
}

impl<T: Clone> Compute for Container<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.get().clone()
    }
}

impl<T: Clone> BindingImpl<T> for Container<T> {
    fn set(&self, value: T) {
        Container::set(self, value);
    }
}
