//! Shape views and shape clips rendered end to end.
//!
//! Every case is laid out **non-square** on purpose. A backend that resolves
//! geometry from the unit-space command list instead of the shape kind passes
//! square cases and fails these: unit coordinates are normalized per axis, so
//! a corner radius stretches with the aspect ratio and a circular corner comes
//! out elliptical.
//!
//! Run with `--no-capture` to export `/tmp/waterui_dew_shapes.png` for visual
//! review.

use std::io::Cursor;

use waterui::prelude::*;
use waterui::shape::{Capsule, Circle, Path, RoundedRectangle, ShapeExt};
use waterui_dew::render_view_png;

mod support;

/// A decoded frame, addressable by pixel.
struct Snapshot {
    pixels: Vec<[u8; 3]>,
    width: usize,
}

impl Snapshot {
    fn render<V: View>(build: impl Fn() -> V + 'static, width: u32, height: u32) -> Self {
        let png = render_view_png(build, support::test_environment(), width, height);
        Self::decode(&png, width)
    }

    fn decode(png: &[u8], width: u32) -> Self {
        let pixmap = vello_cpu::Pixmap::from_png(Cursor::new(png)).expect("png decodes back");
        Self {
            pixels: pixmap
                .data()
                .iter()
                .map(|pixel| [pixel.r, pixel.g, pixel.b])
                .collect(),
            width: width as usize,
        }
    }

    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        self.pixels[y * self.width + x]
    }

    /// Whether the pixel is the shape's fill rather than the empty background.
    fn is_filled(&self, x: usize, y: usize) -> bool {
        let [red, green, blue] = self.pixel(x, y);
        red > 150 && green < 100 && blue < 100
    }

    fn assert_filled(&self, x: usize, y: usize, why: &str) {
        assert!(
            self.is_filled(x, y),
            "({x}, {y}) should be inside the shape ({why}), got {:?}",
            self.pixel(x, y)
        );
    }

    fn assert_empty(&self, x: usize, y: usize, why: &str) {
        assert!(
            !self.is_filled(x, y),
            "({x}, {y}) should be outside the shape ({why}), got {:?}",
            self.pixel(x, y)
        );
    }
}

fn fill_red(shape: impl ShapeExt) -> impl View {
    shape.fill(Color::red())
}

/// A circle is inscribed in its bounds: on a 120×60 view it is a 60px-wide
/// disc in the middle, not an ellipse filling the box.
#[test]
fn a_circle_is_inscribed_rather_than_stretched() {
    let frame = Snapshot::render(|| fill_red(Circle), 120, 60);
    frame.assert_filled(60, 30, "the centre of the inscribed disc");
    frame.assert_filled(35, 30, "25px left of centre, inside the 30px radius");
    frame.assert_empty(10, 30, "50px left of centre, outside the disc");
    frame.assert_empty(110, 30, "50px right of centre, outside the disc");
}

/// A capsule's corners are circular arcs of the shorter side's half, so a
/// 120×60 capsule has a flat top edge from x = 30 to x = 90. Corners derived
/// from the per-axis command list would sweep to x = 60 and leave (35, 2)
/// outside the shape.
#[test]
fn capsule_corners_are_circular() {
    let frame = Snapshot::render(|| fill_red(Capsule), 120, 60);
    frame.assert_filled(35, 2, "the flat top edge begins 30px in");
    frame.assert_filled(2, 30, "the left cap touches the edge at mid-height");
    frame.assert_empty(2, 2, "the top-left corner is outside the cap");
}

/// A rounded rectangle's radius is a fraction of the **shorter** side: 0.5 on
/// a 200×40 view is 20px, not 100px.
#[test]
fn rounded_rectangle_radius_follows_the_shorter_side() {
    let frame = Snapshot::render(|| fill_red(RoundedRectangle::new(0.5)), 200, 40);
    frame.assert_filled(25, 2, "the flat top edge begins 20px in");
    frame.assert_filled(2, 20, "the left arc reaches the edge at mid-height");
    frame.assert_empty(1, 1, "the corner is rounded away");
}

/// A clip resolves the same geometry as a fill. Clipping a filled rect to a
/// circle must mask exactly the pixels the circle would have painted.
#[test]
fn a_circle_clip_masks_what_the_circle_fill_paints() {
    let frame = Snapshot::render(|| Color::red().clip(Circle), 120, 60);
    frame.assert_filled(60, 30, "the centre survives the mask");
    frame.assert_filled(35, 30, "25px left of centre is inside the disc");
    frame.assert_empty(10, 30, "the mask removes everything outside the disc");
    frame.assert_empty(2, 2, "the corners are masked away");
}

/// A capsule clip is a stadium, not a disc: the whole mid-height band survives.
#[test]
fn a_capsule_clip_keeps_the_middle_band() {
    let frame = Snapshot::render(|| Color::red().clip(Capsule), 120, 60);
    frame.assert_filled(10, 30, "a capsule spans the full width at mid-height");
    frame.assert_filled(110, 30, "including the far side");
    frame.assert_empty(2, 2, "the rounded corner is still masked");
}

/// A custom path has nothing but its unit-space commands, so it scales with
/// each axis — the deliberate exception to resolving geometry from the kind.
#[test]
fn a_custom_path_scales_with_both_axes() {
    let triangle = Path::new()
        .move_to(0.5, 0.0)
        .line_to(1.0, 1.0)
        .line_to(0.0, 1.0)
        .close();
    let frame = Snapshot::render(move || fill_red(triangle.clone()), 120, 60);
    frame.assert_filled(60, 55, "deep inside the triangle");
    frame.assert_empty(5, 5, "above the left edge");
    frame.assert_empty(115, 5, "above the right edge");
}

/// Visual review artifact: the built-in kinds at deliberately non-square
/// sizes, where a shape resolved from unit-space commands would look wrong.
#[test]
fn export_shapes_for_visual_review() {
    let png = render_view_png(
        || {
            vstack((
                hstack((fill_red(Circle), Capsule.fill(Color::blue()))).spacing(16.0),
                hstack((
                    RoundedRectangle::new(0.5).fill(Color::green()),
                    Color::cyan().clip(Circle),
                ))
                .spacing(16.0),
            ))
            .spacing(16.0)
            .padding()
        },
        support::test_environment(),
        320,
        180,
    );
    std::fs::write("/tmp/waterui_dew_shapes.png", png).expect("export visual review PNG");
}
