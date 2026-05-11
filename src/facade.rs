#![doc = include_str!("../README.md")]

extern crate self as waterui;

// Keep the shared WaterUI runtime in the final link when dynamic linking is enabled.
#[cfg(all(feature = "dynamic_linking", not(windows)))]
use waterui_dylib as _;

pub use waterui_internal::__export_preview;
pub use waterui_internal::configure_environment;
pub use waterui_internal::*;
