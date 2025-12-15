//! Shared utilities for the GTK backend.

use glib::object::ObjectExt;
use nami::watcher::BoxWatcherGuard;

/// Stores a watcher guard on a widget to prevent it from being dropped.
///
/// The guard is stored as widget data with a unique key, ensuring the reactive
/// subscription stays alive as long as the widget exists.
pub fn store_watcher_guard(widget: &impl ObjectExt, guard: BoxWatcherGuard) {
    let boxed = Box::new(guard);
    let ptr = Box::into_raw(boxed);
    unsafe {
        widget.set_data("waterui_watcher_guard", ptr);
    }
}

/// Stores multiple watcher guards on a widget.
///
/// Use this when a component has multiple reactive subscriptions that need
/// to be kept alive with the widget.
pub fn store_watcher_guards(widget: &impl ObjectExt, guards: Vec<BoxWatcherGuard>) {
    let boxed = Box::new(guards);
    let ptr = Box::into_raw(boxed);
    unsafe {
        widget.set_data("waterui_watcher_guards", ptr);
    }
}
