//! GTK4 `SecureField` (Password Entry) component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use nami::{Signal, SignalExt};
use waterui_core::{Environment, Native};
use waterui_form::secure::{Secure, SecureFieldConfig};

use crate::component::GtkComponent;
use crate::renderer::{GtkRenderer, mark_focus_anchor};
use crate::util::store_watcher_guard;

impl GtkComponent for Native<SecureFieldConfig> {
    /// Renders a `WaterUI` `SecureField` as a GTK4 `PasswordEntry`.
    ///
    /// This creates a two-way binding:
    /// - Entry text changes update the `Binding<Secure>`
    /// - `Binding<Secure>` changes update the entry text
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        // Create container for label + entry
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 4);

        // Render the label
        let label_widget = renderer.render(config.label, env);
        container.append(&label_widget);

        // Create the password entry
        let entry = gtk4::PasswordEntry::new();
        entry.set_hexpand(true);
        entry.set_show_peek_icon(true);
        mark_focus_anchor(&entry);

        let binding = config.value;

        // Set initial value
        entry.set_text(binding.get().expose());

        // Watch for binding changes -> update entry
        let guard = binding.computed().watch({
            let entry = entry.clone();
            move |ctx| {
                let value = ctx.into_value();
                let exposed = value.expose().to_string();
                let entry = entry.clone();
                glib::idle_add_local_once(move || {
                    if entry.text().as_str() != exposed {
                        entry.set_text(&exposed);
                    }
                });
            }
        });

        // Watch for entry changes -> update binding
        entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            let current = binding.get();
            if current.expose() != text {
                // Create a new Secure value and set it
                binding.set(Secure::new(text));
            }
        });

        container.append(&entry);

        // Store watcher guard
        store_watcher_guard(&container, guard);

        container.upcast()
    }
}
