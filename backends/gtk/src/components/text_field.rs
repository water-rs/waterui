//! GTK4 `TextField` (Entry) component implementation.

use gtk4::prelude::*;
use gtk4::Widget;
use nami::{Signal, SignalExt};
use waterui_controls::text_field::TextFieldConfig;
use waterui_core::{Environment, Native};
use waterui_text::styled::StyledStr;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guards;

impl GtkComponent for Native<TextFieldConfig> {
    /// Renders a `WaterUI` `TextField` component as a GTK4 Entry.
    ///
    /// This creates a two-way binding:
    /// - Entry text changes update the `Binding<StyledStr>` as plain text
    /// - `Binding<StyledStr>` changes update the entry text from plain representation
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        // Create container for label + entry
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 4);

        // Render the label
        let label_widget = renderer.render_any(config.label, env);
        container.append(&label_widget);

        // Create the entry
        let entry = gtk4::Entry::new();
        entry.set_hexpand(true);

        let binding = config.value;
        let prompt = config.prompt.content();

        // Set placeholder text
        let prompt_text = prompt.get().to_plain();
        entry.set_placeholder_text(Some(&prompt_text));

        // Set initial value
        entry.set_text(binding.get().to_plain().as_str());

        // Watch for binding changes -> update entry
        // Clone before .computed() since it consumes self
        let value_guard = binding.clone().computed().watch({
            let entry = entry.clone();
            move |ctx| {
                let value = ctx.into_value().to_plain().to_string();
                let entry = entry.clone();
                glib::idle_add_local_once(move || {
                    if entry.text().as_str() != value {
                        entry.set_text(&value);
                    }
                });
            }
        });

        let prompt_guard = prompt.watch({
            let entry = entry.clone();
            move |ctx| {
                let prompt_text = ctx.into_value().to_plain().to_string();
                let entry = entry.clone();
                glib::idle_add_local_once(move || {
                    entry.set_placeholder_text(Some(&prompt_text));
                });
            }
        });

        // Watch for entry changes -> update binding
        entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            let current_plain = binding.get().to_plain();
            if current_plain.as_str() != text {
                binding.set(StyledStr::plain(text));
            }
        });

        container.append(&entry);

        // Store watcher guards
        store_watcher_guards(&container, vec![value_guard, prompt_guard]);

        container.upcast()
    }
}
