use core::num::NonZeroUsize;

use crate::subscriber::BoxSubscriber;

pub trait Reactive {
    fn register_subscriber(&self, subscriber: BoxSubscriber) -> Option<NonZeroUsize>;
    fn cancel_subscriber(&self, id: NonZeroUsize);
    fn notify(&self);
}

impl<T: Reactive> Reactive for &T {
    fn register_subscriber(&self, subscriber: BoxSubscriber) -> Option<NonZeroUsize> {
        (*self).register_subscriber(subscriber)
    }

    fn cancel_subscriber(&self, id: NonZeroUsize) {
        (*self).cancel_subscriber(id)
    }

    fn notify(&self) {
        (*self).notify()
    }
}

impl<T: Reactive> Reactive for &mut T {
    fn register_subscriber(&self, subscriber: BoxSubscriber) -> Option<NonZeroUsize> {
        (**self).register_subscriber(subscriber)
    }

    fn cancel_subscriber(&self, id: NonZeroUsize) {
        (**self).cancel_subscriber(id)
    }

    fn notify(&self) {
        (**self).notify()
    }
}

impl<T: Reactive> Reactive for Option<T> {
    fn register_subscriber(&self, subscriber: BoxSubscriber) -> Option<NonZeroUsize> {
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

impl<T: Reactive, E: Reactive> Reactive for Result<T, E> {
    fn register_subscriber(&self, subscriber: BoxSubscriber) -> Option<NonZeroUsize> {
        match self {
            Ok(v) => v.register_subscriber(subscriber),
            Err(v) => v.register_subscriber(subscriber),
        }
    }

    fn cancel_subscriber(&self, id: NonZeroUsize) {
        match self {
            Ok(v) => v.cancel_subscriber(id),
            Err(v) => v.cancel_subscriber(id),
        }
    }

    fn notify(&self) {
        match self {
            Ok(v) => v.notify(),
            Err(v) => v.notify(),
        }
    }
}
