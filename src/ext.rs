//! Extension traits for reactive computations.
//!
//! This module provides additional convenience methods for working with reactive values
//! and computations in the `WaterUI` framework.

use nami::{Computed, Signal, map::Map, signal::WithMetadata, zip::Zip};
use waterui_core::animation::Animation;

/// Extension trait providing additional methods for `Signal` types.
///
/// This trait adds convenient methods for transforming, combining, and working with
/// reactive computations in a fluent interface style.
pub trait SignalExt: Signal + Sized {
    /// Transforms the output of this computation using the provided function.
    ///
    /// # Arguments
    ///
    /// * `f` - A function that transforms the output value.
    ///
    /// # Returns
    ///
    /// A new computation that applies the transformation.
    fn map<F, Output>(self, f: F) -> Map<Self, F, Output>
    where
        F: 'static + Fn(Self::Output) -> Output,
        Output: 'static,
        Self: 'static,
    {
        Map::new(self, f)
    }

    /// Combines this computation with another computation.
    ///
    /// # Arguments
    ///
    /// * `b` - Another computation to combine with this one.
    ///
    /// # Returns
    ///
    /// A new computation that produces a tuple of both values.
    fn zip<B: Signal>(self, b: B) -> Zip<Self, B> {
        Zip::new(self, b)
    }

    /// Converts this computation into a `Computed` wrapper.
    ///
    /// This allows the computation to be cloned efficiently.
    ///
    /// # Returns
    ///
    /// A new `Computed` wrapper around this computation.
    fn computed(self) -> Computed<Self::Output>
    where
        Self: Clone + 'static,
    {
        Computed::new(self)
    }

    /// Attaches metadata to this computation.
    ///
    /// # Arguments
    ///
    /// * `metadata` - The metadata to attach.
    ///
    /// # Returns
    ///
    /// A new computation with the attached metadata.
    fn with<T>(self, metadata: T) -> WithMetadata<Self, T> {
        WithMetadata::new(metadata, self)
    }

    /// Marks this computation for animation with default settings.
    ///
    /// # Returns
    ///
    /// A new computation that will be animated when the value changes.
    fn animated(self) -> impl Signal<Output = Self::Output> {
        self.with(Animation::Default)
    }
}

impl<C: Signal + Sized> SignalExt for C {}
