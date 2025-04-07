//! # Constant Values for Reactive Computation
//!
//! This module provides functionality for working with constant reactive values.
//!
//! ## Overview
//!
//! Constants are immutable values that implement the `Compute` trait but never change.
//! They provide a way to incorporate fixed values into a reactive computation graph.
//!
//! The value type must implement the `ComputeResult` trait, which requires:
//! - `'static` (doesn't contain non-static references)
//! - `Clone` (can be copied)
//! - `PartialEq` (can be compared for equality)
//!
//! ## Examples
//!
//! ```
//! use waterui_reactive::{Compute, ComputeExt, constant, binding};
//!
//! // Create a constant
//! let tax_rate = constant(0.08);
//!
//! // Use in a reactive computation
//! let price = binding(100.0);
//! let total = price.clone().zip(tax_rate)
//!     .map(|(price, rate)| price * (1.0 + rate));
//!
//! assert_eq!(total.compute(), 108.0);
//! ```

use crate::{
    Compute,
    compute::ComputeResult,
    watcher::{Watcher, WatcherGuard},
};

/// A reactive constant value that never changes.
///
/// `Constant<T>` is a simple implementation of the `Compute` trait that always
/// returns the same value when computed. It serves as a way to introduce static
/// values into a reactive computation graph.
///
/// # Type Parameters
///
/// * `T`: The value type, which must implement `ComputeResult`.
///
/// # Examples
///
/// ```
/// use waterui_reactive::{Compute, constant};
///
/// let c = constant(42);
/// assert_eq!(c.compute(), 42);
/// ```
#[derive(Debug, Clone)]
pub struct Constant<T: ComputeResult>(T);

impl<T: ComputeResult> From<T> for Constant<T> {
    /// Creates a new `Constant` from a value.
    ///
    /// # Parameters
    ///
    /// * `value`: The value to be wrapped in a `Constant`.
    ///
    /// # Returns
    ///
    /// A new `Constant` instance containing the provided value.
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: ComputeResult> Compute for Constant<T> {
    type Output = T;

    /// Computes the constant value.
    ///
    /// This simply returns a clone of the contained value.
    ///
    /// # Returns
    ///
    /// A clone of the constant value.
    fn compute(&self) -> Self::Output {
        self.0.clone()
    }

    /// Adds a watcher to this constant.
    ///
    /// Since a constant never changes, this function returns a `WatcherGuard`
    /// with an empty cleanup function. The provided watcher will never be notified
    /// of any changes.
    ///
    /// # Parameters
    ///
    /// * `_watcher`: A watcher that would be notified of changes (unused).
    ///
    /// # Returns
    ///
    /// A `WatcherGuard` with an empty cleanup function.
    fn add_watcher(&self, _watcher: Watcher<Self::Output>) -> WatcherGuard {
        WatcherGuard::new(|| {})
    }
}

/// Creates a new constant reactive value.
///
/// This is a convenience function for creating a `Constant<T>` instance.
///
/// # Parameters
///
/// * `value`: The value to be wrapped in a `Constant`.
///
/// # Returns
///
/// A new `Constant` instance containing the provided value.
///
/// # Examples
///
/// ```
/// use waterui_reactive::{Compute, constant};
///
/// let c = constant("Hello, world!");
/// assert_eq!(c.compute(), "Hello, world!");
/// ```
pub fn constant<T: ComputeResult>(value: T) -> Constant<T> {
    Constant::from(value)
}
