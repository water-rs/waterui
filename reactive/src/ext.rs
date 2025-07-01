use crate::{
    Compute, Computed, cache::Cached, compute::WithMetadata, map::Map, watcher::WatcherGuard,
    zip::Zip,
};

/// Extension trait providing convenient methods for reactive computations.
///
/// This trait adds useful combinators and utility methods to any type
/// that implements `Compute`, making reactive programming more ergonomic.
pub trait ComputeExt: Compute + Sized {
    /// Transforms the output of this computation using a mapping function.
    ///
    /// Creates a new computation that applies the function `f` to every
    /// value produced by this computation.
    fn map<F, Output>(self, f: F) -> Map<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Output,
        Self: 'static,
    {
        Map::new(self, f)
    }

    /// Combines this computation with another, producing a tuple of both values.
    ///
    /// The resulting computation updates whenever either input computation changes.
    fn zip<B: Compute>(self, b: B) -> Zip<Self, B> {
        Zip::new(self, b)
    }

    /// Attaches a watcher function to observe changes in this computation.
    ///
    /// The watcher function is called whenever the computation's value changes.
    fn watch(&self, watcher: impl Fn(Self::Output) + 'static) -> WatcherGuard {
        self.add_watcher(move |value, _| watcher(value))
    }

    /// Creates a cached version of this computation.
    ///
    /// The cached computation stores the last computed value and only
    /// recomputes when the source changes.
    fn cached(self) -> Cached<Self>
    where
        Self::Output: Clone,
    {
        Cached::new(self)
    }

    /// Converts this computation into a `Computed<T>`.
    ///
    /// This provides a type-erased interface for the computation,
    /// useful when you need to store computations of different types.
    fn computed(self) -> Computed<Self::Output>
    where
        Self: 'static,
    {
        Computed::new(self)
    }

    /// Attaches metadata to this computation.
    ///
    /// The metadata can be used to provide additional context or
    /// debugging information about the computation.
    fn with<T>(self, metadata: T) -> WithMetadata<Self, T> {
        WithMetadata::new(metadata, self)
    }
}

impl<C: Compute + Sized> ComputeExt for C {}
