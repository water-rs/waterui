//! GTK4 Progress (`ProgressBar`) component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use nami::Signal;
use waterui::component::progress::ProgressConfig;
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<ProgressConfig> {
    /// Renders a `WaterUI` Progress component as a GTK4 `ProgressBar`.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        // Create container for label + progress bar + value label
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 4);

        // Render the label
        let label_widget = renderer.render_any(config.label, env);
        container.append(&label_widget);

        // Create the progress bar
        let progress_bar = gtk4::ProgressBar::new();
        progress_bar.set_hexpand(true);

        // Set initial value (clamped to 0.0-1.0)
        let initial = config.value.get().clamp(0.0, 1.0);
        progress_bar.set_fraction(initial);

        // Watch for computed value changes -> update progress bar
        let guard = config.value.watch({
            let progress_bar = progress_bar.clone();
            move |ctx| {
                let value = ctx.into_value().clamp(0.0, 1.0);
                let progress_bar = progress_bar.clone();
                glib::idle_add_local_once(move || {
                    progress_bar.set_fraction(value);
                });
            }
        });

        container.append(&progress_bar);

        // Render the value label
        let value_label = renderer.render_any(config.value_label, env);
        container.append(&value_label);

        // Store watcher guard
        store_watcher_guard(&container, guard);

        container.upcast()
    }
}
