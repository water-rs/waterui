//! GTK `ResolvedShape` component implementation.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::Widget;
use gtk4::glib;
use gtk4::prelude::*;
use waterui::shape::{PathCommand, ResolvedShape, ShapeKind};
use waterui_core::{Environment, Native};
use waterui_graphics::color::ResolvedColor;

use super::gsk_path;
use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::shape_geometry::{Corner, RoundedRect, ShapeGeometry, resolve};
use crate::shape_path::bez_path;
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
/// The geometry comes from [`crate::shape_geometry`], which is also what the
/// clip widget resolves through, so a clipped fill and its clip agree. A custom
/// path resolves through [`crate::shape_path`] for the same reason: the clip
/// fills the very path this traces.
fn append_shape(
    cr: &gtk4::cairo::Context,
    kind: ShapeKind,
    commands: &[PathCommand],
    width: f64,
    height: f64,
) {
    match resolve(kind, width, height) {
        ShapeGeometry::Rounded(rect) => append_rounded_rect(cr, &rect),
        ShapeGeometry::CustomPath => {
            gsk_path(&bez_path(commands, width, height)).to_cairo(cr);
        }
    }
}

/// Traces a rounded rectangle, clockwise from the top left.
///
/// A circle or an ellipse arrives here as a rect whose corner radii are half its
/// own width and height, so the four arcs meet with no straight edge between
/// them and the same routine draws every non-custom shape.
fn append_rounded_rect(cr: &gtk4::cairo::Context, rect: &RoundedRect) {
    use core::f64::consts::{FRAC_PI_2, PI};

    let RoundedRect {
        x: left,
        y: top,
        width,
        height,
        corners: [top_left, top_right, bottom_right, bottom_left],
    } = *rect;
    let right = left + width;
    let bottom = top + height;

    cr.move_to(left + top_left.horizontal, top);
    cr.line_to(right - top_right.horizontal, top);
    append_corner_arc(
        cr,
        right - top_right.horizontal,
        top + top_right.vertical,
        top_right,
        -FRAC_PI_2,
        0.0,
    );
    cr.line_to(right, bottom - bottom_right.vertical);
    append_corner_arc(
        cr,
        right - bottom_right.horizontal,
        bottom - bottom_right.vertical,
        bottom_right,
        0.0,
        FRAC_PI_2,
    );
    cr.line_to(left + bottom_left.horizontal, bottom);
    append_corner_arc(
        cr,
        left + bottom_left.horizontal,
        bottom - bottom_left.vertical,
        bottom_left,
        FRAC_PI_2,
        PI,
    );
    cr.line_to(left, top + top_left.vertical);
    append_corner_arc(
        cr,
        left + top_left.horizontal,
        top + top_left.vertical,
        top_left,
        PI,
        PI + FRAC_PI_2,
    );
    cr.close_path();
}

/// Traces one corner as an elliptical arc, which cairo can only express by
/// scaling a unit circle.
fn append_corner_arc(
    cr: &gtk4::cairo::Context,
    center_x: f64,
    center_y: f64,
    corner: Corner,
    start: f64,
    end: f64,
) {
    if corner.horizontal <= 0.0 || corner.vertical <= 0.0 {
        // A square corner: the two edges already meet at the centre point.
        cr.line_to(center_x, center_y);
        return;
    }

    cr.save().expect("failed to save the cairo state");
    cr.translate(center_x, center_y);
    cr.scale(corner.horizontal, corner.vertical);
    cr.arc(0.0, 0.0, 1.0, start, end);
    cr.restore().expect("failed to restore the cairo state");
}

fn to_rgba(color: ResolvedColor) -> (f64, f64, f64, f64) {
    resolved_color_to_srgba_f64(color)
}
