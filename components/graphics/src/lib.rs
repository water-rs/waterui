#![doc = "Graphics primitives for `WaterUI`."]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

/// Color types and conversion utilities.
pub mod color;
pub use color::{Color, Colorspace, ResolvedColor};

/// Shape primitives for GPU-based vector graphics rendering.
pub mod shape;

/// SVG component for native vector graphics rendering.
pub mod svg;

/// High-performance GPU rendering surface using wgpu (advanced API).
pub mod gpu_surface;

/// Simplified shader-based GPU surface (intermediate API).
pub mod shader_surface;

/// GPU-accelerated gradient rendering.
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
pub use gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

pub use shader_surface::ShaderSurface;

pub use gradient_renderer::{Gradient, GradientConfig, GradientRenderer, GradientType, MeshGradient};

// Re-export shape types for user convenience.
pub use shape::{Circle, Ellipse, FilledShape, Line, PathCommand, Rect, Shape};

/// SVG renderer using resvg.
#[cfg(feature = "svg-render")]
pub use svg_renderer::SvgRenderer;

// Re-export wgpu and bytemuck for users to access GPU types directly.
pub use wgpu;

/// Re-export bytemuck for safe byte conversions in GPU programming.
pub use bytemuck;
