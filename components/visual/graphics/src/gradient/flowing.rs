//! A gradient of drifting, noise-driven colour bands, animated as a shader paint.

use nami::Computed;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, Str, View};

use crate::shader_paint::ShaderPaintView;

/// An animated, smooth flowing gradient: noise-driven colour bands that
/// drift with the engine's frame clock.
///
/// # Layout Behavior
///
/// Stretches on both axes; constrain it with `.frame()`.
#[derive(Debug, Clone, Default)]
pub struct FlowingGradient {
    label: Computed<Option<Str>>,
    value: Computed<Option<Str>>,
}

impl FlowingGradient {
    /// Creates a flowing gradient.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The accessible name the gradient offers. An app's own `.a11y_label(…)`
    /// overrides it.
    #[must_use]
    pub fn offered_label(mut self, label: impl IntoComputed<Option<Str>>) -> Self {
        self.label = label.into_computed();
        self
    }

    /// The accessible value the gradient offers. An app's own `.a11y_value(…)`
    /// overrides it.
    #[must_use]
    pub fn offered_value(mut self, value: impl IntoComputed<Option<Str>>) -> Self {
        self.value = value.into_computed();
        self
    }
}

impl View for FlowingGradient {
    fn body(self, _env: &Environment) -> impl View {
        ShaderPaintView::new(include_str!("flowing.wgsl"))
            .expect("built-in flowing gradient WGSL is valid")
            .continuous(true)
            .offered_label(self.label)
            .offered_value(self.value)
    }
}
