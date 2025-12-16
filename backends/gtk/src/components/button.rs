//! GTK4 Button component implementation.

use std::cell::RefCell;

use gtk4::prelude::*;
use gtk4::Widget;
use waterui_controls::button::ButtonConfig;
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<ButtonConfig> {
    /// Renders a `WaterUI` Button component as a GTK4 Button.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        let button = gtk4::Button::new();

        // Render the label view and set as button child
        let label_widget = renderer.render_any(config.label, env);
        button.set_child(Some(&label_widget));

        // Connect click handler using RefCell for interior mutability
        // since GTK's connect_clicked requires Fn, not FnMut
        let action = RefCell::new(config.action);
        let env_clone = env.clone();
        button.connect_clicked(move |_| {
            action.borrow_mut().handle(&env_clone);
        });

        // TODO: Apply button style based on config.style

        button.upcast()
    }
}
