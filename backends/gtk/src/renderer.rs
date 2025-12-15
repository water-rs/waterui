//! View renderer that dispatches `WaterUI` views to GTK widgets.

use gtk4::prelude::*;
use gtk4::Widget;
use waterui_backend_core::ViewDispatcher;
use waterui_core::{AnyView, Environment, Native, View};
use waterui_layout::container::FixedContainer;
use waterui_layout::spacer::Spacer;
use waterui_text::TextConfig;

use crate::components::spacer::render_spacer;
use crate::components::text::render_text;

/// Context passed to component renderers.
#[derive(Debug, Clone)]
pub struct RenderContext {
    /// Reference to the renderer for recursive rendering.
    /// This is a raw pointer because we can't have self-referential borrows.
    renderer_ptr: *mut GtkRenderer,
}

impl Default for RenderContext {
    fn default() -> Self {
        Self {
            renderer_ptr: std::ptr::null_mut(),
        }
    }
}

impl RenderContext {
    /// Creates a context with a renderer reference.
    fn with_renderer(renderer: &mut GtkRenderer) -> Self {
        Self {
            renderer_ptr: renderer as *mut GtkRenderer,
        }
    }

    /// Gets a mutable reference to the renderer.
    ///
    /// # Safety
    ///
    /// The caller must ensure the renderer pointer is valid.
    unsafe fn renderer(&self) -> Option<&mut GtkRenderer> {
        if self.renderer_ptr.is_null() {
            None
        } else {
            Some(&mut *self.renderer_ptr)
        }
    }
}

/// GTK renderer that converts `WaterUI` views to GTK widgets.
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
        let ctx = RenderContext::with_renderer(self);
        self.dispatcher.dispatch(view, env, ctx)
    }

    /// Renders an `AnyView` to a GTK widget.
    pub fn render_any(&mut self, view: AnyView, env: &Environment) -> Widget {
        let ctx = RenderContext::with_renderer(self);
        self.dispatcher.dispatch_any(view, env, ctx)
    }

    fn register_components(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        // Text component
        dispatcher.register::<Native<TextConfig>>(|_state, _ctx, native, env| {
            render_text(native.into_inner(), env)
        });

        // Spacer component
        dispatcher.register::<Native<Spacer>>(|_state, _ctx, native, env| {
            render_spacer(native.into_inner(), env)
        });

        // FixedContainer - layout container
        dispatcher.register::<Native<FixedContainer>>(|_state, ctx, native, env| {
            // SAFETY: We created the context with a valid renderer pointer
            let renderer = unsafe { ctx.renderer() }
                .expect("renderer must be set in context for container rendering");

            crate::components::container::render_fixed_container(native.into_inner(), env, renderer)
        });

        // TODO: Register more components
        // - Button
        // - Toggle
        // - Slider
        // - TextField
        // - Progress
        // - Divider
    }
}

impl Default for GtkRenderer {
    fn default() -> Self {
        Self::new()
    }
}
