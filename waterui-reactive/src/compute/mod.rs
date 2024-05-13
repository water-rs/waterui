use core::any::type_name;
use core::fmt::Debug;
use core::ops::Deref;

use alloc::rc::{Rc, Weak};
mod constant;
pub use constant::{constant, Constant};
mod f;
pub use f::ComputeFn;
mod map;

use crate::reactive::ReactiveExt;
use crate::subscriber::{SubscribeGuard, Subscriber};
use crate::subscriber::{SubscriberId, SubscriberManager};
use crate::Reactive;
pub use map::Map;

pub trait Compute: Reactive {
    type Output;
    fn compute(&self) -> Self::Output;
}

pub trait IntoCompute<T> {
    fn into_compute(self) -> impl Compute<Output = T>;
}

impl<C, T> IntoCompute<T> for C
where
    C: Compute,
    T: From<C::Output>,
{
    fn into_compute(self) -> impl Compute<Output = T> {
        self.map(Into::into)
    }
}

pub trait IntoComputed<T>: IntoCompute<T> + 'static {
    fn into_computed(self) -> Computed<T>;
}

impl<C, T> IntoComputed<T> for C
where
    C: IntoCompute<T> + 'static,
    T: 'static,
{
    fn into_computed(self) -> Computed<T> {
        self.into_compute().computed()
    }
}

impl<C: Compute> Compute for &C {
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        (*self).compute()
    }
}

impl<C: Compute> Compute for &mut C {
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        (**self).compute()
    }
}

impl<C: Compute> Compute for Rc<C> {
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        self.deref().compute()
    }
}

impl<C: Compute> Compute for Option<C> {
    type Output = Option<C::Output>;

    fn compute(&self) -> Self::Output {
        self.as_ref().map(C::compute)
    }
}

pub trait ComputeExt: Compute {
    fn map<F, Output>(self, f: F) -> Map<Self, F>
    where
        F: Fn(Self::Output) -> Output,
        Self: Sized;
    fn computed(self) -> Computed<Self::Output>
    where
        Self: Sized + 'static;
}

impl<C: Compute> ComputeExt for C {
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

pub struct Computed<T>(Rc<dyn Compute<Output = T>>);

impl<T> Compute for Computed<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.0.compute()
    }
}

impl<T> Clone for Computed<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Debug for Computed<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

pub struct WeakComputed<T>(Weak<dyn Compute<Output = T>>);

impl<T> WeakComputed<T> {
    pub fn upgrade(&self) -> Option<Computed<T>> {
        self.0.upgrade().map(Computed)
    }
}

impl<T> Computed<T> {
    pub fn new(compute: impl Compute<Output = T> + 'static) -> Self {
        Self(Rc::new(compute))
    }

    pub fn downgrade(&self) -> WeakComputed<T> {
        WeakComputed(Rc::downgrade(&self.0))
    }

    pub fn watch(&self, watcher: impl Fn(T) + 'static) -> SubscribeGuard<&Self>
    where
        T: 'static,
    {
        let weak = self.downgrade();
        self.subscribe(move || {
            if let Some(rc) = weak.upgrade() {
                watcher(rc.compute())
            }
        })
    }

    pub fn from_fn<F>(f: F) -> Self
    where
        F: 'static + Fn(&SubscriberManager) -> T,
    {
        Self::new(ComputeFn::new(f))
    }
}

impl<T> Reactive for Computed<T> {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<SubscriberId> {
        self.0.register_subscriber(subscriber)
    }
    fn cancel_subscriber(&self, id: SubscriberId) {
        self.0.cancel_subscriber(id)
    }
}
