//! GTK4 List component implementation.
//!
//! Renders a `WaterUI` List as a GTK4 `ListView` inside a `ScrolledWindow`
//! for efficient handling of large lists.

use std::rc::Rc;

use gtk4::Widget;
use gtk4::prelude::*;
use nami::{Signal, SignalExt};
use waterui::component::list::ListConfig;
use waterui_core::views::Views;
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::components::layout::keyed_model::{KeyedModel, list_item_id};
use crate::renderer::GtkRenderer;
use crate::util::{store_watcher_guard, store_watcher_guards};

impl GtkComponent for Native<ListConfig> {
    /// Renders a `WaterUI` `List` as a GTK4 scrollable list.
    ///
    /// Uses GTK `ListView` recycling so rows are created lazily for visible items.
    /// Each `ListItem` is rendered as a row with optional delete functionality.
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();
        let contents = config.contents;
        let on_delete = config.on_delete.map(Rc::new);
        let editing = config.editing;
        let scroll_controller = config.scroll_controller;
        let env = env.clone();

        // Create the scrolled container
        let scrolled_window = gtk4::ScrolledWindow::new();
        scrolled_window.set_hexpand(true);
        scrolled_window.set_vexpand(true);
        scrolled_window.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

        let model = Rc::new(KeyedModel::new());
        let initial_ids = (0..contents.len().get())
            .map(|index| {
                let id = contents
                    .get_id(index)
                    .expect("List contents must provide an ID for every row");
                i32::from(*id)
            })
            .collect::<Vec<_>>();
        model.reconcile(&initial_ids);

        let factory = gtk4::SignalListItemFactory::new();
        {
            let contents = contents.clone();
            let editing = editing;
            let on_delete = on_delete.clone();
            let env = env;
            factory.connect_bind(move |_, item| {
                let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() else {
                    return;
                };
                let id = list_item_id(list_item);
                let index = usize::try_from(list_item.position())
                    .expect("GTK List position must fit in usize");
                let current_id = contents
                    .get_id(index)
                    .expect("GTK List position must exist in WaterUI contents");
                assert_eq!(
                    i32::from(*current_id),
                    id,
                    "GTK List model position must match WaterUI contents"
                );
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
                    let contents = contents.clone();
                    let list_item = list_item.downgrade();
                    delete_btn.connect_clicked(move |_| {
                        let list_item = list_item
                            .upgrade()
                            .expect("GTK deleted List row must remain bound while visible");
                        let current_index = usize::try_from(list_item.position())
                            .expect("GTK List position must fit in usize");
                        let current_id = contents
                            .get_id(current_index)
                            .expect("GTK deleted List row must still exist");
                        assert_eq!(
                            i32::from(*current_id),
                            id,
                            "GTK deleted List row identity changed before its action"
                        );
                        on_delete(&env_clone, current_index);
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

        let selection = gtk4::NoSelection::new(Some(model.store()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.set_hexpand(true);
        list_view.set_vexpand(true);
        list_view.add_css_class("boxed-list");

        // Reconcile the GTK model by stable WaterUI row identity.
        let contents_guard = contents.watch(.., {
            let model = Rc::clone(&model);
            move |context| {
                let ids = context
                    .value()
                    .iter()
                    .map(|id| i32::from(**id))
                    .collect::<Vec<_>>();
                let model = Rc::clone(&model);
                glib::idle_add_local_once(move || {
                    model.reconcile(&ids);
                });
            }
        });
        let mut guards = vec![contents_guard];
        if let Some(controller) = scroll_controller {
            let target = controller.target();
            let contents = contents.clone();
            let list_view_for_scroll = list_view.clone();
            guards.push(controller.generation().watch(move |_| {
                let index = target.get();
                let len = contents.len().get();
                assert!(
                    index < len,
                    "List scroll target {index} exceeds collection length {len}"
                );
                let index =
                    u32::try_from(index).expect("List scroll target exceeds the GTK index range");
                let list_view = list_view_for_scroll.clone();
                glib::idle_add_local_once(move || {
                    list_view.scroll_to(index, gtk4::ListScrollFlags::NONE, None);
                });
            }));
        }
        store_watcher_guards(&list_view, guards);

        scrolled_window.set_child(Some(&list_view));
        scrolled_window.upcast()
    }
}
