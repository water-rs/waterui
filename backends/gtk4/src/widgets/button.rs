//! GTK4 button widget implementation.

use crate::renderer::render;
use gtk4::prelude::*;
use waterui::{Environment, View, component::button::ButtonConfig};

/// Render a WaterUI button component as a GTK4 Button.
pub fn render_button<V: View>(button: ButtonConfig, env: &Environment) -> gtk4::Widget {
    let widget = gtk4::Button::new();

    // Render the button content and set it as the button's child
    let label_widget = render(button.label, env);
    widget.set_child(Some(&label_widget));

    widget.connect_clicked({
        let env = env.clone();
        move |_| {
            button.action.handle(&env);
        }
    });

    widget.upcast()
}
