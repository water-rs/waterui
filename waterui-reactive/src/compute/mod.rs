use core::fmt::Debug;
use core::{any::type_name, num::NonZeroUsize};

mod grouped;
pub use grouped::GroupedCompute;

use alloc::boxed::Box;
mod impls;
mod map;
use crate::{subscriber::SubscriberManager, Subscriber};
pub use map::Map;

pub trait IntoCompute<T>: Compute {
    fn into_compute(self) -> impl Compute<Output = T>;
}

pub trait IntoComputed<T>: IntoCompute<T> + 'static {
    fn into_computed(self) -> Computed<T>;
}

impl<C, T> IntoCompute<T> for C
where
    C: Compute,
    C::Output: Into<T>,
{
    fn into_compute(self) -> impl Compute<Output = T> {
        self.map(Into::into)
    }
}

impl<C, T> IntoComputed<T> for C
where
    C: Compute + 'static,
    C::Output: Into<T>,
    T: 'static,
{
    fn into_computed(self) -> Computed<T> {
        Computed::new(self.into_compute())
    }
}

pub trait Compute {
    type Output;
    fn compute(&self) -> Self::Output;
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize>;
    fn cancel_subscriber(&self, id: NonZeroUsize);
    fn notify(&self);
}

impl<C: Compute + 'static> Compute for &C {
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        (*self).compute()
    }

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

impl<C: Compute + 'static> Compute for Option<C> {
    type Output = Option<C::Output>;

    fn compute(&self) -> Self::Output {
        self.as_ref().map(C::compute)
    }

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

pub trait ComputeExt: Compute {
    fn subscribe(&self, subscriber: impl Fn() + 'static) -> SubscribeGuard<'_, Self>;
    fn watch(&self, watcher: impl Fn(Self::Output) + 'static) -> SubscribeGuard<'_, Self>
    where
        Self: 'static + Clone;

    fn map<F, Output>(self, f: F) -> Map<Self, F>
    where
        F: Fn(Self::Output) -> Output,
        Self: Sized;
    fn computed(self) -> Computed<Self::Output>
    where
        Self: Sized + 'static;
}

impl<C: Compute> ComputeExt for C {
    fn subscribe(&self, subscriber: impl Fn() + 'static) -> SubscribeGuard<'_, Self> {
        SubscribeGuard::new(self, self.register_subscriber(Box::new(subscriber)))
    }

    fn watch(&self, watcher: impl Fn(Self::Output) + 'static) -> SubscribeGuard<'_, Self>
    where
        Self: 'static + Clone,
    {
        let this = self.clone();
        self.subscribe(move || watcher(this.compute()))
    }

    fn map<F, Output>(self, f: F) -> Map<Self, F>
    where
        F: Fn(Self::Output) -> Output,
        Self: Sized,
    {
        Map::new(self, f)
    }

    fn computed(self) -> Computed<Self::Output>
    where
        Self: Sized + 'static,
    {
        Computed::new(self)
    }
}

pub struct Computed<T> {
    inner: Box<dyn Compute<Output = T>>,
}

impl<T> Debug for Computed<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<T> Computed<T> {
    pub fn new(compute: impl Compute<Output = T> + 'static) -> Self {
        Self {
            inner: Box::new(compute),
        }
    }
}

impl<T: Clone + 'static> Computed<T> {
    pub fn constant(value: T) -> Self {
        Self::new(Constant::new(value))
    }
}

impl<T> Compute for Computed<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.inner.compute()
    }
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        self.inner.register_subscriber(subscriber)
    }
    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.inner.cancel_subscriber(id)
    }

    fn notify(&self) {
        self.inner.notify()
    }
}

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
}

impl<T, F> Compute for ComputeFn<F>
where
    F: Fn(&SubscriberManager) -> T,
{
    type Output = T;
    fn compute(&self) -> Self::Output {
        (self.f)(&self.subscribers)
    }
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        Some(self.subscribers.register(subscriber))
    }
    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.subscribers.cancel(id)
    }

    fn notify(&self) {
        self.subscribers.notify();
    }
}

pub struct Constant<T> {
    value: T,
}

impl<T> Constant<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

impl<T: Clone> Compute for Constant<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.value.clone()
    }
    fn register_subscriber(&self, _subscriber: Subscriber) -> Option<NonZeroUsize> {
        None
    }
    fn cancel_subscriber(&self, _id: NonZeroUsize) {}
    fn notify(&self) {}
}

#[must_use]
pub struct SubscribeGuard<'a, V: ?Sized>
where
    V: Compute,
{
    source: &'a V,
    id: Option<NonZeroUsize>,
}

impl<'a, V> SubscribeGuard<'a, V>
where
    V: Compute,
{
    pub fn new(source: &'a V, id: Option<NonZeroUsize>) -> Self {
        Self { source, id }
    }
}

impl<'a, V> Drop for SubscribeGuard<'a, V>
where
    V: Compute + ?Sized,
{
    fn drop(&mut self) {
        self.id.inspect(|id| self.source.cancel_subscriber(*id));
    }
}
