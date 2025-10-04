#![doc = "2D and custom WGPU graphics components for `WaterUI`."]

extern crate alloc;

/// The main canvas view component.
pub mod canvas;
/// The 2D drawing context.
pub mod context;
/// Shape and path definitions.
pub mod shape;
/// The primitive view for custom WGPU rendering.
pub mod wgpu_view;

// Re-export key types for user convenience.
pub use canvas::{Canvas, canvas};
pub use context::GraphicsContext;
pub use shape::{DrawStyle, Path, PathBuilder};
pub use wgpu_view::{WgpuDrawCallback, WgpuView};
