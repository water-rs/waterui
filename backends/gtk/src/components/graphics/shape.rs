//! GTK `ResolvedShape` component implementation.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::Widget;
use gtk4::glib;
use gtk4::prelude::*;
use waterui::shape::{PathCommand, ResolvedShape};
use waterui_core::{Environment, Native};
use waterui_graphics::color::ResolvedColor;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::{resolved_color_to_srgba_f64, store_watcher_guard, subscribe_then_get};

impl GtkComponent for Native<ResolvedShape> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let resolved = self.into_inner();

        let area = gtk4::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);

        // The resolved fill stays reactive to theme changes, so the draw function
        // reads the latest color from a shared cell that the watcher repaints on.
        let fill = Rc::new(Cell::new(ResolvedColor::default()));
        let (initial_fill, fill_guard) = subscribe_then_get(&resolved.fill, {
            let area = area.clone();
            let fill = Rc::clone(&fill);
            move |ctx| {
                let color = ctx.into_value();
                let area = area.clone();
                let fill = Rc::clone(&fill);
                glib::idle_add_local_once(move || {
                    fill.set(color);
                    area.queue_draw();
                });
            }
        });
        fill.set(initial_fill);
        store_watcher_guard(&area, Box::new(fill_guard));

        let commands = resolved.commands;
        area.set_draw_func(move |_area, cr, width, height| {
            let width = f64::from(width);
            let height = f64::from(height);
            let (red, green, blue, alpha) = to_rgba(fill.get());

            cr.new_path();
            for command in &commands {
                apply_path_command(cr, *command, width, height);
            }

            cr.set_source_rgba(red, green, blue, alpha);
            cr.fill().expect("failed to draw resolved shape");
        });

        area.upcast()
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "GTK widget geometry is integer pixels while WaterUI layout is f32"
)]
fn apply_path_command(cr: &gtk4::cairo::Context, command: PathCommand, width: f64, height: f64) {
    match command {
        PathCommand::MoveTo { x, y } => {
            cr.move_to(f64::from(x) * width, f64::from(y) * height);
        }
        PathCommand::LineTo { x, y } => {
            cr.line_to(f64::from(x) * width, f64::from(y) * height);
        }
        PathCommand::QuadTo { cx, cy, x, y } => {
            let cx = f64::from(cx) * width;
            let cy = f64::from(cy) * height;
            let x = f64::from(x) * width;
            let y = f64::from(y) * height;

            let (sx, sy) = cr
                .current_point()
                .expect("quad command requires an active current point");
            let c1x = f64::mul_add((2.0 / 3.0), cx - sx, sx);
            let c1y = f64::mul_add((2.0 / 3.0), cy - sy, sy);
            let c2x = f64::mul_add((2.0 / 3.0), cx - x, x);
            let c2y = f64::mul_add((2.0 / 3.0), cy - y, y);
            cr.curve_to(c1x, c1y, c2x, c2y, x, y);
        }
        PathCommand::CubicTo {
            c1x,
            c1y,
            c2x,
            c2y,
            x,
            y,
        } => {
            cr.curve_to(
                f64::from(c1x) * width,
                f64::from(c1y) * height,
                f64::from(c2x) * width,
                f64::from(c2y) * height,
                f64::from(x) * width,
                f64::from(y) * height,
            );
        }
        PathCommand::Arc {
            cx,
            cy,
            rx,
            ry,
            start,
            sweep,
        } => {
            let center_x = f64::from(cx) * width;
            let center_y = f64::from(cy) * height;
            let radius_x = f64::from(rx) * width;
            let radius_y = f64::from(ry) * height;

            let segments = 32usize;
            let start = f64::from(start);
            let step = f64::from(sweep) / segments as f64;
            let start_x = radius_x.mul_add(start.cos(), center_x);
            let start_y = radius_y.mul_add(start.sin(), center_y);

            cr.line_to(start_x, start_y);
            let mut angle = start;
            for _ in 0..segments {
                angle += step;
                let x = radius_x.mul_add(angle.cos(), center_x);
                let y = radius_y.mul_add(angle.sin(), center_y);
                cr.line_to(x, y);
            }
        }
        PathCommand::Close => {
            cr.close_path();
        }
    }
}

fn to_rgba(color: ResolvedColor) -> (f64, f64, f64, f64) {
    resolved_color_to_srgba_f64(color)
}
