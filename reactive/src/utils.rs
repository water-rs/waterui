//! # Addition Operations for Compute Types
//!
//! This module provides functionality for adding two `Compute` values together.
//! It leverages the `zip` and `map` operations to combine computations and apply
//! the addition operation to their results.
//!
//! The addition is performed using the standard `Add` trait from Rust's core library,
//! allowing for flexible addition semantics depending on the types involved.

use core::ops::Add;

use crate::{
    Compute,
    map::{Map, map},
    zip::{Zip, zip},
};

/// Adds two `Compute` values together.
///
/// This function takes two values implementing the `Compute` trait and returns a new
/// computation that, when executed, will produce the sum of the outputs of the two
/// input computations.
///
/// # Type Parameters
///
/// * `A`: The first computation type that implements `Compute`.
/// * `B`: The second computation type that implements `Compute`.
///
/// # Constraints
///
/// * `A::Output`: Must implement `Add<B::Output>` to allow addition between the outputs.
/// * `<A::Output as Add<B::Output>>::Output`: The result type must implement `ComputeResult`.
///
/// # Returns
///
/// A new computation that will yield the sum of the outputs from computations `a` and `b`.
///
/// # Examples
///
/// ```
/// use waterui_reactive::{Compute, utils::add};
///
/// // Assuming implementations exist
/// let computation_a = /* some computation that produces a number */;
/// let computation_b = /* some computation that produces a number */;
///
/// let sum_computation = add(computation_a, computation_b);
/// // When executed, sum_computation will produce the sum of the results
/// ```
#[allow(clippy::type_complexity)]
pub fn add<A, B>(
    a: A,
    b: B,
) -> Map<
    Zip<A, B>,
    fn((A::Output, B::Output)) -> <A::Output as Add<B::Output>>::Output,
    <A::Output as Add<B::Output>>::Output,
>
where
    A: Compute + 'static,
    B: Compute + 'static,
    A::Output: Add<B::Output>,
{
    let zip = zip(a, b);
    map(zip, |(a, b)| a.add(b))
}
