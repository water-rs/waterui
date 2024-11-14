mod computed;
mod ext;
pub use computed::*;
pub use ext::ComputeExt;

use crate::{
    map::Map,
    watcher::{Watcher, WatcherGuard},
};

pub trait ComputeResult: 'static + Clone + PartialEq {}

impl<T: 'static + Clone + PartialEq> ComputeResult for T {}

impl<T: ComputeResult> Compute for T {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.clone()
    }
    fn add_watcher(&self, _watcher: Watcher<Self::Output>) -> WatcherGuard {
        WatcherGuard::new(|| {})
    }
}

pub trait Compute: Clone + 'static {
    type Output: ComputeResult;
    fn compute(&self) -> Self::Output;
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard;
}

pub trait IntoCompute<Output: ComputeResult> {
    type Compute: Compute<Output = Output>;
    fn into_compute(self) -> Self::Compute;
}

pub trait IntoComputed<Output: ComputeResult>: IntoCompute<Output> + 'static {
    fn into_computed(self) -> Computed<Output>;
}

impl<C, Output> IntoCompute<Output> for C
where
    C: Compute + 'static,

    C::Output: 'static,
    Output: From<C::Output> + ComputeResult,
{
    type Compute = Map<C, fn(C::Output) -> Output, Output>;
    fn into_compute(self) -> Self::Compute {
        self.map(Into::into)
    }
}

impl<C, Output> IntoComputed<Output> for C
where
    C: IntoCompute<Output> + 'static,
    C::Compute: Clone,
    Output: ComputeResult,
{
    fn into_computed(self) -> Computed<Output> {
        self.into_compute().computed()
    }
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

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        let with = self.metadata.clone();
        self.compute
            .add_watcher(Watcher::new(move |value, metadata| {
                watcher.notify_with_metadata(value, metadata.with(with.clone()));
            }))
    }
}
