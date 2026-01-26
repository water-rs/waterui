//! Shared utilities for the GTK backend.

use glib::object::ObjectExt;
use nami::watcher::BoxWatcherGuard;

/// Stores a watcher guard on a widget to prevent it from being dropped.
///
/// The guard is stored as widget data with a unique key, ensuring the reactive
/// subscription stays alive as long as the widget exists.
pub fn store_watcher_guard(widget: &impl ObjectExt, guard: BoxWatcherGuard) {
    // `set_data` takes ownership and will drop the value when the widget is destroyed
    // (or when overwritten by another `set_data` call using the same key).
    unsafe { widget.set_data("waterui_watcher_guard", guard) }
}

/// Stores multiple watcher guards on a widget.
///
/// Use this when a component has multiple reactive subscriptions that need
/// to be kept alive with the widget.
pub fn store_watcher_guards(widget: &impl ObjectExt, guards: Vec<BoxWatcherGuard>) {
    unsafe { widget.set_data("waterui_watcher_guards", guards) }
}
