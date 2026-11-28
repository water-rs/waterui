//! Pixel tests for gradient-filled shapes (`Paint`) and alpha `.mask(...)`
//! compositing in Hydrolysis.

use std::time::Instant;

use waterui::component::text;
use waterui::graphics::{Color, Gradient};
use waterui::layout::UnitPoint;
use waterui::shape::{Circle, ShapeExt as _};
use waterui::{Binding, View, ViewExt as _};
use waterui_core::handler::AnyViewBuilder;
use waterui_layout::stack::zstack;

use super::{MinimalTestTheme, pumped_test_environment};
use crate::HeadlessRuntime;

fn pixel(snapshot: &crate::runner::HeadlessSnapshot, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * snapshot.width() + x) * 4) as usize;
    snapshot.rgba8()[offset..offset + 4]
        .try_into()
        .expect("pixel in bounds")
}

fn snapshot<V: View + 'static>(
    width: u32,
    height: u32,
    view: impl Fn() -> V + 'static,
) -> crate::runner::HeadlessSnapshot {
    let keep = Binding::new(0_i32);
    let builder = AnyViewBuilder::<waterui_core::AnyView>::new(move || {
        let _ = &keep;
        waterui_core::AnyView::new(view())
    });
    let env = pumped_test_environment();
    let mut runtime =
        HeadlessRuntime::new_for_tests(env, builder, width, height, MinimalTestTheme::default());
    runtime
        .pump_at(true, Instant::now())
        .into_snapshot()
        .expect("frame must produce a snapshot")
}

#[test]
fn gradient_filled_circle_paints_gradient_inside_and_keeps_corners_clear() {
    let frame = snapshot(100, 100, || {
        zstack((
            Color::srgb(0, 180, 0),
            Circle
                .fill(Gradient::linear(
                    [Color::srgb(255, 0, 0), Color::srgb(0, 0, 255)],
                    UnitPoint::LEADING,
                    UnitPoint::TRAILING,
                ))
                .size(100.0, 100.0),
        ))
    });

    let corner = pixel(&frame, 4, 4);
    assert!(
        corner[1] > 140 && corner[0] < 30 && corner[2] < 30,
        "corner outside circle must keep green backdrop, got {corner:?}"
    );

    let left = pixel(&frame, 20, 50);
    assert!(
        left[0] > 150 && left[0] > left[2].saturating_add(60),
        "left interior of gradient-filled circle must be red-dominant, got {left:?}"
    );

    let right = pixel(&frame, 80, 50);
    assert!(
        right[2] > 150 && right[2] > right[0].saturating_add(60),
        "right interior of gradient-filled circle must be blue-dominant, got {right:?}"
    );
}

#[test]
fn solid_color_masked_by_circle_isolates_backdrop() {
    let frame = snapshot(100, 100, || {
        zstack((
            Color::srgb(0, 180, 0),
            Color::srgb(255, 0, 0)
                .size(100.0, 100.0)
                .mask(Circle.fill(Color::srgb(255, 255, 255))),
        ))
    });

    let corner = pixel(&frame, 4, 4);
    assert!(
        corner[1] > 140 && corner[0] < 30 && corner[2] < 30,
        "corner outside circle mask must preserve green backdrop, got {corner:?}"
    );

    let center = pixel(&frame, 50, 50);
    assert!(
        center[0] > 200 && center[1] < 30 && center[2] < 30,
        "center inside circle mask must show red content, got {center:?}"
    );
}

#[test]
fn gradient_masked_by_text_reveals_gradient_glyphs_without_erasing_backdrop() {
    let frame = snapshot(200, 80, || {
        zstack((
            Color::srgb(0, 180, 0),
            Gradient::linear(
                [Color::srgb(255, 0, 0), Color::srgb(0, 0, 255)],
                UnitPoint::LEADING,
                UnitPoint::TRAILING,
            )
            .size(200.0, 80.0)
            .mask(text("MMMMMM").font_size(32.0).bold(true)),
        ))
    });

    let corner = pixel(&frame, 4, 4);
    assert!(
        corner[1] > 140 && corner[0] < 30 && corner[2] < 30,
        "corner outside masked text must preserve green backdrop, got {corner:?}"
    );

    let mut left_red_glyph_pixels = 0_u32;
    let mut right_blue_glyph_pixels = 0_u32;
    for y in 12..68 {
        for x in 10..100 {
            let px = pixel(&frame, x, y);
            if px[0] > 130 && px[0] > px[1].saturating_add(30) && px[0] > px[2].saturating_add(30) {
                left_red_glyph_pixels += 1;
            }
        }
        for x in 100..190 {
            let px = pixel(&frame, x, y);
            if px[2] > 130 && px[2] > px[1].saturating_add(30) && px[2] > px[0].saturating_add(30) {
                right_blue_glyph_pixels += 1;
            }
        }
    }

    assert!(
        left_red_glyph_pixels > 25,
        "left half of gradient-masked text must contain red gradient glyph pixels, got {left_red_glyph_pixels}"
    );
    assert!(
        right_blue_glyph_pixels > 25,
        "right half of gradient-masked text must contain blue gradient glyph pixels, got {right_blue_glyph_pixels}"
    );
}
