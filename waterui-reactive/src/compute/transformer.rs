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
    F: 'static + Send + Sync + Fn(C::Output) -> Output,
    C: Compute,
    C::Output: 'static,
{
    type Output = Output;

    fn compute(&self) -> Self::Output {
        (self.transformer)(self.source.compute())
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> usize {
        self.source.register_subscriber(subscriber)
    }

    fn cancel_subscriber(&self, id: usize) {
        self.source.cancel_subscriber(id)
    }

    fn computed(self) -> Computed<Self::Output> {
        Computed::new(ComputeTransformer {
            source: self.source.computed(),
            transformer: self.transformer,
        })
    }
}
