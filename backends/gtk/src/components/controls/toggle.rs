//! GTK4 Toggle (Switch) component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use nami::{Signal, SignalExt};
use waterui_controls::toggle::ToggleConfig;
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<ToggleConfig> {
    /// Renders a `WaterUI` Toggle component as a GTK4 Switch.
    ///
    /// This creates a two-way binding:
    /// - Switch state changes update the `Binding<bool>`
    /// - `Binding<bool>` changes update the switch state
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        // Create container for label + switch
        let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        container.set_hexpand(true);

        // Render the label
        let label_widget = renderer.render(config.label, env);
        container.append(&label_widget);

        // Add spacer
        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        container.append(&spacer);

        // Create the switch
        let switch = gtk4::Switch::new();
        let binding = config.toggle;

        // Set initial state
        switch.set_active(binding.get());

        // Watch for binding changes -> update switch
        // Clone before .computed() since it consumes self
        let guard = binding.computed().watch({
            let switch = switch.clone();
            move |ctx| {
                let value = ctx.into_value();
                let switch = switch.clone();
                glib::idle_add_local_once(move || {
                    if switch.is_active() != value {
                        switch.set_active(value);
                    }
                });
            }
        });

        // Watch for switch changes -> update binding
        switch.connect_state_set(move |_, state| {
            if binding.get() != state {
                binding.set(state);
            }
            glib::Propagation::Proceed
        });

        container.append(&switch);

        // Store watcher guard
        store_watcher_guard(&container, guard);

        container.upcast()
    }
}
