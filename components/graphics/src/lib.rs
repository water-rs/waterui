#![doc = "Graphics primitives for `WaterUI`."]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

/// Color types and conversion utilities.
pub mod color;
pub use color::{Color, Colorspace, ResolvedColor};

/// High-performance GPU rendering surface using wgpu (advanced API).
pub mod gpu_surface;

/// Simplified shader-based GPU surface (intermediate API).
pub mod shader_surface;

/// GPU-accelerated gradient rendering.
pub mod gradient_renderer;

/// Shared GPU context for efficient multi-view rendering.
pub mod shared_context;

/// Shader pre-warming functionality.
pub mod prewarm;

// Re-export key types for user convenience.
pub use gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

pub use shader_surface::ShaderSurface;

pub use gradient_renderer::{Gradient, GradientConfig, GradientRenderer, GradientType, MeshGradient};

// Re-export dependencies used by macros
pub use inventory;
pub use rayon;

// Re-export wgpu and bytemuck for users to access GPU types directly.
pub use wgpu;

/// Re-export bytemuck for safe byte conversions in GPU programming.
pub use bytemuck;

pub use pollster;
