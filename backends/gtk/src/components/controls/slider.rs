//! GTK4 Slider (Scale) component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use nami::{Signal, SignalExt};
use waterui_controls::slider::SliderConfig;
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<SliderConfig> {
    /// Renders a `WaterUI` Slider component as a GTK4 Scale.
    ///
    /// This creates a two-way binding:
    /// - Scale value changes update the `Binding<f64>`
    /// - `Binding<f64>` changes update the scale value
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        // Create container for label + scale
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 4);

        // Render the label if present
        let label_widget = renderer.render(config.label, env);
        container.append(&label_widget);

        // Create horizontal box for min label + scale + max label
        let scale_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);

        // Min value label
        let min_label = renderer.render_any(config.min_value_label, env);
        scale_row.append(&min_label);

        // Create the scale
        let range = config.range;
        let scale = gtk4::Scale::with_range(
            gtk4::Orientation::Horizontal,
            *range.start(),
            *range.end(),
            (*range.end() - *range.start()) / 100.0, // Step size
        );
        scale.set_hexpand(true);

        let binding = config.value;

        // Set initial value
        scale.set_value(binding.get());

        // Watch for binding changes -> update scale
        // Clone before .computed() since it consumes self
        let guard = binding.clone().computed().watch({
            let scale = scale.clone();
            move |ctx| {
                let value = ctx.into_value();
                let scale = scale.clone();
                glib::idle_add_local_once(move || {
                    if (scale.value() - value).abs() > f64::EPSILON {
                        scale.set_value(value);
                    }
                });
            }
        });

        // Watch for scale changes -> update binding
        let binding_clone = binding;
        scale.connect_value_changed(move |scale| {
            let value = scale.value();
            if (binding_clone.get() - value).abs() > f64::EPSILON {
                binding_clone.set(value);
            }
        });

        scale_row.append(&scale);

        // Max value label
        let max_label = renderer.render_any(config.max_value_label, env);
        scale_row.append(&max_label);

        container.append(&scale_row);

        // Store watcher guard
        store_watcher_guard(&container, guard);

        container.upcast()
    }
}
