//! GTK4 general widget implementations.

use gtk4::{Box, Orientation, ProgressBar, Separator, Widget, prelude::*};
use waterui::{
    Environment,
    component::{Metadata, layout::Edge},
};

use crate::renderer::render;

/// Render a Progress component as a GTK4 ProgressBar - takes ownership.
pub fn render_progress(progress_value: f64, _env: &Environment) -> Widget {
    let progress_bar = ProgressBar::new();

    // Clamp progress value between 0.0 and 1.0
    let clamped_value = progress_value.clamp(0.0, 1.0);
    progress_bar.set_fraction(clamped_value);

    // Show percentage text
    progress_bar.set_show_text(true);
    progress_bar.set_text(Some(&format!("{:.0}%", clamped_value * 100.0)));

    progress_bar.upcast()
}

/// Render a loading Progress component (indeterminate) as a GTK4 ProgressBar.
pub fn render_loading_progress(_env: &Environment) -> Widget {
    let progress_bar = ProgressBar::new();

    // Enable pulse mode for indeterminate progress
    progress_bar.pulse();
    progress_bar.set_show_text(true);
    progress_bar.set_text(Some("Loading..."));

    progress_bar.upcast()
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
