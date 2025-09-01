//! GTK4 general widget implementations.

use gtk4::{Orientation, ProgressBar, Separator, Widget, prelude::*};
use waterui::{
    Environment, Signal,
    component::{Metadata, layout::Edge, progress::ProgressConfig},
};

use crate::renderer::render;

/// Render a Progress component as a GTK4 ProgressBar - takes ownership.
pub fn render_progress(progress: ProgressConfig, _env: &Environment) -> Widget {
    let widget = ProgressBar::new();

    let value = progress.value.get();

    if value.is_nan() {
        widget.pulse();
    } else {
        widget.set_fraction(progress.value.get());
    }

    widget.upcast()
}

/// Render a Divider as a GTK4 Separator - takes ownership.
pub fn render_divider(orientation: Orientation, _env: &Environment) -> Widget {
    let separator = Separator::new(orientation);
    separator.upcast()
}

/// Render a horizontal divider.
pub fn render_horizontal_divider(env: &Environment) -> Widget {
    render_divider(Orientation::Horizontal, env)
}

/// Render a vertical divider.
pub fn render_vertical_divider(env: &Environment) -> Widget {
    render_divider(Orientation::Vertical, env)
}

/// Apply padding around a widget by wrapping it in a container.
pub fn render_with_padding(padding: Metadata<Edge>, env: &Environment) -> Widget {
    let edge = padding.value;
    let widget = render(padding.content, env);
    widget.set_margin_top(value(edge.top));
    widget.set_margin_bottom(value(edge.bottom));
    widget.set_margin_start(value(edge.left));
    widget.set_margin_end(value(edge.right));

    widget.upcast()
}

fn value(f: f64) -> i32 {
    if f.is_nan() { 10 } else { f as i32 }
}
