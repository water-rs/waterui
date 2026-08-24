//! GTK `ResolvedShape` component implementation.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::Widget;
use gtk4::glib;
use gtk4::prelude::*;
use waterui::shape::{PathCommand, ResolvedShape, ShapeKind};
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
        let kind = resolved.kind;
        area.set_draw_func(move |_area, cr, width, height| {
            let width = f64::from(width);
            let height = f64::from(height);
            let (red, green, blue, alpha) = to_rgba(fill.get());

            cr.new_path();
            append_shape(cr, kind, &commands, width, height);

            cr.set_source_rgba(red, green, blue, alpha);
            cr.fill().expect("failed to draw resolved shape");
        });

        area.upcast()
    }
}

/// Appends the shape's outline, resolved against the size it is drawn at.
///
/// The path commands are in unit space, so a corner traced from them stretches
/// with the rect: a rounded rectangle far wider than it is tall gets flat
/// elliptical corners sweeping the whole edge. The kind carries what the
/// commands cannot — a corner radius as a fraction of the *shorter* side — and
/// only a custom path has nothing better than the commands to describe it.
fn append_shape(
    cr: &gtk4::cairo::Context,
    kind: ShapeKind,
    commands: &[PathCommand],
    width: f64,
    height: f64,
) {
    let shorter = width.min(height);
    let limit = shorter / 2.0;
    let scaled = |radius: f32| (f64::from(radius) * shorter).clamp(0.0, limit);
    match kind {
        ShapeKind::Rect => cr.rectangle(0.0, 0.0, width, height),
        // A circle is inscribed in the bounds: centred, its diameter the
        // shorter side.
        ShapeKind::Circle => append_ellipse(cr, width / 2.0, height / 2.0, limit, limit),
        ShapeKind::Ellipse => {
            append_ellipse(cr, width / 2.0, height / 2.0, width / 2.0, height / 2.0)
        }
        ShapeKind::RoundedRect { corner_radius } => {
            let radius = scaled(corner_radius);
            append_rounded_rect(cr, width, height, [radius; 4]);
        }
        ShapeKind::UnevenRoundedRect {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        } => append_rounded_rect(
            cr,
            width,
            height,
            [
                scaled(top_left),
                scaled(top_right),
                scaled(bottom_right),
                scaled(bottom_left),
            ],
        ),
        ShapeKind::Capsule => append_rounded_rect(cr, width, height, [limit; 4]),
        ShapeKind::CustomPath => {
            for command in commands {
                apply_path_command(cr, *command, width, height);
            }
        }
    }
}

/// Traces an axis-aligned ellipse, which cairo has no primitive for.
fn append_ellipse(
    cr: &gtk4::cairo::Context,
    center_x: f64,
    center_y: f64,
    radius_x: f64,
    radius_y: f64,
) {
    cr.save().expect("failed to save the cairo state");
    cr.translate(center_x, center_y);
    cr.scale(radius_x.max(f64::EPSILON), radius_y.max(f64::EPSILON));
    cr.arc(0.0, 0.0, 1.0, 0.0, core::f64::consts::TAU);
    cr.restore().expect("failed to restore the cairo state");
}

/// Traces a rounded rectangle from four circular corner radii, clockwise from
/// the top left.
fn append_rounded_rect(
    cr: &gtk4::cairo::Context,
    width: f64,
    height: f64,
    [top_left, top_right, bottom_right, bottom_left]: [f64; 4],
) {
    use core::f64::consts::{FRAC_PI_2, PI};
    cr.move_to(top_left, 0.0);
    cr.line_to(width - top_right, 0.0);
    cr.arc(width - top_right, top_right, top_right, -FRAC_PI_2, 0.0);
    cr.line_to(width, height - bottom_right);
    cr.arc(
        width - bottom_right,
        height - bottom_right,
        bottom_right,
        0.0,
        FRAC_PI_2,
    );
    cr.line_to(bottom_left, height);
    cr.arc(
        bottom_left,
        height - bottom_left,
        bottom_left,
        FRAC_PI_2,
        PI,
    );
    cr.line_to(0.0, top_left);
    cr.arc(top_left, top_left, top_left, PI, PI + FRAC_PI_2);
    cr.close_path();
}

/// Appends one resolved path command to the cairo context.
///
/// # Panics
///
/// Panics if a quadratic command is emitted with no current point.
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
            let c1x = f64::mul_add(2.0 / 3.0, cx - sx, sx);
            let c1y = f64::mul_add(2.0 / 3.0, cy - sy, sy);
            let c2x = f64::mul_add(2.0 / 3.0, cx - x, x);
            let c2y = f64::mul_add(2.0 / 3.0, cy - y, y);
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
