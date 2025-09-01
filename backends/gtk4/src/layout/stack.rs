use gtk4::{Box as GtkBox, Orientation, Widget, prelude::*};
use waterui::{
    Environment,
    component::layout::stack::{Stack, StackMode},
};

use crate::renderer::render;

/// Render a generic Stack based on its mode - takes ownership
pub fn render_stack(stack: Stack, env: &Environment) -> Widget {
    match stack.mode {
        StackMode::Vertical => {
            let gtk_box = GtkBox::new(Orientation::Vertical, 0);
            for child in stack.contents {
                let child_widget = render(child, env);
                gtk_box.append(&child_widget);
            }
            gtk_box.upcast()
        }
        StackMode::Horizonal => {
            let gtk_box = GtkBox::new(Orientation::Horizontal, 0);
            for child in stack.contents {
                let child_widget = render(child, env);
                gtk_box.append(&child_widget);
            }
            gtk_box.upcast()
        }
        StackMode::Layered => {
            let overlay = gtk4::Overlay::new();
            let mut first = true;
            for child in stack.contents {
                let child_widget = render(child, env);
                if first {
                    overlay.set_child(Some(&child_widget));
                    first = false;
                } else {
                    overlay.add_overlay(&child_widget);
                }
            }
            overlay.upcast()
        }
    }
}
