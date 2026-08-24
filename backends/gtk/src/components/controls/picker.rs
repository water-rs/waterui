//! GTK4 Picker (DropDown/ComboBox) component implementation.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Widget;
use gtk4::prelude::*;
use nami::{Signal, SignalExt};
use waterui_core::id::Id;
use waterui_core::{Environment, Native};
use waterui_form::picker::{PickerConfig, PickerItem};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guards;

impl GtkComponent for Native<PickerConfig> {
    /// Renders a `WaterUI` `Picker` as a GTK4 `DropDown`.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "widget-model indices are bounded by the collection length"
    )]
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();
        let items = config.items;
        let selection = config.selection;

        // Create dropdown (model is installed reactively below).
        let dropdown = gtk4::DropDown::new(None::<gtk4::StringList>, gtk4::Expression::NONE);

        // The picker's label names the control. Without it a screen reader
        // announces only the selected option, which says what was chosen but
        // never what it was choosing — and the label is mandatory at
        // construction precisely so that cannot happen.
        {
            let label = config
                .label
                .clone()
                .resolve(env)
                .accessibility_label()
                .get()
                .to_plain();
            dropdown.update_property(&[gtk4::accessible::Property::Label(label.as_str())]);
        }

        // Shared IDs for two-way selection syncing.
        let ids = Rc::new(RefCell::new(Vec::new()));

        let refresh_items: Rc<dyn Fn(Vec<PickerItem<Id>>)> = {
            let dropdown = dropdown.clone();
            let ids = ids.clone();
            let selection = selection.clone();
            let env = env.clone();
            Rc::new(move |item_list| {
                let labels: Vec<String> = item_list
                    .iter()
                    .map(|item| {
                        item.content
                            .resolve(&env)
                            .content
                            .get()
                            .to_plain()
                            .to_string()
                    })
                    .collect();
                let new_ids: Vec<_> = item_list.iter().map(|item| item.tag).collect();
                let current_id = selection.get();

                *ids.borrow_mut() = new_ids;

                let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
                let string_list = gtk4::StringList::new(&label_refs);
                dropdown.set_model(Some(&string_list));

                let ids_ref = ids.borrow();
                if ids_ref.is_empty() {
                    dropdown.set_selected(gtk4::INVALID_LIST_POSITION);
                } else if let Some(index) = ids_ref.iter().position(|id| *id == current_id) {
                    dropdown.set_selected(index as u32);
                } else {
                    dropdown.set_selected(0);
                }
            })
        };

        refresh_items(items.get());

        let ids_for_handler = ids.clone();
        let selection_for_handler = selection.clone();

        // Watch dropdown selection changes -> update binding
        dropdown.connect_selected_notify(move |dropdown| {
            let selected_idx = dropdown.selected() as usize;
            let ids_ref = ids_for_handler.borrow();
            if let Some(selected_id) = ids_ref.as_slice().get(selected_idx).copied()
                && selection_for_handler.get() != selected_id
            {
                selection_for_handler.set(selected_id);
            }
        });

        // Watch binding changes -> update dropdown
        let selection_guard = selection.computed().watch({
            let dropdown = dropdown.clone();
            move |ctx| {
                let value = ctx.into_value();
                let dropdown = dropdown.clone();
                let ids = ids.clone();
                glib::idle_add_local_once(move || {
                    if let Some(idx) = ids.borrow().iter().position(|id| *id == value)
                        && dropdown.selected() != idx as u32
                    {
                        dropdown.set_selected(idx as u32);
                    }
                });
            }
        });

        // Watch items changes -> update dropdown model and selection
        let items_guard = items.watch({
            let refresh_items = refresh_items.clone();
            move |ctx| {
                let item_list = ctx.into_value();
                let refresh_items = refresh_items.clone();
                glib::idle_add_local_once(move || {
                    refresh_items(item_list);
                });
            }
        });

        store_watcher_guards(&dropdown, vec![selection_guard, items_guard]);

        dropdown.upcast()
    }
}
