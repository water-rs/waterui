use crate::{
    Compute, Computed,
    compute::{ComputeResult, Unique, WithMetadata},
    map::Map,
    watcher::{Watcher, WatcherGuard},
    zip::Zip,
};

/// Extension trait that provides utility methods for types implementing the `Compute` trait.
///
/// This trait adds several convenience methods to make working with compute nodes easier
/// and more composable.
pub trait ComputeExt: Compute + Sized {
    /// Transforms the output of this compute node using the provided function.
    ///
    /// # Arguments
    ///
    /// * `f` - A function that takes the output of this compute node and returns a new value
    ///
    /// # Returns
    ///
    /// A new compute node that will apply the transformation function to the output of this node.
    ///
    /// # Type Parameters
    ///
    /// * `F` - The transformation function type
    /// * `Output` - The type returned by the transformation function
    fn map<F, Output>(self, f: F) -> Map<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Output,
        Output: ComputeResult,
        Self: 'static;
    fn unique_map<F, Output>(self, f: F) -> impl Compute<Output = Unique<Output>>
    where
        Output: Clone + 'static,
        F: 'static + Fn(Self::Output) -> Output,
        Self: 'static;

    /// Combines this compute node with another one into a single node that produces both outputs.
    ///
    /// # Arguments
    ///
    /// * `b` - Another compute node to combine with this one
    ///
    /// # Returns
    ///
    /// A new compute node that produces both outputs as a tuple.
    fn zip<B: Compute>(self, b: B) -> Zip<Self, B>;

    /// Registers a watcher for this compute node that will be notified on output changes.
    ///
    /// # Arguments
    ///
    /// * `watcher` - The watcher to register for change notifications
    ///
    /// # Returns
    ///
    /// A guard that will automatically unregister the watcher when dropped.
    fn watch(&self, watcher: impl Watcher<Self::Output>) -> WatcherGuard;

    /// Creates a memoized version of this compute node that caches its result.
    ///
    /// # Returns
    ///
    /// A new compute node that will cache the output of this node.
    fn computed(self) -> Computed<Self::Output>
    where
        Self: Clone + 'static;

    /// Attaches metadata to this compute node.
    ///
    /// # Arguments
    ///
    /// * `metadata` - The metadata to attach to this compute node
    ///
    /// # Returns
    ///
    /// A new compute node with the attached metadata.
    fn with<T>(self, metadata: T) -> WithMetadata<Self, T>;
}

impl<C: Compute> ComputeExt for C {
    fn unique_map<F, Output>(self, f: F) -> impl Compute<Output = Unique<Output>>
    where
        F: 'static + Fn(Self::Output) -> Output,
        Output: Clone + 'static,
        Self: 'static,
    {
        self.map(move |value| Unique(f(value)))
    }
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
}
