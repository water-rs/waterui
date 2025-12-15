//! View renderer that dispatches WaterUI views to GTK widgets.

use gtk4::prelude::*;
use gtk4::Widget;
use waterui_backend_core::ViewDispatcher;
use waterui_core::{AnyView, Environment, Native, View};
use waterui_text::TextConfig;

use crate::components::text::render_text;

/// Context passed to component renderers.
#[derive(Debug, Clone, Default)]
pub struct RenderContext {
    // Future: parent widget, depth, etc.
}

/// GTK renderer that converts WaterUI views to GTK widgets.
#[derive(Debug)]
pub struct GtkRenderer {
    dispatcher: ViewDispatcher<(), RenderContext, Widget>,
}

impl GtkRenderer {
    /// Creates a new GTK renderer with all component handlers registered.
    #[must_use]
    pub fn new() -> Self {
        let mut dispatcher = ViewDispatcher::new();

        // Register component handlers
        Self::register_components(&mut dispatcher);

        Self { dispatcher }
    }

    /// Renders a view to a GTK widget.
    pub fn render<V: View>(&mut self, view: V, env: &Environment) -> Widget {
        self.dispatcher.dispatch(view, env, RenderContext::default())
    }

    /// Renders an AnyView to a GTK widget.
    pub fn render_any(&mut self, view: AnyView, env: &Environment) -> Widget {
        self.dispatcher
            .dispatch_any(view, env, RenderContext::default())
    }

    fn register_components(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        // Text component
        dispatcher.register::<Native<TextConfig>>(|_state, _ctx, native, env| {
            render_text(native.into_inner(), env)
        });

        // TODO: Register more components
        // - Button
        // - Toggle
        // - Slider
        // - TextField
        // - Progress
        // - Layout containers
    }
}

impl Default for GtkRenderer {
    fn default() -> Self {
        Self::new()
    }
}
