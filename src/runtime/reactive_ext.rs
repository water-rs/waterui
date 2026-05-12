//! Extension traits for reactive computations.
//!
//! This module provides additional convenience methods for working with reactive values
//! and computations in the `WaterUI` framework.

use nami::{SignalExt, signal::WithMetadata};
use waterui_core::animation::Animation;

/// Extension trait providing animation-related methods for `Signal` types.
pub trait AnimationExt: SignalExt {
    /// Marks this signal for animation with default settings.
    fn animated(&self) -> WithMetadata<Self, Animation>
    where
        Self: Clone,
    {
        self.with(Animation::Default)
    }
}

impl<C: SignalExt> AnimationExt for C {}
