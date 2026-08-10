//! GTK4 Tabs (Notebook) component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use nami::{Signal, SignalExt};
use waterui_core::id::Id;
use waterui_core::{Environment, Native};
use waterui_navigation::tab::{NativeTabStyle, Tabs};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<Tabs> {
    /// Renders `WaterUI` `Tabs` as a GTK4 `Notebook`.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "widget-model indices are bounded by the collection length"
    )]
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let tabs = self.into_inner();

        let notebook = gtk4::Notebook::new();
        notebook.set_hexpand(true);
        notebook.set_vexpand(true);

        let position = match tabs.style {
            NativeTabStyle::Automatic | NativeTabStyle::TabBar => gtk4::PositionType::Bottom,
            NativeTabStyle::Sidebar => gtk4::PositionType::Left,
        };
        notebook.set_tab_pos(position);

        // Track tab IDs for selection binding
        let mut tab_ids = Vec::new();

        // Add each tab
        for tab in tabs.tabs {
            let id = tab.id;
            tab_ids.push(id);

            // Render the label
            let label_widget = renderer.render_any(tab.label, env);

            // Build and render the content (NavigationView)
            let navigation_view = tab.content.build();
            let content_widget = renderer.render_any(navigation_view.content, env);

            notebook.append_page(&content_widget, Some(&label_widget));
        }

        // Set initial selection
        let initial_selection = tabs.selection.get();
        if let Some(index) = tab_ids.iter().position(|id| *id == initial_selection) {
            #[allow(clippy::cast_possible_wrap)]
            notebook.set_current_page(Some(index as u32));
        }

        // Watch for selection binding changes -> update notebook
        let binding = tabs.selection;
        let guard = binding.computed().watch({
            let notebook = notebook.clone();
            let tab_ids = tab_ids.clone();
            move |ctx: nami::watcher::Context<Id>| {
                let selected_id = ctx.into_value();
                if let Some(index) = tab_ids.iter().position(|id| *id == selected_id) {
                    let notebook = notebook.clone();
                    #[allow(clippy::cast_possible_wrap)]
                    glib::idle_add_local_once(move || {
                        notebook.set_current_page(Some(index as u32));
                    });
                }
            }
        });

        // Watch for notebook page changes -> update selection binding
        notebook.connect_switch_page({
            let tab_ids = tab_ids.clone();
            move |_, _, page_num| {
                if let Some(id) = tab_ids.as_slice().get(page_num as usize)
                    && binding.get() != *id
                {
                    binding.set(*id);
                }
            }
        });

        // Store watcher guard
        store_watcher_guard(&notebook, guard);

        notebook.upcast()
    }
}
