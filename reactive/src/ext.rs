use crate::{Compute, Computed, compute::WithMetadata, map::Map, watcher::WatcherGuard, zip::Zip};

pub trait ComputeExt: Compute + Sized {
    fn map<F, Output>(self, f: F) -> Map<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Output,
        Self: 'static,
    {
        Map::new(self, f)
    }

    fn zip<B: Compute>(self, b: B) -> Zip<Self, B> {
        Zip::new(self, b)
    }

    fn watch(&self, watcher: impl Fn(Self::Output) + 'static) -> WatcherGuard {
        self.add_watcher(move |value, _| watcher(value))
    }

    fn computed(self) -> Computed<Self::Output>
    where
        Self: 'static,
    {
        Computed::new(self)
    }

    fn with<T>(self, metadata: T) -> WithMetadata<Self, T> {
        WithMetadata::new(metadata, self)
    }
}

impl<C: Compute + Sized> ComputeExt for C {}
