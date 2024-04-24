use core::{cell::RefCell, num::NonZeroUsize};

use crate::{subscriber::SubscriberManager, Compute, Subscriber};

use super::BindingImpl;

pub struct ContainerBinding<T: Clone> {
    value: RefCell<T>,
    subscribers: SubscriberManager,
}

impl<T: Clone> ContainerBinding<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: RefCell::new(value),
            subscribers: SubscriberManager::new(),
        }
    }
}

impl<T: Clone> Compute for ContainerBinding<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.value.borrow().clone()
    }

    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.subscribers.cancel(id)
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        Some(self.subscribers.register(subscriber))
    }
    fn notify(&self) {
        self.subscribers.notify();
    }
}

impl<T: Clone> BindingImpl<T> for ContainerBinding<T> {
    fn set(&self, value: T) {
        *self.value.borrow_mut() = value;
        self.notify();
    }
}
