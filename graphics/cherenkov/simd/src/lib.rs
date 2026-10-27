//! SIMD dispatch and mathematics shared by rasterization and filters.
#![no_std]

pub use fearless_simd::{Level, dispatch};

pub mod math;
pub mod span;

/// Detect the execution level once when constructing a renderer or executor.
#[must_use]
pub fn detect() -> Level {
    #[cfg(any(feature = "std", target_arch = "wasm32"))]
    {
        Level::new()
    }
    #[cfg(not(any(feature = "std", target_arch = "wasm32")))]
    {
        Level::baseline()
    }
}
