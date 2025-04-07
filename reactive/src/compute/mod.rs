//! This module provides a framework for reactive computations that can track dependencies
//! and automatically update when their inputs change.
//!
//! The core abstractions include:
//! - `Compute` - A trait for values that can be computed and watched for changes
//! - `ComputeResult` - A trait for types that can be produced by computations
//! - `IntoCompute`/`IntoComputed` - Conversion traits for working with computations
//!
//! This system enables building reactive data flows where computations automatically
//! re-execute when their dependencies change, similar to reactive programming models
//! found in front-end frameworks.

mod computed;
pub use computed::*;

use crate::{
    map::{Map, map},
    watcher::{Watcher, WatcherGuard},
};

/// Represents a result type that can be computed.
///
/// This trait is automatically implemented for types that are:
/// - 'static: Has full ownership of its data, ensuring values can be safely stored and shared throughout the reactive system
/// - Clone: Can be duplicated for sharing across the reactive graph
/// - PartialEq: Enables efficient change detection
///
/// For optimal performance, types used with this trait should be cheap to clone and compare.
/// For example, prefer [Str](waterui_str::Str) over `String` as it offers low-cost copying
/// similar to `Rc<str>` without the allocation overhead of `String`. This significantly
/// improves performance as values flow through the reactive system.
///
/// The reactive engine uses `PartialEq` to detect changes, only triggering updates when
/// values actually differ - keeping your application responsive with minimal overhead.
pub trait ComputeResult: 'static + Clone + PartialEq {}

/// Blanket implementation for any type that meets the requirements.
impl<T: 'static + Clone + PartialEq> ComputeResult for T {}

/// Implementation of `Compute` for any type that implements `ComputeResult`.
///
/// For non-computed values, this simply returns the value itself and provides
/// a no-op watcher since the value doesn't change.
impl<T: ComputeResult> Compute for T {
    type Output = T;

    /// Returns a clone of the value.
    fn compute(&self) -> Self::Output {
        self.clone()
    }

    /// Creates a watcher guard that does nothing, since plain values don't change.
    fn add_watcher(&self, _watcher: Watcher<Self::Output>) -> WatcherGuard {
        WatcherGuard::new(|| {})
    }
}

/// The core trait for computations.
///
/// Types implementing `Compute` represent a computation that can produce a value
/// and notify observers when that value changes.
pub trait Compute: Clone + 'static {
    /// The type of value produced by this computation.
    type Output: ComputeResult;

    /// Execute the computation and return the current value.
    fn compute(&self) -> Self::Output;

    /// Register a watcher to be notified when the computed value changes.
    ///
    /// Returns a guard that, when dropped, will unregister the watcher.
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard;
}

/// A trait for converting a value into a computation.
pub trait IntoCompute<Output: ComputeResult> {
    /// The specific computation type that will be produced.
    type Compute: Compute<Output = Output>;

    /// Convert this value into a computation.
    fn into_compute(self) -> Self::Compute;
}

/// A trait for converting a value directly into a `Computed<Output>`.
///
/// This is a convenience trait that builds on `IntoCompute`.
pub trait IntoComputed<Output: ComputeResult>: IntoCompute<Output> + 'static {
    /// Convert this value into a `Computed<Output>`.
    fn into_computed(self) -> Computed<Output>;
}

/// Blanket implementation of `IntoCompute` for any type that implements `Compute`.
///
/// This allows for automatic conversion between compatible computation types.
impl<C, Output> IntoCompute<Output> for C
where
    C: Compute + 'static,
    C::Output: 'static,
    Output: From<C::Output> + ComputeResult,
{
    type Compute = Map<C, fn(C::Output) -> Output, Output>;

    /// Convert this computation into one that produces the desired output type.
    fn into_compute(self) -> Self::Compute {
        map(self, Into::into)
    }
}

/// Blanket implementation of `IntoComputed` for any type that implements `IntoCompute`.
impl<C, Output> IntoComputed<Output> for C
where
    C: IntoCompute<Output> + 'static,
    C::Compute: Clone,
    Output: ComputeResult,
{
    /// Convert this value into a `Computed<Output>`.
    fn into_computed(self) -> Computed<Output> {
        Computed::new(self.into_compute())
    }
}

/// A wrapper for a computation that attaches additional metadata.
///
/// This can be used to carry extra information alongside a computation.
#[derive(Debug, Clone)]
pub struct WithMetadata<C, T> {
    /// The metadata to be associated with the computation.
    metadata: T,

    /// The underlying computation.
    compute: C,
}

impl<C, T> WithMetadata<C, T> {
    /// Create a new computation with associated metadata.
    pub fn new(metadata: T, compute: C) -> Self {
        Self { metadata, compute }
    }
}

/// Implementation of `Compute` for `WithMetadata`.
///
/// This delegates the computation to the wrapped value but enriches
/// the watcher notifications with the metadata.
impl<C: Compute, T: Clone + 'static> Compute for WithMetadata<C, T> {
    type Output = C::Output;

    /// Execute the underlying computation.
    fn compute(&self) -> Self::Output {
        self.compute.compute()
    }

    /// Register a watcher, enriching notifications with the metadata.
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        let with = self.metadata.clone();
        self.compute
            .add_watcher(Watcher::new(move |value, metadata| {
                watcher.notify_with_metadata(value, metadata.with(with.clone()));
            }))
    }
}
