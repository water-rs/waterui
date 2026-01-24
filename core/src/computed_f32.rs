//! Ergonomic conversion traits for f32 computed signals.
//!
//! This module provides [`IntoComputedF32`] which allows functions to accept
//! both raw numeric literals (`0.5`, `1`, `2.0`) and reactive signals that
//! produce numeric values, converting them uniformly to `Computed<f32>`.

use nami::{Computed, Signal, SignalExt, signal::IntoComputed};

/// A trait for types that can be converted into a `Computed<f32>`.
///
/// This enables ergonomic APIs where users can pass either:
/// - Raw numeric literals: `0.5`, `1.0`, `42`
/// - Reactive signals: `Binding<f32>`, `Computed<i32>`, etc.
///
/// # Example
///
/// ```ignore
/// fn set_opacity(opacity: impl IntoComputedF32) {
///     let computed: Computed<f32> = opacity.into_computed_f32();
///     // use computed...
/// }
///
/// // All of these work:
/// set_opacity(0.5);      // f64 literal
/// set_opacity(1.0_f32);  // f32 literal
/// set_opacity(1);        // i32 literal
/// set_opacity(binding);  // Binding<f32>
/// ```
pub trait IntoComputedF32: 'static {
    /// Converts this value into a `Computed<f32>`.
    fn into_computed_f32(self) -> Computed<f32>;
}

impl<S> IntoComputedF32 for S
where
    S: Signal + 'static,
    S::Output: IntoF32,
{
    fn into_computed_f32(self) -> Computed<f32> {
        self.map(IntoF32::into_f32).into_computed()
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
