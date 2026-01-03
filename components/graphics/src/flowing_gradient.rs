//! GPU-animated flowing gradient using ShaderSurface.

use crate::include_fragment_shader;
use crate::shader_surface::ShaderSurface;
use waterui_core::View;

static FLOWING_GRADIENT_SHADER: crate::prewarm::PrewarmedShader =
    include_fragment_shader!("shaders/flowing_gradient.wgsl");

/// A GPU-animated, smooth flowing gradient.
pub struct FlowingGradient {
    inner: ShaderSurface,
}

impl core::fmt::Debug for FlowingGradient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FlowingGradient").finish_non_exhaustive()
    }
}

impl FlowingGradient {
    /// Creates a new flowing gradient surface.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: ShaderSurface::with_label(
                FLOWING_GRADIENT_SHADER.label,
                FLOWING_GRADIENT_SHADER.source,
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
