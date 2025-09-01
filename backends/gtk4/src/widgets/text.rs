//! GTK4 text widget implementation.

use gtk4::{Label, Widget, prelude::*};
use waterui::{
    Environment, Signal,
    component::{Text, text::TextConfig},
};

/// Render a WaterUI Text component as a GTK4 Label.
pub fn render_text(text: TextConfig, _env: &Environment) -> Widget {
    // Get the current text content
    let content = text.content.get();
    let widget = Label::new(Some(&content));

    // Apply basic text styling
    widget.set_selectable(true); // Make text selectable
    widget.set_wrap(true); // Enable text wrapping

    let guard = text.content.watch({
        let widget = widget.clone();
        move |ctx| {
            if ctx.value != widget.text().as_str() {
                widget.set_text(&ctx.value);
            }
        }
    });

    widget.connect_destroy(move |_| {
        let _ = &guard;
    });

    // TODO: Set up reactive content updates when GTK4 reactive bindings are implemented
    // This would require connecting to the text.content() signal and updating the label

    // TODO: Apply font styling when font configuration is available
    // This would involve reading text.font() and applying GTK4 CSS or Pango attributes

    widget.upcast()
}

pub fn render_label(label: waterui::Str) -> Widget {
    let widget = Label::new(Some(&label));

    widget.upcast()
}
