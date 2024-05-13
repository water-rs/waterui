use alloc::{boxed::Box, vec::Vec};

use crate::{
    reactive::BoxReactive,
    subscriber::{
        SharedSubscriberManager, SubscribeGuard, Subscriber, SubscriberId, SubscriberManager,
    },
    Compute, Reactive,
};

pub struct ComputeFn<F> {
    f: F,
    subscribers: SharedSubscriberManager,
    guards: Vec<SubscribeGuard<BoxReactive>>,
}

impl<F> ComputeFn<F> {
    pub fn new(f: F) -> Self {
        Self {
            f,
            subscribers: SharedSubscriberManager::default(),
            guards: Vec::new(),
        }
    }

    pub fn depend(&mut self, value: impl Reactive + 'static) {
        let subscribers = self.subscribers.clone();
        let id = value.register_subscriber(Subscriber::new(move |metadata| {
            subscribers.notify(metadata)
        }));

        if id.is_some() {
            let guard: SubscribeGuard<BoxReactive> = SubscribeGuard::new(Box::new(value), id);

            self.guards.push(guard);
        }
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
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<SubscriberId> {
        Some(self.subscribers.register(subscriber))
    }
    fn cancel_subscriber(&self, id: SubscriberId) {
        self.subscribers.cancel(id)
    }
}
