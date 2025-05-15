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

use core::{cell::RefCell, marker::PhantomData};

use alloc::rc::Rc;
use waterui_task::LocalTask;

use crate::{
    Compute,
    compute::ComputeResult,
    watcher::{Watcher, WatcherGuard},
};

/// A reactive computation that transforms values from a source computation.
///
/// `Map<C, F, Output>` applies a transformation function `F` to the results
/// of a source computation `C`, producing a value of type `Output`. The result
/// is automatically cached and only recomputed when the source value changes.
pub struct Map<C, F, Output> {
    source: C,
    cache: Rc<RefCell<Option<Output>>>,

    inner: Rc<MapInner<C, F>>,
}

struct MapInner<C, F> {
    f: F,
    #[allow(unused)]
    guard: WatcherGuard,
    _marker: PhantomData<C>,
}

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
        let cache: Rc<RefCell<Option<Output>>> = Rc::default();

        let guard = {
            let cache = cache.clone();
            source.watch(move |_, _| {
                cache.replace(None);
            })
        };

        let inner = Rc::new(MapInner {
            f,
            guard,
            _marker: PhantomData,
        });

        Self {
            source,
            cache,
            inner,
        }
    }

    fn compute_from_source(&self, source: &C) -> Output
    where
        C: Compute,
        Output: ComputeResult,
        F: 'static + Fn(C::Output) -> Output,
    {
        let mut cache = self.cache.borrow_mut();

        let cache = cache.get_or_insert_with(|| (self.inner.f)(source.compute()));

        cache.clone()
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
    C: Compute,
    Output: ComputeResult,
    F: 'static + Fn(C::Output) -> Output,
{
    Map::new(source, f)
}

impl<C: Clone, F, Output> Clone for Map<C, F, Output> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            cache: self.cache.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl<C, F, Output> Compute for Map<C, F, Output>
where
    C: Compute,
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
        self.compute_from_source(&self.source)
    }

    /// Registers a watcher to be notified when the transformed value changes.
    ///
    /// This sets up a watcher on the source value that will:
    /// 1. Recompute the transformed value when the source changes
    /// 2. Notify the provided watcher with the new transformed value
    fn watch(&self, watcher: impl Watcher<Self::Output>) -> WatcherGuard {
        let this = self.clone();

        self.source
            .watch(move |_value, metadata| watcher.notify(this.compute(), metadata))
    }
}

pub struct AsyncMap<C, F, Output> {
    source: C,
    cache: Rc<RefCell<Option<Output>>>,
    inner: Rc<AsyncMapInner<F>>,
}

struct AsyncMapInner<F> {
    f: F,
    task: RefCell<Option<LocalTask<()>>>,
    #[allow(unused)]
    guard: WatcherGuard,
}

impl<C, F, Output> AsyncMap<C, F, Output>
where
    C: Compute,
    F: 'static + AsyncFn(C::Output) -> Output,
    Output: ComputeResult,
{
    pub fn compute_from_source(&self, source: C) {
        // Cancel previous task
        if let Some(task) = self.inner.task.take() {
            LocalTask::on_main(task.cancel());
        }
        let inner = self.inner.clone();
        let value = self.cache.clone();
        let task = LocalTask::on_main(async move {
            let result = (inner.f)(source.compute()).await;
            value.replace(Some(result));
        });
        self.inner.task.replace(Some(task));
    }
}

impl<C: Compute, F, Output> Clone for AsyncMap<C, F, Output> {
    fn clone(&self) -> Self {
        AsyncMap {
            source: self.source.clone(),
            inner: self.inner.clone(),
            cache: self.cache.clone(),
        }
    }
}

impl<C: Compute, F, Output: 'static> AsyncMap<C, F, Output> {
    pub fn new(source: C, f: F) -> Self {
        let cache: Rc<RefCell<Option<Output>>> = Rc::default();

        let guard = {
            let cache = cache.clone();
            source.watch(move |_, _| {
                cache.replace(None);
            })
        };
        Self {
            source,
            cache: Rc::default(),
            inner: Rc::new(AsyncMapInner {
                f,
                task: RefCell::default(),
                guard,
            }),
        }
    }
}
