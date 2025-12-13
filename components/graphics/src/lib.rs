#![doc = "Graphics primitives for `WaterUI`."]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

/// High-performance GPU rendering surface using wgpu (advanced API).
#[cfg(feature = "wgpu")]
pub mod gpu_surface;

/// Simplified shader-based GPU surface (intermediate API).
#[cfg(feature = "wgpu")]
pub mod shader_surface;

/// Canvas for 2D vector graphics using Vello (beginner-friendly API).
#[cfg(feature = "canvas")]
pub mod canvas;

/// Path builder for Canvas (internal conversions module).
#[cfg(feature = "canvas")]
mod conversions;

/// Drawing state management for Canvas.
#[cfg(feature = "canvas")]
pub mod state;

/// Path construction API for Canvas.
#[cfg(feature = "canvas")]
pub mod path;

/// Gradient builders for Canvas.
#[cfg(feature = "canvas")]
pub mod gradient;

/// Image loading and handling for Canvas.
#[cfg(feature = "canvas")]
pub mod image;

// Re-export key types for user convenience.
#[cfg(feature = "wgpu")]
pub use gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

#[cfg(feature = "wgpu")]
pub use shader_surface::ShaderSurface;

#[cfg(feature = "canvas")]
pub use canvas::{Canvas, DrawingContext};

#[cfg(feature = "canvas")]
pub use path::Path;

#[cfg(feature = "canvas")]
pub use state::{LineCap, LineJoin};

#[cfg(feature = "canvas")]
pub use gradient::{ConicGradient, LinearGradient, RadialGradient};

#[cfg(feature = "canvas")]
pub use image::{CanvasImage, ImageError};

// Re-export wgpu and bytemuck for users to access GPU types directly.
#[cfg(feature = "wgpu")]
pub use wgpu;

/// Re-export bytemuck for safe byte conversions in GPU programming.
#[cfg(feature = "wgpu")]
pub use bytemuck;
