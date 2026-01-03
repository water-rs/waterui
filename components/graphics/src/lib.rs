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
/// GPU-animated mesh gradient.
pub mod animated_mesh_gradient;
/// GPU-animated flowing gradient.
pub mod flowing_gradient;

/// Shared GPU context for efficient multi-view rendering.
pub mod shared_context;

/// Shader pre-warming functionality.
pub mod prewarm;

/// Shared shader sources.
pub mod shaders;

/// GPU effect rendering for captured view content.
pub mod view_effect;

/// Filter-based view effects using the Filter trait system.
pub mod filter_view;

// Re-export key types for user convenience.
pub use gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface, PointerState};

pub use shader_surface::ShaderSurface;

pub use gradient_renderer::{Gradient, GradientConfig, GradientRenderer, GradientType, MeshGradient};
pub use animated_mesh_gradient::{AnimatedMeshGradient, AnimatedMeshGradientConfig, ANIMATED_MESH_PALETTE_LEN};
pub use flowing_gradient::FlowingGradient;

pub use view_effect::{EffectContext, EffectInput, EffectOutput, EffectRenderer, OutputSize, ViewEffect};

pub use filter_view::{
    AppliedFilter, FilterAdapter, FilterContext, FilterInput, FilterOutput, FilterViewExt,
    FilteredView, GpuFilter,
};

// Re-export dependencies used by macros
pub use inventory;
pub use rayon;

// Re-export wgpu and bytemuck for users to access GPU types directly.
pub use wgpu;

/// Re-export bytemuck for safe byte conversions in GPU programming.
pub use bytemuck;

pub use pollster;
