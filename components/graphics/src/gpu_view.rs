//! GPU view abstraction that resolves into a concrete [`GpuRenderer`].
//!
//! `GpuView` is useful when a renderer needs environment-dependent construction.
//! For simple cases, every type that already implements [`GpuRenderer`] also
//! implements `GpuView` via a blanket impl.

use crate::gpu_surface::GpuRenderer;

/// A value that can build a GPU renderer from an environment.
pub trait GpuView: 'static {
    /// Resolves this view into a concrete renderer.
    fn gpu_body(self, env: &mut waterui_core::Environment) -> impl GpuRenderer;
}

impl<T: GpuRenderer> GpuView for T {
    fn gpu_body(self, _env: &mut waterui_core::Environment) -> impl GpuRenderer {
        self
    }
}
