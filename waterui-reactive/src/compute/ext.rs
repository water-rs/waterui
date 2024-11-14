use crate::{
    compute::{ComputeResult, WithMetadata},
    map::Map,
    watcher::{Watcher, WatcherGuard},
    zip::Zip,
    Compute, Computed,
};

pub trait ComputeExt: Compute + Sized {
    fn map<F, Output>(self, f: F) -> Map<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Output,
        Output: ComputeResult,
        Self: 'static;

    fn zip<B: Compute>(self, b: B) -> Zip<Self, B>;

    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard;

    fn computed(self) -> Computed<Self::Output>
    where
        Self: Clone + 'static;
    fn with<T>(self, metadata: T) -> WithMetadata<Self, T>;
}

impl<C: Compute> ComputeExt for C {
    fn map<F, Output>(self, f: F) -> Map<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Output,
        Output: ComputeResult,
        Self: 'static,
    {
        Map::new(self, f)
    }

    fn zip<B: Compute>(self, b: B) -> Zip<Self, B> {
        Zip::new(self, b)
    }
    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        self.add_watcher(watcher.into())
    }

    fn computed(self) -> Computed<Self::Output>
    where
        Self: Clone + 'static,
    {
        Computed::new(self)
    }

    fn with<T>(self, metadata: T) -> WithMetadata<Self, T> {
        WithMetadata::new(metadata, self)
    }
}
