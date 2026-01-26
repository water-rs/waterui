//! GTK4 List component implementation.
//!
//! Renders a WaterUI List as a GTK4 ListView inside a ScrolledWindow
//! for efficient handling of large lists.

use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::Widget;
use nami::Signal;
use waterui::component::list::ListConfig;
use waterui_core::views::Views;
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<ListConfig> {
    /// Renders a `WaterUI` `List` as a GTK4 scrollable list.
    ///
    /// Uses a vertical Box inside a ScrolledWindow for the list layout.
    /// Each ListItem is rendered as a row with optional delete functionality.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        // Create the scrolled container
        let scrolled_window = gtk4::ScrolledWindow::new();
        scrolled_window.set_hexpand(true);
        scrolled_window.set_vexpand(true);
        scrolled_window.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

        // Create a vertical box for list items
        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::None);
        list_box.add_css_class("boxed-list");

        // Render each list item using Views trait
        let contents = config.contents;
        let on_delete = config.on_delete.map(Rc::new);
        let editing = config.editing.get();
        let len = contents.len();
        for index in 0..len {
            if let Some(item) = contents.get_view(index) {
                let row = gtk4::ListBoxRow::new();

                // Create row content container
                let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                row_box.set_margin_top(8);
                row_box.set_margin_bottom(8);
                row_box.set_margin_start(12);
                row_box.set_margin_end(12);

                // Render the item content
                let content_widget = renderer.render_any(item.content, env);
                content_widget.set_hexpand(true);
                row_box.append(&content_widget);

                // Add delete button if enabled and item is deletable
                if let Some(on_delete) = on_delete.as_ref()
                    && editing
                    && item.deletable.get()
                {
                    let delete_btn = gtk4::Button::from_icon_name("edit-delete-symbolic");
                    delete_btn.add_css_class("destructive-action");
                    delete_btn.add_css_class("flat");

                    let env_clone = env.clone();
                    let on_delete = on_delete.clone();
                    delete_btn.connect_clicked(move |_| {
                        on_delete(&env_clone, index);
                    });

                    row_box.append(&delete_btn);
                }

                row.set_child(Some(&row_box));
                list_box.append(&row);
            }
        }

        scrolled_window.set_child(Some(&list_box));
        scrolled_window.upcast()
    }
}
