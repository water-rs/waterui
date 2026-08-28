//! GTK4 `LazyContainer` component with virtual scrolling.
//!
//! Uses GTK4's `ListView` with `SignalListItemFactory` for lazy view reconstruction.

use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Orientation, Widget};
use nami::Signal;
use waterui_core::views::{SharedAnyViews, Views};
use waterui_core::{Environment, Native};
use waterui_layout::container::LazyContainer;
use waterui_layout::stack::{LazyStackAxis, lazy_stack_axis};

use crate::component::GtkComponent;
use crate::components::layout::keyed_model::{KeyedModel, list_item_id};
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<LazyContainer> {
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let (layout, contents) = self.into_inner().into_inner();
        let contents = SharedAnyViews::from(contents);
        let env = env.clone();

        let model = Rc::new(KeyedModel::new());
        let initial_ids = (0..contents.len().get())
            .map(|index| {
                let id = contents
                    .get_id(index)
                    .expect("LazyContainer contents must provide an ID for every child");
                i32::from(*id)
            })
            .collect::<Vec<_>>();
        model.reconcile(&initial_ids);

        // The axis, spacing and cross-axis alignment all come from the layout the
        // container was built with. Deriving the axis from `Layout::stretch_axis`
        // instead — a different question — is what laid every lazy `HStack` out
        // vertically once the stacks became content-sized.
        let axis = lazy_stack_axis(layout.as_ref()).unwrap_or_else(|| {
            panic!("GTK LazyContainer supports the virtualizable stack layouts; got {layout:?}")
        });
        let (orientation, spacing, cross_alignment) = match &axis {
            LazyStackAxis::Vertical { spacing, alignment } => (
                Orientation::Vertical,
                spacing.get(),
                gtk_align_from_horizontal(*alignment),
            ),
            LazyStackAxis::Horizontal { spacing, alignment } => (
                Orientation::Horizontal,
                spacing.get(),
                gtk_align_from_vertical(*alignment),
            ),
        };

        // Create factory for lazy binding
        let factory = gtk4::SignalListItemFactory::new();
        let contents_clone = contents.clone();
        let env_clone = env;

        factory.connect_setup(|_, item| {
            let list_item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let placeholder = gtk4::Box::new(Orientation::Vertical, 0);
            list_item.set_child(Some(&placeholder));
        });

        factory.connect_bind(move |_, item| {
            let list_item = item.downcast_ref::<gtk4::ListItem>().unwrap();
            let id = list_item_id(list_item);
            let index = usize::try_from(list_item.position())
                .expect("GTK LazyContainer position must fit in usize");
            let current_id = contents_clone
                .get_id(index)
                .expect("GTK LazyContainer position must exist in WaterUI contents");
            assert_eq!(
                i32::from(*current_id),
                id,
                "GTK LazyContainer model position must match WaterUI contents"
            );

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
        let selection = gtk4::NoSelection::new(Some(model.store()));
        let list_view = gtk4::ListView::new(Some(selection), Some(factory));
        list_view.set_orientation(orientation);
        // GTK spaces list rows through the widget's CSS box, so the stack's
        // spacing becomes the inter-row gap rather than being dropped.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "GTK spacing is integer pixels while WaterUI layout is f32"
        )]
        list_view.set_property("row-spacing", spacing.max(0.0) as i32);
        match orientation {
            Orientation::Vertical => list_view.set_halign(cross_alignment),
            _ => list_view.set_valign(cross_alignment),
        }
        list_view.set_hexpand(true);
        list_view.set_vexpand(true);

        // Reconcile the GTK model by stable WaterUI child identity.
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
        store_watcher_guard(&list_view, Box::new(contents_guard));

        list_view.upcast()
    }
}

/// Maps a `WaterUI` cross-axis alignment onto GTK's, for a vertical stack.
fn gtk_align_from_horizontal(alignment: waterui_layout::HorizontalAlignment) -> gtk4::Align {
    use waterui_layout::HorizontalAlignment;
    if alignment == HorizontalAlignment::Leading {
        gtk4::Align::Start
    } else if alignment == HorizontalAlignment::Trailing {
        gtk4::Align::End
    } else {
        gtk4::Align::Center
    }
}

/// Maps a `WaterUI` cross-axis alignment onto GTK's, for a horizontal stack.
fn gtk_align_from_vertical(alignment: waterui_layout::VerticalAlignment) -> gtk4::Align {
    use waterui_layout::VerticalAlignment;
    if alignment == VerticalAlignment::Top {
        gtk4::Align::Start
    } else if alignment == VerticalAlignment::Bottom {
        gtk4::Align::End
    } else {
        gtk4::Align::Center
    }
}
