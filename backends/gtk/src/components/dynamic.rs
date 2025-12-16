//! GTK dynamic view component implementation.

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Orientation, Widget};
use nami::watcher::Context;
use waterui_core::dynamic::Dynamic;
use waterui_core::{AnyView, Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<Dynamic> {
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let dynamic = self.into_inner();

        // Create a container that will hold the dynamic content
        let container = GtkBox::new(Orientation::Vertical, 0);

        // We need to store the renderer and environment for later use
        // This is tricky because we need to render new content when it arrives
        let env = env.clone();
        let container_clone = container.clone();

        // Store renderer pointer for later rendering
        // SAFETY: This is safe because the renderer outlives the container
        let renderer_ptr = renderer as *mut GtkRenderer;

        dynamic.connect(move |ctx: Context<AnyView>| {
            let view = ctx.into_value();

            // Clear existing children
            while let Some(child) = container_clone.first_child() {
                container_clone.remove(&child);
            }

            // Render the new view
            // SAFETY: The renderer pointer is valid during the app lifetime
            let widget = unsafe {
                if let Some(renderer) = renderer_ptr.as_mut() {
                    renderer.render_any(view, &env)
                } else {
                    // Fallback: create an empty widget if renderer is gone
                    GtkBox::new(Orientation::Horizontal, 0).upcast()
                }
            };

            container_clone.append(&widget);
        });

        container.upcast()
    }
}
