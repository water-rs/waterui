#![doc = "2D graphics primitives for `WaterUI`."]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

/// The main canvas view component.
pub mod canvas;
/// The 2D drawing context.
pub mod context;
/// The renderer bridge primitive that backends integrate with.
pub mod renderer_view;
/// Optional WGPU shader support built on top of the renderer view.
#[cfg(feature = "wgpu")]
pub mod shader;
/// Shape and path definitions.
pub mod shape;

// Re-export key types for user convenience.
pub use canvas::{Canvas, canvas};
pub use context::GraphicsContext;
#[cfg(feature = "wgpu")]
pub use renderer_view::RendererWgpuSurface;
pub use renderer_view::{RendererBufferFormat, RendererCpuSurface, RendererSurface, RendererView};
#[cfg(feature = "wgpu")]
pub use shader::Shader;
pub use shape::{DrawStyle, Path, PathBuilder};
