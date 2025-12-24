//! GTK4 Color component implementation.
//!
//! Renders a solid color rectangle using cairo drawing.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::Widget;
use nami::Signal;
use waterui_graphics::color::Color;
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<Color> {
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let color = self.into_inner();
        let drawing_area = gtk4::DrawingArea::new();
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);

        // Resolve color reactively
        let resolved = color.resolve(env);

        // Store resolved color in Rc<RefCell> for sharing between draw func and watcher
        let current_color = Rc::new(RefCell::new(resolved.get()));

        // Set up cairo draw callback
        let color_ref = current_color.clone();
        drawing_area.set_draw_func(move |_, cr, width, height| {
            let c = *color_ref.borrow();
            let srgb = c.to_srgb_with_headroom();
            cr.set_source_rgba(
                f64::from(srgb.red.clamp(0.0, 1.0)),
                f64::from(srgb.green.clamp(0.0, 1.0)),
                f64::from(srgb.blue.clamp(0.0, 1.0)),
                f64::from(c.opacity.clamp(0.0, 1.0)),
            );
            cr.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
            let _ = cr.fill();
        });

        // Watch for color changes and redraw
        let color_ref = current_color;
        let drawing_area_clone = drawing_area.clone();
        let guard = resolved.watch(move |ctx| {
            let new_color = ctx.into_value();
            *color_ref.borrow_mut() = new_color;
            // Queue redraw on GTK main thread
            let area = drawing_area_clone.clone();
            glib::idle_add_local_once(move || {
                area.queue_draw();
            });
        });

        store_watcher_guard(&drawing_area, guard);

        drawing_area.upcast()
    }
}
