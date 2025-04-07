//! # Map Module
//!
//! This module provides transformation and memoization capabilities for reactive values.
//!
//! The `Map` type enables you to transform values from one type to another while preserving
//! the reactive nature of the computation. It automatically caches the result of the transformation
//! for better performance, invalidating the cache only when the source value changes.
//!
//! ## Key Components
//!
//! - `Map<C, F, Output>`: A reactive value that applies transformation `F` to source `C`
//! - `map()`: Helper function for creating `Map` instances
//! - Automatic caching of transformation results
//! - Reactive propagation of changes from source to transformed value
//!
//! ## Usage Example
//!
//! ```rust
//! use waterui_reactive::{binding, Compute};
//! use waterui_reactive::map::map;
//!
//! let number = binding(5);
//! let doubled = map(number, |n| n * 2);
//!
//! assert_eq!(doubled.compute(), 10);
//!
//! // The transformation is automatically cached
//! doubled.compute(); // Uses cached value, doesn't recompute
//! ```

use core::{cell::RefCell, ops::Deref};

use alloc::rc::Rc;

use crate::{
    Compute,
    compute::ComputeResult,
    watcher::{Watcher, WatcherGuard},
};

struct MapInner<C, F, Output> {
    source: C,
    f: F,
    cache: RefCell<Option<Output>>,
    _guard: RefCell<Option<WatcherGuard>>,
}

/// A reactive computation that transforms values from a source computation.
///
/// `Map<C, F, Output>` applies a transformation function `F` to the results
/// of a source computation `C`, producing a value of type `Output`. The result
/// is automatically cached and only recomputed when the source value changes.
pub struct Map<C, F, Output>(Rc<MapInner<C, F, Output>>);

impl<C: Compute + 'static, F: 'static, Output: ComputeResult> Map<C, F, Output> {
    /// Creates a new `Map` that transforms values from `source` using function `f`.
    ///
    /// # Parameters
    ///
    /// * `source`: The source computation whose results will be transformed
    /// * `f`: The transformation function to apply to the source's results
    ///
    /// # Returns
    ///
    /// A new `Map` instance that will transform values from the source.
    pub fn new(source: C, f: F) -> Self {
        let inner = Rc::new(MapInner {
            source,
            cache: RefCell::default(),
            f,
            _guard: RefCell::default(),
        });

        {
            let rc = inner.clone();
            let guard = inner
                .source
                .add_watcher(Watcher::new(move |_value, _metadata| {
                    rc.cache.replace(None);
                }));
            inner._guard.replace(Some(guard));
        }

        Self(inner)
    }
}

/// Helper function to create a new `Map` transformation.
///
/// This is a convenience wrapper around `Map::new()` with improved type inference.
///
/// # Parameters
///
/// * `source`: The source computation whose results will be transformed
/// * `f`: The transformation function to apply to the source's results
///
/// # Returns
///
/// A new `Map` instance that will transform values from the source.
///
/// # Example
///
/// ```rust
/// use waterui_reactive::{binding, Compute};
/// use waterui_reactive::map::map;
///
/// let counter = binding(1);
/// let doubled = map(counter, |n| n * 2);
/// assert_eq!(doubled.compute(), 2);
/// ```
pub fn map<C, F, Output>(source: C, f: F) -> Map<C, F, Output>
where
    C: Compute + 'static,
    Output: ComputeResult,
    F: 'static + Fn(C::Output) -> Output,
{
    Map::new(source, f)
}

impl<C, F, Output> Clone for Map<C, F, Output> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<C, F, Output> Compute for Map<C, F, Output>
where
    C: Compute + 'static,
    Output: ComputeResult,
    F: 'static + Fn(C::Output) -> Output,
{
    type Output = Output;

    /// Computes the transformed value, using the cache when available.
    ///
    /// This method will:
    /// 1. Check if a cached result exists
    /// 2. If cached, return the cached value
    /// 3. If not cached, compute the source value, apply the transformation,
    ///    cache the result, and return it
    fn compute(&self) -> Self::Output {
        let this = &self.0;
        let mut cache = this.cache.borrow_mut();
        if let Some(cache) = cache.deref() {
            cache.clone()
        } else {
            let result = (this.f)(this.source.compute());
            cache.replace(result.clone());
            result
        }
    }

    /// Registers a watcher to be notified when the transformed value changes.
    ///
    /// This sets up a watcher on the source value that will:
    /// 1. Recompute the transformed value when the source changes
    /// 2. Notify the provided watcher with the new transformed value
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        let this = self.clone();
        self.0
            .source
            .add_watcher(Watcher::new(move |_value, metadata| {
                watcher.notify_with_metadata(this.compute(), metadata)
            }))
    }
}
