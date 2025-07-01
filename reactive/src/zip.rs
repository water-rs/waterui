//! Provides functionality for combining and transforming computations.
//!
//! This module contains:
//! - `Zip`: A structure to combine two `Compute` instances into one computation
//!   that produces a tuple of their results.
//! - `FlattenMap`: A trait for flattening and mapping nested tuple structures,
//!   which simplifies working with multiple zipped computations.
//!
//! These utilities enable composition of reactive computations, making it easier
//! to work with multiple interdependent values in a reactive context.

use alloc::rc::Rc;

use crate::{
    Compute,
    map::{Map, map},
    watcher::{Watcher, WatcherGuard},
};

/// A structure that combines two `Compute` instances into a single computation
/// that produces a tuple of their results.
#[derive(Debug, Clone)]
pub struct Zip<A, B> {
    /// The first computation to be zipped.
    a: A,
    /// The second computation to be zipped.
    b: B,
}

impl<A, B> Zip<A, B> {
    /// Creates a new `Zip` instance by combining two computations.
    ///
    /// # Parameters
    /// - `a`: The first computation to be zipped.
    /// - `b`: The second computation to be zipped.
    ///
    /// # Returns
    /// A new `Zip` instance containing both computations.
    pub const fn new(a: A, b: B) -> Self {
        Self { a, b }
    }
}

/// A/ This trait provides a way to apply a function to the individual elements
/// of a nested tuple structure, flattening the structure in the process.
pub trait FlattenMap<F, T, Output>: Sized + Compute {
    /// Maps a function over the flattened elements of a nested tuple.
    ///
    /// # Parameters
    /// - `self`: The computation that produces a nested tuple.
    /// - `f`: The function to apply to the flattened elements.
    ///
    /// # Returns
    /// A new computation that produces the result of applying `f` to the flattened elements.
    fn flatten_map(self, f: F) -> Map<Self, impl Fn(Self::Output) -> Output, Output>;
}

/// Implementation for flattening and mapping a tuple of two elements.
impl<C, F, T1, T2, Output> FlattenMap<F, (T1, T2), Output> for C
where
    C: Compute<Output = (T1, T2)> + 'static,
    F: 'static + Fn(T1, T2) -> Output,
    T1: 'static,
    T2: 'static,
    Output: 'static,
{
    fn flatten_map(self, f: F) -> Map<C, impl Fn((T1, T2)) -> Output, Output> {
        map(self, move |(t1, t2)| f(t1, t2))
    }
}

/// Implementation for flattening and mapping a tuple of three elements.
impl<C, F, T1, T2, T3, Output> FlattenMap<F, (T1, T2, T3), Output> for C
where
    C: Compute<Output = ((T1, T2), T3)> + 'static,
    F: 'static + Fn(T1, T2, T3) -> Output,
{
    fn flatten_map(self, f: F) -> Map<C, impl Fn(((T1, T2), T3)) -> Output, Output> {
        map(self, move |((t1, t2), t3)| f(t1, t2, t3))
    }
}

/// Creates a new `Zip` computation that combines two separate computations.
///
/// This function is a convenience wrapper around `Zip::new`.
///
/// # Parameters
/// - `a`: The first computation to zip.
/// - `b`: The second computation to zip.
///
/// # Returns
/// A new `Zip` instance that computes both values and returns them as a tuple.
pub const fn zip<A, B>(a: A, b: B) -> Zip<A, B>
where
    A: Compute,
    B: Compute,
{
    Zip::new(a, b)
}

/// Implementation of the `Compute` trait for `Zip`.
impl<A: Compute, B: Compute> Compute for Zip<A, B> {
    /// The output type of the zipped computation is a tuple of the outputs of the individual computations.
    type Output = (A::Output, B::Output);

    /// Computes both values and returns them as a tuple.
    ///
    /// # Returns
    /// A tuple containing the results of computing `a` and `b`.
    fn compute(&self) -> Self::Output {
        let Self { a, b } = self;
        (a.compute(), b.compute())
    }

    /// Adds a watcher to the zipped computation.
    ///
    /// This method sets up watchers for both `a` and `b` such that when either one
    /// changes, the watcher for the `Zip` is notified with the new tuple.
    ///
    /// # Parameters
    /// - `watcher`: The watcher to notify when either computation changes.
    ///
    /// # Returns
    /// A `WatcherGuard` that, when dropped, will remove the watchers from both computations.
    fn add_watcher(&self, watcher: impl Watcher<Self::Output>) -> WatcherGuard {
        let watcher = Rc::new(watcher);
        let Self { a, b } = self;
        let guard_a = {
            let watcher = watcher.clone();
            let b = b.clone();
            self.a.add_watcher(move |value: A::Output, metadata| {
                let result = (value, b.compute());
                watcher.notify(result, metadata);
            })
        };

        let guard_b = {
            let a = a.clone();
            self.b.add_watcher(move |value: B::Output, metadata| {
                let result = (a.compute(), value);
                watcher.notify(result, metadata);
            })
        };

        WatcherGuard::new(move || {
            let _ = (guard_a, guard_b);
        })
    }
}
