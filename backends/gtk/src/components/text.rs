//! GTK4 Text component implementation.

use glib::object::ObjectExt;
use gtk4::prelude::*;
use gtk4::{Label, Widget};
use nami::watcher::BoxWatcherGuard;
use waterui_core::Environment;
use waterui_text::TextConfig;

/// Renders a WaterUI Text component as a GTK4 Label.
#[must_use]
pub fn render_text(config: TextConfig, _env: &Environment) -> Widget {
    // Get the initial text content
    let content = config.content.get();
    let label = Label::new(Some(&content.to_plain()));

    // Apply basic styling
    label.set_selectable(true);
    label.set_wrap(true);

    // Set up reactive updates
    let guard = config.content.watch({
        let label = label.clone();
        move |ctx| {
            let text = ctx.value.to_plain();
            let label = label.clone();
            // Schedule update on GTK main thread
            glib::idle_add_local_once(move || {
                label.set_text(&text);
            });
        }
    });

    // Store the watcher guard to keep it alive
    // Using unsafe data storage since GTK's set_data requires 'static
    store_watcher_guard(&label, guard);

    label.upcast()
}

/// Stores a watcher guard on a widget to prevent it from being dropped.
fn store_watcher_guard(widget: &impl ObjectExt, guard: BoxWatcherGuard) {
    // Box the guard and leak it - it will live as long as the widget
    // TODO: Properly clean up on widget destroy using connect_destroy
    let boxed = Box::new(guard);
    let ptr = Box::into_raw(boxed);

    // Store the pointer as data on the widget
    // SAFETY: We're storing a raw pointer that we own
    unsafe {
        widget.set_data("waterui_watcher_guard", ptr);
    }
}
