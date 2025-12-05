#![doc = "Graphics primitives for WaterUI."]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

/// High-performance GPU rendering surface using wgpu.
#[cfg(feature = "wgpu")]
pub mod gpu_surface;

// Re-export key types for user convenience.
#[cfg(feature = "wgpu")]
pub use gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};
