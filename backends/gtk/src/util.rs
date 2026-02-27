//! Shared utilities for the GTK backend.

use glib::object::ObjectExt;
use nami::watcher::BoxWatcherGuard;
use waterui_graphics::color::ResolvedColor;

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

/// Converts a resolved color to clamped sRGBA byte channels.
#[must_use]
pub fn resolved_color_to_rgba8(color: ResolvedColor) -> (u8, u8, u8, f32) {
    let srgb = color.to_srgb_with_headroom();
    let red = (srgb.red.clamp(0.0, 1.0) * 255.0) as u8;
    let green = (srgb.green.clamp(0.0, 1.0) * 255.0) as u8;
    let blue = (srgb.blue.clamp(0.0, 1.0) * 255.0) as u8;
    let alpha = color.opacity.clamp(0.0, 1.0);
    (red, green, blue, alpha)
}

/// Converts a resolved color to `#RRGGBB` format.
#[must_use]
pub fn resolved_color_to_hex(color: ResolvedColor) -> String {
    let (red, green, blue, _) = resolved_color_to_rgba8(color);
    format!("#{red:02X}{green:02X}{blue:02X}")
}

/// Converts a resolved color to CSS `rgba(r, g, b, a)` format.
#[must_use]
pub fn resolved_color_to_css_rgba(color: ResolvedColor) -> String {
    let (red, green, blue, alpha) = resolved_color_to_rgba8(color);
    format!("rgba({red}, {green}, {blue}, {alpha})")
}
