use core::num::NonZeroUsize;

use crate::Subscriber;

pub trait Reactive {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize>;
    fn cancel_subscriber(&self, id: NonZeroUsize);
    fn notify(&self);
}

impl<T: Reactive> Reactive for &T {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        (*self).register_subscriber(subscriber)
    }

    fn cancel_subscriber(&self, id: NonZeroUsize) {
        (*self).cancel_subscriber(id)
    }

    fn notify(&self) {
        (*self).notify()
    }
}

impl<T: Reactive> Reactive for Option<T> {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        self.as_ref()
            .and_then(|c| c.register_subscriber(subscriber))
    }

    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.as_ref().inspect(|c| c.cancel_subscriber(id));
    }

    fn notify(&self) {
        self.as_ref().inspect(|c| c.notify());
    }
}
