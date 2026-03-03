//! Ergonomic conversion traits for f32 computed signals.
//!
//! This module provides [`IntoComputedF32`] which allows functions to accept
//! both raw numeric literals (`0.5`, `1`, `2.0`) and reactive signals that
//! produce numeric values, converting them uniformly to `Computed<f32>`.

use nami::{Signal, SignalExt};

/// Converts constants or reactive signals into an `f32` signal.
pub trait IntoSignalF32 {
    /// Concrete signal type after conversion.
    type Signal: Signal<Output = f32>;
    /// Converts the input into an `f32` signal.
    fn into_signal_f32(self) -> Self::Signal;
}

impl<S> IntoSignalF32 for S
where
    S: Signal + 'static,
    S::Output: IntoF32,
{
    type Signal = nami::map::Map<S, fn(S::Output) -> f32, f32>;
    fn into_signal_f32(self) -> Self::Signal {
        self.map(IntoF32::into_f32)
    }
}

/// A trait for types that can be converted to f32.
///
/// This is implemented for common numeric types to allow
/// seamless conversion in signal pipelines.
pub trait IntoF32: 'static {
    /// Converts this value into an f32.
    fn into_f32(self) -> f32;
}

macro_rules! impl_into_f32 {
    ($($t:ty),*) => {
        $(
            impl IntoF32 for $t {
                #[inline]
                fn into_f32(self) -> f32 {
                    self as f32
                }
            }
        )*
    };
}

impl_into_f32!(f32, f64, i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
