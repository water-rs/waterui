//! GTK4 Picker (DropDown/ComboBox) component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use nami::{Signal, SignalExt};
use waterui_core::{Environment, Native};
use waterui_form::picker::PickerConfig;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<PickerConfig> {
    /// Renders a `WaterUI` `Picker` as a GTK4 DropDown.
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();
        let items = config.items;
        let selection = config.selection;

        // Collect items and create string list for dropdown
        let item_list = items.get();
        let labels: Vec<String> = item_list
            .iter()
            .map(|item| item.content.content().get().to_plain().to_string())
            .collect();
        let ids: Vec<_> = item_list.iter().map(|item| item.tag).collect();

        // Create string list model
        let string_list =
            gtk4::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());

        // Create dropdown
        let dropdown = gtk4::DropDown::new(Some(string_list), gtk4::Expression::NONE);

        // Find initial selected index
        let current_id = selection.get();
        let initial_index = ids.iter().position(|id| *id == current_id).unwrap_or(0);
        dropdown.set_selected(initial_index as u32);

        // Store IDs for selection handling
        let ids_for_handler = ids.clone();
        let selection_for_handler = selection.clone();

        // Watch dropdown selection changes -> update binding
        dropdown.connect_selected_notify(move |dropdown| {
            let selected_idx = dropdown.selected() as usize;
            if selected_idx < ids_for_handler.len() {
                let selected_id = ids_for_handler[selected_idx];
                if selection_for_handler.get() != selected_id {
                    selection_for_handler.set(selected_id);
                }
            }
        });

        // Watch binding changes -> update dropdown
        let guard = selection.computed().watch({
            let dropdown = dropdown.clone();
            let ids = ids.clone();
            move |ctx| {
                let value = ctx.into_value();
                let dropdown = dropdown.clone();
                let ids = ids.clone();
                glib::idle_add_local_once(move || {
                    if let Some(idx) = ids.iter().position(|id| *id == value) {
                        if dropdown.selected() != idx as u32 {
                            dropdown.set_selected(idx as u32);
                        }
                    }
                });
            }
        });

        store_watcher_guard(&dropdown, guard);

        dropdown.upcast()
    }
}
