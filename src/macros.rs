//! Utility macros for common implementations across the `WaterUI` framework.
//!
//! This module provides macros that help reduce boilerplate code when implementing
//! common traits and patterns throughout the framework.

/// A macro for implementing `Debug` trait with simple type name formatting.
///
/// This macro generates a `Debug` implementation that outputs the type name
/// instead of the actual field values, which is useful for types that should
/// not expose their internal structure in debug output.
///
/// # Arguments
///
/// * `$ty` - The type for which to implement `Debug`.
///
/// # Example
///
/// ```rust
/// macro_rules! impl_debug {
///    ($ty:ty) => {
///        impl core::fmt::Debug for $ty {
///            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
///                f.write_str(core::any::type_name::<Self>())
///            }
///        }
///    };
/// }
/// struct MyStruct {
///     private_field: String,
/// }
///
/// impl_debug!(MyStruct);
/// ```
macro_rules! impl_debug {
    ($ty:ty) => {
        impl core::fmt::Debug for $ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(core::any::type_name::<Self>())
            }
        }
    };
}
