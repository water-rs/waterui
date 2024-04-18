use core::{
    fmt::{Debug, Display},
    ops::Deref,
};

mod grouped;
use alloc::{boxed::Box, rc::Rc};
pub use grouped::GroupedCompute;
mod impls;
mod transformer;
use crate::{
    subscriber::{SharedSubscriberManager, SubscribeGuard, SubscriberManager},
    Subscriber,
};

use self::transformer::ComputeTransformer;

pub trait Compute {
    type Output;
    fn compute(&self) -> Self::Output;
    fn register_subscriber(&self, subscriber: Subscriber) -> usize;
    fn cancel_subscriber(&self, id: usize);
    fn computed(self) -> Computed<Self::Output>;
}

impl<C: Compute + Clone + 'static> Compute for &C {
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        (*self).compute()
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> usize {
        (*self).register_subscriber(subscriber)
    }

    fn cancel_subscriber(&self, id: usize) {
        (*self).cancel_subscriber(id)
    }

    fn computed(self) -> Computed<Self::Output> {
        Computed::new(self.clone())
    }
}

pub trait ComputeExt: Compute {
    fn subscribe(&self, subscriber: impl Fn() + 'static) -> SubscribeGuard<'_, Self>;
    fn transform<Output>(
        &self,
        transformer: impl 'static + Fn(Self::Output) -> Output,
    ) -> impl Compute<Output = Output>
    where
        Self::Output: 'static,
        Self: Clone;
}

impl<C: Compute> ComputeExt for C {
    fn subscribe(&self, subscriber: impl Fn() + 'static) -> SubscribeGuard<'_, Self> {
        SubscribeGuard::new(self, self.register_subscriber(Box::new(subscriber)))
    }

    fn transform<Output>(
        &self,
        transformer: impl 'static + Fn(Self::Output) -> Output,
    ) -> impl Compute<Output = Output>
    where
        Self::Output: 'static,
        Self: Clone,
    {
        ComputeTransformer::new(self.clone().computed(), transformer)
    }
}

pub struct Computed<T> {
    inner: Box<dyn Compute<Output = T>>,
}

impl<T> Computed<T> {
    pub fn from_fn(compute: impl 'static + Fn() -> T) -> (Computed<T>, SharedSubscriberManager) {
        let subscribers = Rc::new(SubscriberManager::new());
        (
            Computed::new(ComputeImpl {
                compute: Rc::new(compute),
                subscribers: subscribers.clone(),
            }),
            subscribers,
        )
    }

    pub fn from_fn_with_subscribers(
        compute: impl 'static + Fn() -> T,
        subscribers: SharedSubscriberManager,
    ) -> Computed<T> {
        Computed::new(ComputeImpl {
            compute: Rc::new(compute),
            subscribers: subscribers.clone(),
        })
    }

    pub fn into_inner(self) -> Box<dyn Compute<Output = T>> {
        self.inner
    }
}

impl<T: Debug + 'static> Debug for Computed<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Compute::compute(self).fmt(f)
    }
}

impl<T: Display + 'static> Display for Computed<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Compute::compute(self).fmt(f)
    }
}

struct ComputeImpl<F> {
    compute: Rc<F>,
    subscribers: SharedSubscriberManager,
}

impl<F, T> Compute for ComputeImpl<F>
where
    F: 'static + Fn() -> T,
{
    type Output = T;
    fn compute(&self) -> T {
        (self.compute)()
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> usize {
        self.subscribers.register(subscriber)
    }

    fn cancel_subscriber(&self, id: usize) {
        self.subscribers.cancel(id)
    }

    fn computed(self) -> Computed<Self::Output> {
        Computed::new(self)
    }
}

struct ConsantCompute<T> {
    value: T,
}

impl<T> ConsantCompute<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

impl<T: Clone + 'static> Compute for ConsantCompute<T> {
    type Output = T;

    fn compute(&self) -> Self::Output {
        self.value.clone()
    }

    fn register_subscriber(&self, _subscriber: Subscriber) -> usize {
        0
    }

    fn cancel_subscriber(&self, _id: usize) {}

    fn computed(self) -> Computed<Self::Output> {
        Computed::new(self)
    }
}

impl<T: Clone + 'static> Computed<T> {
    pub fn constant(value: T) -> Self {
        Computed::new(ConsantCompute::new(value))
    }
}

impl<T> Compute for Computed<T> {
    type Output = T;
    fn compute(&self) -> T {
        Compute::compute(self.inner.deref())
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> usize {
        Compute::register_subscriber(self.inner.deref(), subscriber)
    }

    fn cancel_subscriber(&self, id: usize) {
        Compute::cancel_subscriber(self.inner.deref(), id)
    }

    fn computed(self) -> Computed<Self::Output> {
        self
    }
}

impl<T> Computed<T> {
    pub fn new(compute: impl Compute<Output = T> + 'static) -> Self {
        Self {
            inner: Box::new(compute),
        }
    }
}
