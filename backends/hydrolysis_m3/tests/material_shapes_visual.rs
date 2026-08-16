//! Visual acceptance for the Material shape set the loading indicator morphs
//! through.
//!
//! Numbers only say each outline closes; whether it is the right silhouette is
//! a question for the eye. The PNG shows all seven, plus a morph mid-flight.

use hydrolysis_m3::material_shapes::{material_shape_sequence, morph, radii_to_path};
use vello::kurbo::Point;
use waterui::layout::Point as CanvasPoint;
use waterui::prelude::*;
use waterui_canvas::{Canvas, DrawingContext};
use waterui_testing::{OffscreenApp, UiBuilder};

const CELL: f32 = 72.0;
const RADIUS: f64 = 30.0;

/// Draws one shape's outline centred in its own cell.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the canvas takes f32 coordinates; the outline is a few hundred points across"
)]
fn draw_radii(ctx: &mut DrawingContext<'_>, radii: &[f64], centre: Point) {
    let outline = radii_to_path(radii, centre, RADIUS);
    let mut path = ctx.begin_path();
    let mut first = true;
    vello::kurbo::flatten(outline.iter(), 0.05, |element| match element {
        vello::kurbo::PathEl::MoveTo(point) | vello::kurbo::PathEl::LineTo(point) => {
            let point = CanvasPoint::new(point.x as f32, point.y as f32);
            if first {
                path.move_to(point);
                first = false;
            } else {
                path.line_to(point);
            }
        }
        _ => {}
    });
    path.close();
    ctx.fill_path(&path);
}

#[expect(
    clippy::cast_precision_loss,
    reason = "the grid is four cells wide and two tall"
)]
fn shapes() -> impl View {
    Canvas::new(|ctx: &mut DrawingContext<'_>| {
        let sequence = material_shape_sequence();
        ctx.set_fill_style(waterui::color::ResolvedColor {
            red: 0.196,
            green: 0.184,
            blue: 0.208,
            headroom: 1.0,
            opacity: 1.0,
        });
        for (index, radii) in sequence.iter().enumerate() {
            let column = index % 4;
            let row = index / 4;
            let centre = Point::new(
                f64::from(CELL) * (column as f64 + 0.5),
                f64::from(CELL) * (row as f64 + 0.5),
            );
            draw_radii(ctx, radii, centre);
        }
        // The eighth cell shows a morph halfway between the last two shapes.
        let halfway = morph(&sequence[5], &sequence[6], 0.5);
        draw_radii(
            ctx,
            &halfway,
            Point::new(f64::from(CELL) * 3.5, f64::from(CELL) * 1.5),
        );
    })
    .size(CELL * 4.0, CELL * 2.0)
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (288, 144))]
fn the_material_shape_set_renders(ui: UiBuilder) {
    let mut app: OffscreenApp = ui.mount_offscreen(shapes);
    let _ = app.capture_snapshot("material3-preview", "material-shapes", "sequence");
}
