//! A gradient of drifting, noise-driven colour bands, animated on the GPU.

use waterui_core::layout::StretchAxis;
use waterui_core::{Environment, View};

use crate::shader_paint::ShaderPaintView;

/// A GPU-animated, smooth flowing gradient: noise-driven colour bands that
/// drift with the engine's frame clock, with no per-frame CPU work.
///
/// # Layout Behavior
///
/// Stretches on both axes; constrain it with `.frame()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct FlowingGradient;

impl FlowingGradient {
    /// Creates a flowing gradient.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl View for FlowingGradient {
    fn body(self, _env: &Environment) -> impl View {
        ShaderPaintView::new(include_str!("flowing.wgsl")).animated(true)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}
