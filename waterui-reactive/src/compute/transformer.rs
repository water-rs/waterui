use core::num::NonZeroUsize;

use crate::{Compute, Computed, Subscriber};

pub struct ComputeTransformer<C, F> {
    source: C,
    transformer: F,
}

impl<C, F> ComputeTransformer<C, F> {
    pub fn new(source: C, transformer: F) -> Self {
        Self {
            source,
            transformer,
        }
    }
}

impl<C, F, Output> Compute for ComputeTransformer<C, F>
where
    F: 'static + Fn(C::Output) -> Output,
    C: Compute,
    C::Output: 'static,
{
    type Output = Output;

    fn compute(&self) -> Self::Output {
        (self.transformer)(self.source.compute())
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        self.source.register_subscriber(subscriber)
    }

    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.source.cancel_subscriber(id)
    }

    fn computed(self) -> Computed<Self::Output> {
        Computed::new(ComputeTransformer {
            source: self.source.computed(),
            transformer: self.transformer,
        })
    }
}
