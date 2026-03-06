//! GTK4 LazyContainer component with virtual scrolling.
//!
//! Uses GTK4's ListView with SignalListItemFactory for lazy view reconstruction.

use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Orientation, Widget};
use nami::Signal;
use waterui_core::views::{SharedAnyViews, Views};
use waterui_core::{Environment, Native};
use waterui_layout::StretchAxis;
use waterui_layout::container::LazyContainer;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<LazyContainer> {
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let (layout, contents) = self.into_inner().into_inner();
        let contents = SharedAnyViews::from(contents);
        let env = env.clone();

        // Create ListStore with placeholder objects (indices)
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

        // Determine orientation from layout
        let orientation = match layout.stretch_axis() {
            StretchAxis::Vertical => Orientation::Horizontal,
            // Horizontal, Both, and None all default to vertical orientation
            _ => Orientation::Vertical,
        };

        // Create factory for lazy binding
        let factory = gtk4::SignalListItemFactory::new();
        let contents_clone = contents.clone();
        let env_clone = env.clone();

        factory.connect_setup(|_, item| {
            let list_item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let placeholder = gtk4::Box::new(Orientation::Vertical, 0);
            list_item.set_child(Some(&placeholder));
        });

        factory.connect_bind(move |_, item| {
            let list_item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let index = list_item.position() as usize;

            // Reconstruct view lazily
            if let Some(view) = contents_clone.get_view(index) {
                // Render with a fresh renderer to avoid holding a raw pointer.
                let mut renderer = GtkRenderer::new();
                let widget = renderer.render_any(view, &env_clone);
                list_item.set_child(Some(&widget));
            } else {
                list_item.set_child(Option::<&Widget>::None);
            }
        });

        factory.connect_unbind(|_, item| {
            if let Some(list_item) = item.downcast_ref::<gtk4::ListItem>() {
                list_item.set_child(Option::<&Widget>::None);
            }
        });

        // Create ListView (NO ScrolledWindow - parent handles scrolling)
        let selection = gtk4::NoSelection::new(Some(store));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.set_orientation(orientation);
        list_view.set_hexpand(true);
        list_view.set_vexpand(true);

        // Rebuild index model when the underlying list structure changes.
        let contents_guard = contents.watch(.., {
            let reload_model = reload_model.clone();
            move |_| {
                let reload_model = reload_model.clone();
                glib::idle_add_local_once(move || {
                    reload_model();
                });
            }
        });
        store_watcher_guard(&list_view, Box::new(contents_guard));

        list_view.upcast()
    }
}
