use core::ops::Deref;

use alloc::{
    boxed::Box,
    rc::{Rc, Weak},
};

use crate::subscriber::{SubscribeGuard, Subscriber, SubscriberId};

pub trait Reactive {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<SubscriberId>;
    fn cancel_subscriber(&self, id: SubscriberId);
}

pub type BoxReactive = Box<dyn Reactive>;

pub trait ReactiveExt: Reactive {
    fn subscribe(&self, subscriber: impl Into<Subscriber>) -> SubscribeGuard<&Self>
    where
        Self: Sized;
}

impl<T: Reactive> ReactiveExt for T {
    fn subscribe(&self, subscriber: impl Into<Subscriber>) -> SubscribeGuard<&Self> {
        SubscribeGuard::new(self, self.register_subscriber(subscriber.into()))
    }
}

impl<T: Reactive + ?Sized> Reactive for &T {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<SubscriberId> {
        (*self).register_subscriber(subscriber)
    }

    fn cancel_subscriber(&self, id: SubscriberId) {
        (*self).cancel_subscriber(id)
    }
}

impl<T: Reactive + ?Sized> Reactive for Box<T> {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<SubscriberId> {
        self.deref().register_subscriber(subscriber)
    }
    fn cancel_subscriber(&self, id: SubscriberId) {
        self.deref().cancel_subscriber(id)
    }
}

impl<T: Reactive + ?Sized> Reactive for Rc<T> {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<SubscriberId> {
        self.deref().register_subscriber(subscriber)
    }
    fn cancel_subscriber(&self, id: SubscriberId) {
        self.deref().cancel_subscriber(id)
    }
}

impl<T: Reactive + ?Sized> Reactive for Weak<T> {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<SubscriberId> {
        if let Some(rc) = self.upgrade() {
            rc.register_subscriber(subscriber)
        } else {
            None
        }
    }
    fn cancel_subscriber(&self, id: SubscriberId) {
        if let Some(rc) = self.upgrade() {
            rc.cancel_subscriber(id)
        }
    }
}

impl<T: Reactive + ?Sized> Reactive for &mut T {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<SubscriberId> {
        (**self).register_subscriber(subscriber)
    }

    fn cancel_subscriber(&self, id: SubscriberId) {
        (**self).cancel_subscriber(id)
    }
}

impl<T: Reactive> Reactive for Option<T> {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<SubscriberId> {
        self.as_ref()
            .and_then(|c| c.register_subscriber(subscriber))
    }

    fn cancel_subscriber(&self, id: SubscriberId) {
        self.as_ref().inspect(|c| c.cancel_subscriber(id));
    }
}

impl<T: Reactive, E: Reactive> Reactive for Result<T, E> {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<SubscriberId> {
        match self {
            Ok(v) => v.register_subscriber(subscriber),
            Err(v) => v.register_subscriber(subscriber),
        }
    }

    fn cancel_subscriber(&self, id: SubscriberId) {
        match self {
            Ok(v) => v.cancel_subscriber(id),
            Err(v) => v.cancel_subscriber(id),
        }
    }
}
