use core::{any::type_name, fmt::Debug, marker::PhantomData};

use alloc::boxed::Box;

use crate::{
    constant,
    map::Map,
    watcher::{Watcher, WatcherGuard},
};

pub trait Compute: Clone {
    type Output;
    fn compute(&self) -> Self::Output;
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard;
}

pub trait ToCompute<Output> {
    fn to_compute(self) -> impl Compute<Output = Output>;
}

pub trait ToComputed<Output>: ToCompute<Output> + 'static {
    fn to_computed(self) -> Computed<Output>;
}

impl<C, Output> ToCompute<Output> for C
where
    C: Compute,
    Output: From<C::Output> + 'static,
{
    fn to_compute(self) -> impl Compute<Output = Output> {
        ComputeAdapter::<C, C::Output, Output>::new(self)
    }
}

impl<C, Output> ToComputed<Output> for C
where
    C: ToCompute<Output> + 'static,
{
    fn to_computed(self) -> Computed<Output> {
        self.to_compute().computed()
    }
}
struct ComputeAdapter<C, T, Output> {
    compute: C,
    _marker: PhantomData<(T, Output)>,
}

impl<C: Clone, T, Output> Clone for ComputeAdapter<C, T, Output> {
    fn clone(&self) -> Self {
        Self {
            compute: self.compute.clone(),
            _marker: PhantomData,
        }
    }
}

impl<C, T, Output> ComputeAdapter<C, T, Output> {
    fn new(compute: C) -> Self {
        Self {
            compute,
            _marker: PhantomData,
        }
    }
}

impl<C, T, Output> Compute for ComputeAdapter<C, T, Output>
where
    C: Compute<Output = T>,
    Output: From<T> + 'static,
{
    type Output = Output;
    fn compute(&self) -> Self::Output {
        self.compute.compute().into()
    }

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        self.compute
            .add_watcher(Watcher::new(move |value: T, metadata| {
                watcher.notify_with_metadata(value.into(), metadata)
            }))
    }
}

trait ComputedImpl {
    type Output;
    fn compute(&self) -> Self::Output;
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard;
    fn cloned(&self) -> Computed<Self::Output>;
}

impl<C: Compute + 'static> ComputedImpl for C
where
    C::Output: 'static,
{
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        Compute::compute(self)
    }

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        Compute::add_watcher(self, watcher)
    }
    fn cloned(&self) -> Computed<Self::Output> {
        Computed::new(self.clone())
    }
}

pub struct Computed<T: 'static>(Box<dyn ComputedImpl<Output = T>>);

impl<T> Debug for Computed<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<T> Compute for Computed<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.0.compute()
    }

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        self.0.add_watcher(watcher)
    }
}

impl<T> Clone for Computed<T> {
    fn clone(&self) -> Self {
        self.0.cloned()
    }
}

impl<T> Computed<T> {
    pub fn new(value: impl Compute<Output = T> + 'static) -> Self {
        Self(Box::new(value))
    }
}

impl<T: Clone> Computed<T> {
    pub fn constant(value: T) -> Self {
        Self::new(constant(value))
    }
}

pub trait ComputeExt: Compute {
    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard;
    fn map<F, Output>(&self, f: F) -> Computed<Output>
    where
        Self: 'static,
        F: 'static + Fn(Self::Output) -> Output;

    fn computed(&self) -> Computed<Self::Output>
    where
        Self: 'static;
}

impl<C: Compute> ComputeExt for C {
    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        self.add_watcher(watcher.into())
    }
    fn map<F, Output>(&self, f: F) -> Computed<Output>
    where
        Self: 'static,
        F: 'static + Fn(Self::Output) -> Output,
    {
        Computed::new(Map::new(self.clone(), f))
    }

    fn computed(&self) -> Computed<Self::Output>
    where
        Self: 'static,
    {
        Computed::new(self.clone())
    }
}
