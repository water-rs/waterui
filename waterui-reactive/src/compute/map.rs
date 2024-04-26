use core::num::NonZeroUsize;

use crate::{Compute, Reactive, Subscriber};

pub struct Map<C, F> {
    source: C,
    f: F,
}

impl<C, F> Map<C, F> {
    pub fn new(source: C, f: F) -> Self {
        Self { source, f }
    }
}

impl<C, F, Output> Compute for Map<C, F>
where
    F: Fn(C::Output) -> Output,
    C: Compute,
{
    type Output = Output;

    fn compute(&self) -> Self::Output {
        (self.f)(self.source.compute())
    }
}

impl<C: Compute, F> Reactive for Map<C, F> {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        self.source.register_subscriber(subscriber)
    }

    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.source.cancel_subscriber(id);
    }

    fn notify(&self) {
        self.source.notify();
    }
}
