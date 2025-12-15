//! GTK Spacer component implementation.

use gtk4::prelude::*;
use gtk4::Widget;
use waterui_core::Environment;
use waterui_layout::spacer::Spacer;

/// Renders a `WaterUI` Spacer as an empty GTK widget that expands.
///
/// The spacer is implemented as an empty `Box` widget with expansion
/// properties set based on the spacer's axis configuration.
#[must_use]
pub fn render_spacer(spacer: Spacer, _env: &Environment) -> Widget {
    let widget = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

    // Spacers expand to fill available space
    // The actual stretch behavior is handled by waterui-layout
    widget.set_hexpand(true);
    widget.set_vexpand(true);

    // Store spacer info for layout system
    // The GtkSubView will report StretchAxis::MainAxis for this widget

    widget.upcast()
}
