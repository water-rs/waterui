//! GTK4 LazyContainer component with virtual scrolling.
//!
//! Uses GTK4's ListView with SignalListItemFactory for lazy view reconstruction.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Orientation, Widget};
use waterui_core::views::Views;
use waterui_core::{Environment, Native};
use waterui_layout::StretchAxis;
use waterui_layout::container::LazyContainer;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<LazyContainer> {
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let (layout, contents) = self.into_inner().into_inner();
        let contents = Rc::new(contents);
        let env = env.clone();

        // Widget cache for performance (avoid re-rendering on scroll back)
        let widget_cache: Rc<RefCell<HashMap<usize, Widget>>> =
            Rc::new(RefCell::new(HashMap::new()));

        // Create ListStore with placeholder objects (indices)
        let store = gtk4::gio::ListStore::new::<glib::BoxedAnyObject>();
        for i in 0..contents.len() {
            store.append(&glib::BoxedAnyObject::new(i));
        }

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
        let cache_clone = widget_cache.clone();

        factory.connect_setup(|_, item| {
            let list_item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let placeholder = gtk4::Box::new(Orientation::Vertical, 0);
            list_item.set_child(Some(&placeholder));
        });

        factory.connect_bind(move |_, item| {
            let list_item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let index = list_item.position() as usize;

            // Check cache first
            if let Some(widget) = cache_clone.borrow().get(&index) {
                list_item.set_child(Some(widget));
                return;
            }

            // Reconstruct view lazily
            if let Some(view) = contents_clone.get_view(index) {
                // Render with a fresh renderer to avoid holding a raw pointer.
                let mut renderer = GtkRenderer::new();
                let widget = renderer.render_any(view, &env_clone);
                cache_clone.borrow_mut().insert(index, widget.clone());
                list_item.set_child(Some(&widget));
            }
        });

        // Don't destroy widget on unbind - keep in cache for scroll performance
        factory.connect_unbind(|_, _item| {
            // Widget stays in cache; list_item child cleared automatically
        });

        // Create ListView (NO ScrolledWindow - parent handles scrolling)
        let selection = gtk4::NoSelection::new(Some(store));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.set_orientation(orientation);
        list_view.set_hexpand(true);
        list_view.set_vexpand(true);

        list_view.upcast()
    }
}
