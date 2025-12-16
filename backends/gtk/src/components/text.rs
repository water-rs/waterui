//! GTK4 Text component implementation.

use gtk4::prelude::*;
use gtk4::{Label, Widget};
use nami::Signal;
use waterui_core::{Environment, Native};
use waterui_text::TextConfig;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<TextConfig> {
    /// Renders a `WaterUI` Text component as a GTK4 Label.
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        // Get the initial text content
        let content = config.content.get();
        let label = Label::new(Some(&content.to_plain()));

        // Apply basic styling - let text maintain natural width
        label.set_selectable(true);

        // Set up reactive updates
        let guard = config.content.watch({
            let label = label.clone();
            move |ctx| {
                let text = ctx.into_value().to_plain();
                let label = label.clone();
                // Schedule update on GTK main thread
                glib::idle_add_local_once(move || {
                    label.set_text(&text);
                });
            }
        });

        // Store the watcher guard to keep it alive
        store_watcher_guard(&label, guard);

        label.upcast()
    }
}
