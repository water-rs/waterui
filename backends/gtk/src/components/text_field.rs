//! GTK4 `TextField` (Entry) component implementation.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Widget;
use gtk4::prelude::*;
use nami::{Signal, SignalExt};
use waterui_controls::menu::ResolvedMenuItem;
use waterui_controls::text_field::ResolvedTextFieldConfig;
use waterui_core::{Environment, Native};
use waterui_text::styled::StyledStr;

use crate::component::GtkComponent;
use crate::components::menu::rebuild_menu_popover;
use crate::renderer::{GtkRenderer, mark_focus_anchor};
use crate::util::store_watcher_guards;

impl GtkComponent for Native<ResolvedTextFieldConfig> {
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
        mark_focus_anchor(&entry);

        let selection_menu = config.selection_menu;
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

        let selection_items = Rc::new(RefCell::new(selection_menu.get()));
        let selection_guard = selection_menu.watch({
            let selection_items = selection_items.clone();
            move |ctx| {
                *selection_items.borrow_mut() = ctx.into_value();
            }
        });
        install_selection_menu(&entry, env.clone(), selection_items);

        // Watch for entry changes -> update binding
        entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            let current = binding.get();
            assert!(
                current.is_plain(),
                "GTK TextField cannot edit non-plain StyledStr yet; rich text editing backend support is pending"
            );
            let current_plain = current.to_plain();
            if current_plain.as_str() != text {
                binding.set(StyledStr::plain(text));
            }
        });

        container.append(&entry);

        // Store watcher guards
        store_watcher_guards(&container, vec![value_guard, prompt_guard, selection_guard]);

        container.upcast()
    }
}

fn install_selection_menu(
    entry: &gtk4::Entry,
    env: Environment,
    selection_items: Rc<RefCell<Vec<ResolvedMenuItem>>>,
) {
    let popover_state: Rc<RefCell<Option<gtk4::Popover>>> = Rc::new(RefCell::new(None));

    let click = gtk4::GestureClick::new();
    click.set_button(3);
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    click.connect_pressed({
        let entry = entry.clone();
        let selection_items = selection_items.clone();
        let env = env.clone();
        let popover_state = popover_state.clone();
        move |gesture, _n_press, x, y| {
            if entry.selection_bounds().is_none() {
                return;
            }

            let items = selection_items.borrow().clone();
            if items.is_empty() {
                return;
            }

            if let Some(popover) = popover_state.borrow_mut().take() {
                popover.popdown();
                popover.unparent();
            }

            let popover = gtk4::Popover::new();
            rebuild_menu_popover(&popover, items, &env);
            popover.set_has_arrow(true);
            popover.set_autohide(true);
            popover.set_parent(&entry);
            let rect = gdk4::Rectangle::new(x as i32, y as i32, 1, 1);
            popover.set_pointing_to(Some(&rect));
            popover.popup();
            *popover_state.borrow_mut() = Some(popover);

            gesture.set_state(gtk4::EventSequenceState::Claimed);
        }
    });
    entry.add_controller(click);
}
