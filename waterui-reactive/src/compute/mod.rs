use core::fmt::Debug;
use core::{any::type_name, num::NonZeroUsize};

mod grouped;
pub use grouped::GroupedCompute;

use alloc::boxed::Box;
mod constant;
pub use constant::Constant;

mod f;
pub use f::ComputeFn;
mod map;
use crate::subscriber::SubscriberManager;
use crate::subscriber::{BoxSubscriber, SubscribeGuard};
use crate::Reactive;
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

pub trait Compute: Reactive {
    type Output;
    fn compute(&self) -> Self::Output;
}

impl<C: Compute + 'static> Compute for &C {
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        (*self).compute()
    }
}

impl<C: Compute + 'static> Compute for Option<C> {
    type Output = Option<C::Output>;

    fn compute(&self) -> Self::Output {
        self.as_ref().map(C::compute)
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

    pub fn from_fn<F>(f: F) -> Self
    where
        F: 'static + Fn(&SubscriberManager) -> T,
    {
        Self::new(ComputeFn::new(f))
    }

    pub fn from_fn_with_subscribers<F>(f: F, subscribers: SubscriberManager) -> Self
    where
        F: 'static + Fn(&SubscriberManager) -> T,
    {
        Self::new(ComputeFn::new_with_subscribers(f, subscribers))
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
}

impl<T> Reactive for Computed<T> {
    fn register_subscriber(&self, subscriber: BoxSubscriber) -> Option<NonZeroUsize> {
        self.inner.register_subscriber(subscriber)
    }
    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.inner.cancel_subscriber(id)
    }

    fn notify(&self) {
        self.inner.notify()
    }
}
