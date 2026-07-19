#![doc = include_str!("../../README.md")]

extern crate self as waterui;

// Keep the shared WaterUI runtime in the final link when dynamic linking is enabled.
#[cfg(feature = "dynamic_linking")]
use waterui_dylib as _;

pub use waterui_internal::__export_preview;
pub use waterui_internal::configure_environment;
pub use waterui_internal::*;
