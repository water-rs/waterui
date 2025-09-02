//! Shape rendering for GTK4 backend.

use glib::clone;
use gtk4::{DrawingArea, Widget, prelude::*};
use waterui::{
    Color, Environment, Signal,
    shape::{Circle, Rectangle, RoundedRectangle},
};

/// Renders a rectangle shape as a GTK4 DrawingArea widget.
pub fn render_rectangle(_rectangle: Rectangle, env: &Environment) -> Widget {
    let drawing_area = DrawingArea::new();

    // Set a reasonable default size
    drawing_area.set_content_width(100);
    drawing_area.set_content_height(60);

    // Get background color from environment if available
    let bg_color = env
        .get::<Color>()
        .map(|color| color.rgba())
        .unwrap_or((0.2, 0.6, 0.8, 1.0)); // Default blue

    drawing_area.set_draw_func(move |_, cr, width, height| {
        // Fill the rectangle with the specified color
        cr.set_source_rgba(
            bg_color.0 as f64,
            bg_color.1 as f64,
            bg_color.2 as f64,
            bg_color.3 as f64,
        );
        cr.rectangle(0.0, 0.0, width as f64, height as f64);
        cr.fill().expect("Failed to fill rectangle");
    });

    drawing_area.upcast()
}

/// Renders a rounded rectangle shape as a GTK4 DrawingArea widget.
pub fn render_rounded_rectangle(rounded_rect: RoundedRectangle, env: &Environment) -> Widget {
    let drawing_area = DrawingArea::new();

    // Set a reasonable default size
    drawing_area.set_content_width(100);
    drawing_area.set_content_height(60);

    // Get the corner radius value
    let radius = rounded_rect.radius.get();

    // Get background color from environment if available
    let bg_color = env
        .get::<Color>()
        .map(|color| color.rgba())
        .unwrap_or((0.2, 0.6, 0.8, 1.0)); // Default blue

    drawing_area.set_draw_func(clone!(move |_, cr, width, height| {
        let w = width as f64;
        let h = height as f64;
        let r = radius.min(w / 2.0).min(h / 2.0); // Clamp radius to reasonable bounds

        // Draw rounded rectangle using Cairo path
        cr.new_path();
        cr.arc(
            r,
            r,
            r,
            std::f64::consts::PI,
            3.0 * std::f64::consts::PI / 2.0,
        );
        cr.arc(w - r, r, r, 3.0 * std::f64::consts::PI / 2.0, 0.0);
        cr.arc(w - r, h - r, r, 0.0, std::f64::consts::PI / 2.0);
        cr.arc(
            r,
            h - r,
            r,
            std::f64::consts::PI / 2.0,
            std::f64::consts::PI,
        );
        cr.close_path();

        // Fill with the specified color
        cr.set_source_rgba(
            bg_color.0 as f64,
            bg_color.1 as f64,
            bg_color.2 as f64,
            bg_color.3 as f64,
        );
        cr.fill().expect("Failed to fill rounded rectangle");
    }));

    drawing_area.upcast()
}

/// Renders a circle shape as a GTK4 DrawingArea widget.
pub fn render_circle(_circle: Circle, env: &Environment) -> Widget {
    let drawing_area = DrawingArea::new();

    // Set a reasonable default size (square for perfect circle)
    drawing_area.set_content_width(80);
    drawing_area.set_content_height(80);

    // Get background color from environment if available
    let bg_color = env
        .get::<Color>()
        .map(|color| color.rgba())
        .unwrap_or((0.2, 0.6, 0.8, 1.0)); // Default blue

    drawing_area.set_draw_func(move |_, cr, width, height| {
        let center_x = width as f64 / 2.0;
        let center_y = height as f64 / 2.0;
        let radius = (width.min(height) as f64 / 2.0) - 1.0; // Leave 1px margin

        // Draw circle
        cr.arc(center_x, center_y, radius, 0.0, 2.0 * std::f64::consts::PI);
        cr.set_source_rgba(
            bg_color.0 as f64,
            bg_color.1 as f64,
            bg_color.2 as f64,
            bg_color.3 as f64,
        );
        cr.fill().expect("Failed to fill circle");
    });

    drawing_area.upcast()
}
