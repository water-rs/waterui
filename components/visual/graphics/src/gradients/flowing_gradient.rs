//! GPU-animated flowing gradient using `ShaderSurface`.

use crate::shader_surface::ShaderSurface;
use core::fmt;
use waterui_core::View;

/// A GPU-animated, smooth flowing gradient.
pub struct FlowingGradient {
    inner: ShaderSurface,
}

impl fmt::Debug for FlowingGradient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FlowingGradient").finish_non_exhaustive()
    }
}

impl FlowingGradient {
    /// Creates a new flowing gradient surface.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: ShaderSurface::from_prewarmed_source(
                crate::prewarm::flowing_gradient_shader_surface_source(),
            ),
        }
    }
}

impl Default for FlowingGradient {
    fn default() -> Self {
        Self::new()
    }
}

impl View for FlowingGradient {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.inner
    }
}
