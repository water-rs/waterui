//! GTK dynamic view component implementation.

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Orientation, Widget};
use nami::watcher::Context;
use waterui_core::dynamic::Dynamic;
use waterui_core::{AnyView, Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<Dynamic> {
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let dynamic = self.into_inner();

        // Create a container that will hold the dynamic content
        let container = GtkBox::new(Orientation::Vertical, 0);

        // Dynamic updates can happen later; render updates using a fresh renderer to avoid
        // keeping a raw pointer to the original `GtkRenderer` (which is short-lived).
        let env = env.clone();
        let container_clone = container.clone();

        dynamic.connect(move |ctx: Context<AnyView>| {
            let view = ctx.into_value();
            let env = env.clone();
            let container = container_clone.clone();
            glib::idle_add_local_once(move || {
                // Clear existing children
                while let Some(child) = container.first_child() {
                    container.remove(&child);
                }

                // Render the new view
                let mut renderer = GtkRenderer::new();
                let widget = renderer.render_any(view, &env);
                container.append(&widget);
            });
        });

        container.upcast()
    }
}
