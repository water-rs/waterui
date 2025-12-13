#![doc = "Graphics primitives for `WaterUI`."]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

/// High-performance GPU rendering surface using wgpu (advanced API).
#[cfg(feature = "wgpu")]
pub mod gpu_surface;

/// Simplified shader-based GPU surface (intermediate API).
#[cfg(feature = "wgpu")]
pub mod shader_surface;

// Canvas for 2D vector graphics using Vello (beginner-friendly API).
// #[cfg(feature = "canvas")]
//pub mod canvas;
//pub use canvas::{Canvas, DrawingContext};
// Canvas is not available on main branch yet

// Re-export key types for user convenience.
#[cfg(feature = "wgpu")]
pub use gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

#[cfg(feature = "wgpu")]
pub use shader_surface::ShaderSurface;

// Re-export wgpu and bytemuck for users to access GPU types directly.
#[cfg(feature = "wgpu")]
pub use wgpu;

/// Re-export bytemuck for safe byte conversions in GPU programming.
#[cfg(feature = "wgpu")]
pub use bytemuck;
