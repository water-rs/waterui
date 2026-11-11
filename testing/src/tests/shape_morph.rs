//! Rendered coverage for `MorphShape`: four morphs sampled at five
//! progress steps, in a square and a wide box, through the offscreen
//! Hydrolysis renderer. The Apple backend's canvas lowering renders the same
//! grid in `backends/apple/src/components/canvas/tests.rs`; both draw the
//! path the shape crate derives, so the two sheets are meant to match by
//! eye. The image is written for review.

use std::path::PathBuf;

use hydrolysis_m3::Material3;
use waterui::View;
use waterui::ViewExt as _;
use waterui::component::{hstack, vstack};
use waterui::graphics::color::Color;
use waterui::shape::{Capsule, Circle, Ellipse, MorphShape, Rectangle, RoundedRectangle, ShapeExt};

use crate::ui;

/// A morph from one built-in shape to another, filled with `color`.
type Morph = fn(Color) -> MorphShape;

const STEPS: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

const SQUARE: (f32, f32) = (64.0, 64.0);
const WIDE: (f32, f32) = (112.0, 64.0);

const RECT_TO_CIRCLE: Morph = |color| Rectangle.morph_to(Circle, color);
const CIRCLE_TO_ELLIPSE: Morph = |color| Circle.morph_to(Ellipse, color);
const CAPSULE_TO_ELLIPSE: Morph = |color| Capsule.morph_to(Ellipse, color);
const ROUNDED_TO_CAPSULE: Morph = |color| RoundedRectangle::new(0.2).morph_to(Capsule, color);

fn output(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join("waterui_shape_morph");
    std::fs::create_dir_all(&directory).expect("the morph gallery directory is writable");
    directory.join(name)
}

/// One morph at the five progress steps, in a box of `size`.
fn row(morph: Morph, (width, height): (f32, f32)) -> impl View {
    let step = |progress: f32| {
        morph(Color::srgb_hex("#3B82F6"))
            .progress(progress)
            .size(width, height)
    };
    hstack((
        step(STEPS[0]),
        step(STEPS[1]),
        step(STEPS[2]),
        step(STEPS[3]),
        step(STEPS[4]),
    ))
    .spacing(8.0)
}

#[test]
fn morph_sequences_render_in_square_and_wide_boxes() {
    let mut app = ui()
        .viewport(608, 584)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            vstack((
                row(RECT_TO_CIRCLE, SQUARE),
                row(RECT_TO_CIRCLE, WIDE),
                row(CIRCLE_TO_ELLIPSE, SQUARE),
                row(CIRCLE_TO_ELLIPSE, WIDE),
                row(CAPSULE_TO_ELLIPSE, SQUARE),
                row(CAPSULE_TO_ELLIPSE, WIDE),
                row(ROUNDED_TO_CAPSULE, SQUARE),
                row(ROUNDED_TO_CAPSULE, WIDE),
            ))
            .spacing(8.0)
            .padding()
        });
    app.snapshot()
        .save_png(output("hydrolysis.png"))
        .expect("the morph gallery snapshot is writable");
}
