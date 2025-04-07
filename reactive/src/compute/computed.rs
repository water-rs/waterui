use core::ops::Add;

use alloc::boxed::Box;

use crate::{
    constant,
    map::map,
    watcher::{Watcher, WatcherGuard},
    zip::{FlattenMap, zip},
};

use super::{Compute, ComputeResult};

/// A wrapper around a boxed implementation of the `ComputedImpl` trait.
///
/// This type represents a computation that can be evaluated to produce a result of type `T`.
/// The computation is stored as a boxed trait object, allowing for dynamic dispatch.
pub struct Computed<T: ComputeResult>(Box<dyn ComputedImpl<Output = T>>);

/// Internal trait that defines the interface for computed values.
///
/// This trait is implemented by types that can compute a value, register watchers,
/// and provide a cloned version of themselves.
trait ComputedImpl {
    /// The result type of the computation
    type Output: ComputeResult;

    /// Computes and returns the current value
    fn compute(&self) -> Self::Output;

    /// Registers a watcher that will be notified when the computed value changes
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard;

    /// Creates a clone of this computation wrapped in a `Computed` container
    fn cloned(&self) -> Computed<Self::Output>;
}

/// Implements `ComputedImpl` for any type that implements `Compute`.
///
/// This allows any `Compute` type to be used as a `ComputedImpl`, providing
/// a bridge between the public and internal interfaces.
impl<C: Compute> ComputedImpl for C {
    type Output = C::Output;

    fn compute(&self) -> Self::Output {
        <Self as Compute>::compute(self)
    }

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        <Self as Compute>::add_watcher(self, watcher)
    }

    fn cloned(&self) -> Computed<Self::Output> {
        Computed::new(self.clone())
    }
}

/// Implements `Clone` for `Computed<T>`.
///
/// This delegates to the internal `cloned` method of the boxed implementation.
impl<T: ComputeResult> Clone for Computed<T> {
    fn clone(&self) -> Self {
        self.0.cloned()
    }
}

/// Implements addition between two `Computed<T>` values.
///
/// This creates a new computation that adds the results of the two input computations.
impl<T: Add + ComputeResult> Add for Computed<T>
where
    T::Output: ComputeResult,
{
    type Output = Computed<T::Output>;

    fn add(self, rhs: Self) -> Self::Output {
        Computed::new(zip(self, rhs).flatten_map(|left, right| left + right))
    }
}

/// Implements addition between a `Computed<T>` and a bare value of type `T`.
///
/// This creates a new computation that adds the result of the input computation with the provided value.
impl<T: 'static + Add + ComputeResult> Add<T> for Computed<T>
where
    T::Output: ComputeResult,
{
    type Output = Computed<T::Output>;

    fn add(self, rhs: T) -> Self::Output {
        Computed::new(map(self, move |this| this + rhs.clone()))
    }
}

/// Implements `Default` for `Computed<T>` when `T` implements `Default`.
///
/// This creates a constant computation with the default value of `T`.
impl<T: ComputeResult + Default> Default for Computed<T> {
    fn default() -> Self {
        Self::constant(T::default())
    }
}

/// Implements `Debug` for `Computed<T>`.
///
/// This just outputs the type name rather than any internal details.
impl<T: ComputeResult> core::fmt::Debug for Computed<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(core::any::type_name::<Self>())
    }
}

/// Implements `Compute` for `Computed<T>`.
///
/// This delegates to the internal boxed implementation.
impl<T: ComputeResult> Compute for Computed<T> {
    type Output = T;

    fn compute(&self) -> Self::Output {
        self.0.compute()
    }

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        self.0.add_watcher(watcher)
    }
}

impl<T: ComputeResult> Computed<T> {
    /// Creates a new `Computed<T>` from a value that implements `Compute<Output = T>`.
    ///
    /// The provided value is boxed and stored internally.
    pub fn new<C>(value: C) -> Self
    where
        C: Compute<Output = T> + Clone + 'static,
    {
        Self(Box::new(value))
    }
}

impl<T: ComputeResult> Computed<T> {
    /// Creates a new constant computation with the provided value.
    ///
    /// This is a convenience wrapper around `Computed::new(constant(value))`.
    pub fn constant(value: T) -> Self {
        Self::new(constant(value))
    }
}
