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
    /// Uses GTK ListView recycling so rows are created lazily for visible items.
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

        // Backing model stores row indices. GTK ListView will only bind visible rows.
        let store = gtk4::gio::ListStore::new::<glib::BoxedAnyObject>();
        let reload_model: Rc<dyn Fn()> = {
            let store = store.clone();
            let contents = contents.clone();
            Rc::new(move || {
                store.remove_all();
                let len = contents.len().get();
                for index in 0..len {
                    store.append(&glib::BoxedAnyObject::new(index));
                }
            })
        };
        reload_model();

        let factory = gtk4::SignalListItemFactory::new();
        {
            let contents = contents.clone();
            let editing = editing.clone();
            let on_delete = on_delete.clone();
            let env = env.clone();
            factory.connect_bind(move |_, item| {
                let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() else {
                    return;
                };
                let index = list_item.position() as usize;
                let Some(item) = contents.get_view(index) else {
                    list_item.set_child(Option::<&Widget>::None);
                    return;
                };

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
                    store_watcher_guard(&row_box, Box::new(visibility_guard));

                    row_box.append(&delete_btn);
                }

                list_item.set_child(Some(&row_box));
            });
        }
        factory.connect_unbind(|_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Option::<&Widget>::None);
            }
        });

        let selection = gtk4::NoSelection::new(Some(store));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.set_hexpand(true);
        list_view.set_vexpand(true);
        list_view.add_css_class("boxed-list");

        // Rebuild index model on structural updates.
        let contents_guard = contents.watch(.., {
            let reload_model = reload_model.clone();
            move |_| {
                let reload_model = reload_model.clone();
                glib::idle_add_local_once(move || {
                    reload_model();
                });
            }
        });
        store_watcher_guards(&list_view, vec![contents_guard]);

        scrolled_window.set_child(Some(&list_view));
        scrolled_window.upcast()
    }
}
