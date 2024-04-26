use core::num::NonZeroUsize;

use crate::{
    subscriber::{BoxSubscriber, SubscriberManager},
    Compute, Reactive,
};

pub struct ComputeFn<F> {
    f: F,
    subscribers: SubscriberManager,
}

impl<F> ComputeFn<F> {
    pub fn new(f: F) -> Self {
        Self {
            f,
            subscribers: SubscriberManager::new(),
        }
    }

    pub fn new_with_subscribers(f: F, subscribers: SubscriberManager) -> Self {
        Self { f, subscribers }
    }
}

impl<T, F> Compute for ComputeFn<F>
where
    F: Fn(&SubscriberManager) -> T,
{
    type Output = T;
    fn compute(&self) -> Self::Output {
        (self.f)(&self.subscribers)
    }
}

impl<F> Reactive for ComputeFn<F> {
    fn register_subscriber(&self, subscriber: BoxSubscriber) -> Option<NonZeroUsize> {
        Some(self.subscribers.register(subscriber))
    }
    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.subscribers.cancel(id)
    }

    fn notify(&self) {
        self.subscribers.notify();
    }
}
