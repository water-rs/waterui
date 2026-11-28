//! Pixel tests for gradient-filled shapes (`Paint`) and alpha `.mask(...)`
//! compositing in Hydrolysis.

use std::time::Instant;

use waterui::component::text;
use waterui::graphics::{Color, Gradient};
use waterui::layout::UnitPoint;
use waterui::shape::{Capsule, Circle, ShapeExt as _};
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

#[test]
fn gradient_stroked_circle_paints_ring_and_leaves_center_and_corners_clear() {
    let frame = snapshot(100, 100, || {
        zstack((
            Color::srgb(0, 180, 0),
            Circle
                .stroke(
                    Gradient::linear(
                        [Color::srgb(255, 0, 0), Color::srgb(0, 0, 255)],
                        UnitPoint::LEADING,
                        UnitPoint::TRAILING,
                    ),
                    10.0,
                )
                .size(100.0, 100.0),
        ))
    });

    let corner = pixel(&frame, 4, 4);
    assert!(
        corner[1] > 140 && corner[0] < 30 && corner[2] < 30,
        "corner outside stroked circle must keep green backdrop, got {corner:?}"
    );

    let center = pixel(&frame, 50, 50);
    assert!(
        center[1] > 140 && center[0] < 30 && center[2] < 30,
        "center inside stroked circle must remain transparent to green backdrop, got {center:?}"
    );

    let left_ring = pixel(&frame, 5, 50);
    assert!(
        left_ring[0] > 150 && left_ring[0] > left_ring[2].saturating_add(60),
        "left edge of gradient-stroked circle must be red-dominant, got {left_ring:?}"
    );

    let right_ring = pixel(&frame, 95, 50);
    assert!(
        right_ring[2] > 150 && right_ring[2] > right_ring[0].saturating_add(60),
        "right edge of gradient-stroked circle must be blue-dominant, got {right_ring:?}"
    );
}

#[test]
fn border_shape_strokes_capsule_without_clipping_content() {
    let frame = snapshot(160, 80, || {
        zstack((
            Color::srgb(0, 180, 0),
            Color::srgb(240, 240, 240).size(160.0, 80.0).border_shape(
                Capsule,
                Color::srgb(220, 20, 20),
                8.0,
            ),
        ))
    });

    // Capsule border strokes along the pill rim (e.g., top-center (80, 4) and left-mid (4, 40)),
    // while leaving the interior white and NOT clipping the rectangular content at corner (4, 4).
    let top_rim = pixel(&frame, 80, 4);
    assert!(
        top_rim[0] > 180 && top_rim[1] < 60 && top_rim[2] < 60,
        "top rim of capsule border must be red, got {top_rim:?}"
    );

    let left_rim = pixel(&frame, 4, 40);
    assert!(
        left_rim[0] > 180 && left_rim[1] < 60 && left_rim[2] < 60,
        "left arc of capsule border must be red, got {left_rim:?}"
    );

    let center = pixel(&frame, 80, 40);
    assert!(
        center[0] > 220 && center[1] > 220 && center[2] > 220,
        "interior must keep white content, got {center:?}"
    );

    let corner = pixel(&frame, 4, 4);
    assert!(
        corner[0] > 220 && corner[1] > 220 && corner[2] > 220,
        "capsule border must not clip rectangular content at corners, got {corner:?}"
    );
}
