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

/// The core trait for reactive system.
///
/// Types implementing `Compute` represent a computation that can produce a value
/// and notify observers when that value changes.
pub trait Compute: Clone + 'static {
    /// The type of value produced by this computation.
    type Output: 'static;

    /// Execute the computation and return the current value.
    fn compute(&self) -> Self::Output;

    /// Register a watcher to be notified when the computed value changes.
    ///
    /// Returns a guard that, when dropped, will unregister the watcher.
    fn add_watcher(&self, watcher: impl Watcher<Self::Output>) -> WatcherGuard;
}

/// A trait for converting a value into a computation.
pub trait IntoCompute<Output> {
    /// The specific computation type that will be produced.
    type Compute: Compute<Output = Output>;

    /// Convert this value into a computation.
    fn into_compute(self) -> Self::Compute;
}

/// A trait for converting a value directly into a `Computed<Output>`.
///
/// This is a convenience trait that builds on `IntoCompute`.
pub trait IntoComputed<Output>: IntoCompute<Output> + 'static {
    /// Convert this value into a `Computed<Output>`.
    fn into_computed(self) -> Computed<Output>;
}

/// Blanket implementation of `IntoCompute` for any type that implements `Compute`.
///
/// This allows for automatic conversion between compatible computation types.
impl<C, Output> IntoCompute<Output> for C
where
    C: Compute,
    C::Output: 'static,
    Output: From<C::Output> + 'static,
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
    C::Compute: Clone + 'static,
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
    pub const fn new(metadata: T, compute: C) -> Self {
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
    fn add_watcher(&self, watcher: impl Watcher<Self::Output>) -> WatcherGuard {
        let with = self.metadata.clone();
        self.compute
            .add_watcher(move |value, metadata: crate::watcher::Metadata| {
                watcher.notify(value, metadata.with(with.clone()));
            })
    }
}
