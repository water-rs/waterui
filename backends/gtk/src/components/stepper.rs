//! GTK stepper component implementation.

use gtk4::prelude::*;
use gtk4::{Adjustment, Box as GtkBox, Orientation, SpinButton, Widget};
use nami::{Signal, SignalExt};
use waterui_controls::stepper::StepperConfig;
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guards;

impl GtkComponent for Native<StepperConfig> {
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        // Create the layout box
        let container = GtkBox::new(Orientation::Horizontal, 8);

        // Render the label
        let label_widget = renderer.render_any(config.label, env);
        container.append(&label_widget);

        // Create the spin button (stepper)
        let start = *config.range.start();
        let end = *config.range.end();
        let initial_value = config.value.get();
        let step = config.step.get();

        let adjustment = Adjustment::new(
            f64::from(initial_value),
            f64::from(start),
            f64::from(end),
            f64::from(step),
            f64::from(step * 10), // page increment
            0.0,                  // page size (unused for spin buttons)
        );

        // Add spacer to push spin button to the right
        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        container.append(&spacer);

        let spin_button = SpinButton::new(Some(&adjustment), 1.0, 0);

        // Two-way binding: Binding -> SpinButton
        let binding = config.value;
        let value_guard = binding.clone().computed().watch({
            let spin_button = spin_button.clone();
            move |ctx: nami::watcher::Context<i32>| {
                let value = ctx.into_value();
                let spin_button = spin_button.clone();
                glib::idle_add_local_once(move || {
                    #[allow(clippy::cast_possible_truncation)]
                    let current = spin_button.value() as i32;
                    if value != current {
                        spin_button.set_value(f64::from(value));
                    }
                });
            }
        });

        let step_guard = config.step.watch({
            let adjustment = adjustment.clone();
            move |ctx| {
                let step = ctx.into_value();
                let adjustment = adjustment.clone();
                glib::idle_add_local_once(move || {
                    adjustment.set_step_increment(f64::from(step));
                    adjustment.set_page_increment(f64::from(step.saturating_mul(10)));
                });
            }
        });
        store_watcher_guards(&spin_button, vec![value_guard, step_guard]);

        // Two-way binding: SpinButton -> Binding
        let binding_for_signal = binding.clone();
        spin_button.connect_value_changed(move |btn| {
            #[allow(clippy::cast_possible_truncation)]
            let value = btn.value() as i32;
            if binding_for_signal.get() != value {
                binding_for_signal.set(value);
            }
        });

        container.append(&spin_button);

        container.upcast()
    }
}
