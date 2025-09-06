//! Dynamic widget rendering for GTK4.

use gtk4::{Box as GtkBox, Orientation, Widget, prelude::*};
use waterui::{Environment, component::Dynamic};

use crate::renderer::render;

/// Render a dynamic view that can be updated reactively
pub fn render_dynamic(dynamic: Dynamic, env: &Environment) -> Widget {
    let container = GtkBox::new(Orientation::Vertical, 0);

    dynamic.connect({
        let container = container.clone();
        let env = env.clone();
        move |ctx| {
            if let Some(child) = container.first_child() {
                container.remove(&child);
            }
            container.append(&render(ctx.value, &env));
        }
    });

    container.upcast()
}
