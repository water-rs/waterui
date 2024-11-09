mod computed;
mod impls;
pub use computed::*;

use crate::{
    map::Map,
    watcher::{Watcher, WatcherGuard},
};

pub trait ComputeResult: 'static + Clone + PartialEq {}

impl<T: 'static + Clone + PartialEq> ComputeResult for T {}

pub trait Compute: Clone {
    const CONSTANT: bool = false;
    type Output: ComputeResult;
    fn compute(&self) -> Self::Output;
    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard;
}

pub trait ToCompute<Output: ComputeResult> {
    fn to_compute(self) -> impl Compute<Output = Output>;
}

pub trait ToComputed<Output: ComputeResult>: ToCompute<Output> + 'static {
    fn to_computed(self) -> Computed<Output>;
}

impl<C, Output> ToCompute<Output> for C
where
    C: Compute + 'static,
    C::Output: 'static,
    Output: From<C::Output> + ComputeResult,
{
    fn to_compute(self) -> impl Compute<Output = Output> {
        self.map(Into::into)
    }
}

impl<C, Output> ToComputed<Output> for C
where
    C: ToCompute<Output> + 'static,
    Output: ComputeResult,
{
    fn to_computed(self) -> Computed<Output> {
        self.to_compute().computed()
    }
}

pub trait ComputeExt: Compute + 'static {
    fn map<F, Output>(&self, f: F) -> Map<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Output,
        Output: ComputeResult;

    fn computed(&self) -> Computed<Self::Output>;
    fn with<T>(&self, metadata: T) -> WithMetadata<Self, T>;
}

#[derive(Debug, Clone)]
pub struct WithMetadata<C, T> {
    metadata: T,
    compute: C,
}

impl<C, T> WithMetadata<C, T> {
    pub fn new(metadata: T, compute: C) -> Self {
        Self { metadata, compute }
    }
}

impl<C: Compute, T: Clone + 'static> Compute for WithMetadata<C, T> {
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        self.compute.compute()
    }

    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        let watcher: Watcher<_> = watcher.into();
        let with = self.metadata.clone();
        self.compute.watch(Watcher::new(move |value, metadata| {
            watcher.notify_with_metadata(value, metadata.with(with.clone()));
        }))
    }
}

impl<C: Compute + 'static> ComputeExt for C {
    fn map<F, Output>(&self, f: F) -> Map<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Output,
        Output: ComputeResult,
    {
        Map::new(self.clone(), f)
    }

    fn computed(&self) -> Computed<Self::Output> {
        Computed::new(self.clone())
    }

    fn with<T>(&self, metadata: T) -> WithMetadata<Self, T> {
        WithMetadata::new(metadata, self.clone())
    }
}
