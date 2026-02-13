//! GTK4 List component implementation.
//!
//! Renders a WaterUI List as a GTK4 ListView inside a ScrolledWindow
//! for efficient handling of large lists.

use std::rc::Rc;

use gtk4::Widget;
use gtk4::prelude::*;
use nami::{Signal, SignalExt};
use waterui::component::list::ListConfig;
use waterui_core::views::Views;
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::{store_watcher_guard, store_watcher_guards};

impl GtkComponent for Native<ListConfig> {
    /// Renders a `WaterUI` `List` as a GTK4 scrollable list.
    ///
    /// Uses a vertical Box inside a ScrolledWindow for the list layout.
    /// Each ListItem is rendered as a row with optional delete functionality.
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();
        let contents = config.contents;
        let on_delete = config.on_delete.map(Rc::new);
        let editing = config.editing;
        let env = env.clone();

        // Create the scrolled container
        let scrolled_window = gtk4::ScrolledWindow::new();
        scrolled_window.set_hexpand(true);
        scrolled_window.set_vexpand(true);
        scrolled_window.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

        // Create a vertical box for list items
        let list_box = gtk4::ListBox::new();
        list_box.set_selection_mode(gtk4::SelectionMode::None);
        list_box.add_css_class("boxed-list");

        let rebuild_rows: Rc<dyn Fn()> = {
            let list_box = list_box.clone();
            let contents = contents.clone();
            let editing = editing.clone();
            let on_delete = on_delete.clone();
            let env = env.clone();
            Rc::new(move || {
                while let Some(child) = list_box.first_child() {
                    list_box.remove(&child);
                }

                let len = contents.len();
                for index in 0..len {
                    let Some(item) = contents.get_view(index) else {
                        continue;
                    };

                    let row = gtk4::ListBoxRow::new();
                    let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                    row_box.set_margin_top(8);
                    row_box.set_margin_bottom(8);
                    row_box.set_margin_start(12);
                    row_box.set_margin_end(12);

                    let mut row_renderer = GtkRenderer::new();
                    let content_widget = row_renderer.render_any(item.content, &env);
                    content_widget.set_hexpand(true);
                    row_box.append(&content_widget);

                    if let Some(on_delete) = on_delete.as_ref() {
                        let delete_btn = gtk4::Button::from_icon_name("edit-delete-symbolic");
                        delete_btn.add_css_class("destructive-action");
                        delete_btn.add_css_class("flat");

                        let env_clone = env.clone();
                        let on_delete = on_delete.clone();
                        delete_btn.connect_clicked(move |_| {
                            on_delete(&env_clone, index);
                        });

                        let show_delete = editing
                            .clone()
                            .zip(&item.deletable)
                            .map(|(is_editing, deletable)| is_editing && deletable)
                            .computed();

                        delete_btn.set_visible(show_delete.get());
                        let visibility_guard = show_delete.watch({
                            let delete_btn = delete_btn.clone();
                            move |ctx| {
                                let visible = ctx.into_value();
                                let delete_btn = delete_btn.clone();
                                glib::idle_add_local_once(move || {
                                    delete_btn.set_visible(visible);
                                });
                            }
                        });
                        store_watcher_guard(&delete_btn, visibility_guard);

                        row_box.append(&delete_btn);
                    }

                    row.set_child(Some(&row_box));
                    list_box.append(&row);
                }
            })
        };

        // Initial rows.
        rebuild_rows();

        // Rebuild list on structural content updates.
        let contents_guard = contents.watch(.., {
            let rebuild_rows = rebuild_rows.clone();
            move |_| {
                let rebuild_rows = rebuild_rows.clone();
                glib::idle_add_local_once(move || {
                    rebuild_rows();
                });
            }
        });

        store_watcher_guards(&list_box, vec![contents_guard]);

        scrolled_window.set_child(Some(&list_box));
        scrolled_window.upcast()
    }
}
