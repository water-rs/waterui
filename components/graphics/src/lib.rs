#![doc = "Graphics primitives for `WaterUI`."]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

/// SVG component for native vector graphics rendering.
pub mod svg;

/// High-performance GPU rendering surface using wgpu (advanced API).
#[cfg(feature = "wgpu")]
pub mod gpu_surface;

/// Simplified shader-based GPU surface (intermediate API).
#[cfg(feature = "wgpu")]
pub mod shader_surface;

/// GPU-accelerated gradient rendering.
#[cfg(feature = "wgpu")]
pub mod gradient_renderer;

/// SVG renderer using resvg and GpuSurface.
#[cfg(feature = "svg-render")]
pub mod svg_renderer;

// Canvas for 2D vector graphics using Vello (beginner-friendly API).
// #[cfg(feature = "canvas")]
//pub mod canvas;
//pub use canvas::{Canvas, DrawingContext};
// Canvas is not available on main branch yet

// Re-export Svg for user convenience.
pub use svg::Svg;

// Re-export key types for user convenience.
#[cfg(feature = "wgpu")]
pub use gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

#[cfg(feature = "wgpu")]
pub use shader_surface::ShaderSurface;

#[cfg(feature = "wgpu")]
pub use gradient_renderer::{GradientConfig, GradientRenderer, GradientType, GradientView};

/// SVG renderer using resvg.
#[cfg(feature = "svg-render")]
pub use svg_renderer::SvgRenderer;

// Re-export wgpu and bytemuck for users to access GPU types directly.
#[cfg(feature = "wgpu")]
pub use wgpu;

/// Re-export bytemuck for safe byte conversions in GPU programming.
#[cfg(feature = "wgpu")]
pub use bytemuck;
