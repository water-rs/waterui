//! Backend traits implemented by concrete Hydrolysis surfaces (tiny-skia, Vello, …).

use std::fmt::Debug;

use waterui_core::Environment;

use crate::tree::RenderTree;

#[cfg(feature = "cpu")]
pub mod cpu;
#[cfg(feature = "gpu")]
pub mod gpu;

#[cfg(feature = "cpu")]
pub use cpu::TinySkiaBackend;
#[cfg(feature = "gpu")]
pub use gpu::VelloWgpuBackend;

/// Result of rendering a single frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameResult {
    /// Frame presented successfully.
    Presented,
    /// Rendering skipped because there were no dirty nodes.
    Idle,
}

/// Trait implemented by every Hydrolysis backend surface.
pub trait RenderBackend: Debug {
    /// Renders the provided tree into the backend surface using the supplied environment.
    fn render(&mut self, tree: &mut RenderTree, env: &Environment) -> FrameResult;
}
