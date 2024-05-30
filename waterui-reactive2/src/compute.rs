use core::future::Future;

use alloc::boxed::Box;
use waterui_core::Environment;

use crate::{
    constant,
    map::{AsyncMap, Map},
    watcher::{Watcher, WatcherGuard},
};

pub trait Compute: Clone {
    type Output: 'static;
    fn compute(&self) -> Self::Output;
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard;
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
    fn map<F, Output>(&self, f: F) -> Map<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Output;

    fn async_map<F, Fut, Output>(&self, env: &Environment, f: F) -> AsyncMap<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Fut,
        Fut: Future<Output = Output>;

    fn computed(&self) -> Computed<Self::Output>
    where
        Self: 'static;
}

impl<C: Compute> ComputeExt for C {
    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        self.add_watcher(watcher.into())
    }
    fn map<F, Output>(&self, f: F) -> Map<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Output,
    {
        Map::new(self.clone(), f)
    }
    fn async_map<F, Fut, Output>(&self, env: &Environment, f: F) -> AsyncMap<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Fut,
        Fut: Future<Output = Output>,
    {
        AsyncMap::new(self.clone(), env.clone(), f)
    }

    fn computed(&self) -> Computed<Self::Output>
    where
        Self: 'static,
    {
        Computed::new(self.clone())
    }
}
