use waterui_reactive::{
    Compute, Computed,
    compute::{ComputeResult, WithMetadata},
    map::Map,
    watcher::{Watcher, WatcherGuard},
    zip::Zip,
};

use waterui_core::animation::Animation;

/// Extension trait that provides utility methods for types implementing the `Compute` trait
pub trait ComputeExt: Compute + Sized {
    /// Maps the output of a computation using the provided function
    ///
    /// This creates a new computation that applies the function to the result of this computation
    fn map<F, Output>(self, f: F) -> Map<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Output,
        Output: ComputeResult,
        Self: 'static;

    /// Combines this computation with another computation
    ///
    /// The result will be a tuple containing the results of both computations
    fn zip<B: Compute>(self, b: B) -> Zip<Self, B>;

    /// Registers a watcher for this computation
    ///
    /// The watcher will be notified whenever the computation's result changes
    fn watch(&self, watcher: impl Watcher<Self::Output>) -> WatcherGuard;

    /// Creates a memoized version of this computation
    ///
    /// The result will be cached and only recomputed when dependencies change
    fn computed(self) -> Computed<Self::Output>
    where
        Self: Clone + 'static;

    /// Attaches metadata to this computation
    ///
    /// This can be used to associate additional information with a computation
    fn with<T>(self, metadata: T) -> WithMetadata<Self, T>;

    /// Creates an animated version of this computation
    ///
    /// The result will automatically use the default animation settings
    fn animated(self) -> impl Compute<Output = Self::Output>;
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
    fn watch(&self, watcher: impl Watcher<Self::Output>) -> WatcherGuard {
        self.watch(watcher)
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
    fn animated(self) -> impl Compute<Output = Self::Output> {
        self.with(Animation::Default)
    }
}
