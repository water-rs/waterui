//! Geometric clipping consumes filled regions, not raw contour directions.
#![cfg(not(target_arch = "wasm32"))]

use cherenkov::kurbo::{BezPath, Rect};
use cherenkov::{
    Draw, Engine, EvenOdd, FillRule, FrameTime, Offscreen, OffscreenFormat, Recorder, WorkingColor,
};
use cherenkov_cpu::{Raster, RasterConfig};

const WIDTH: usize = 20;
const HEIGHT: usize = 12;
const COLOR: [f32; 4] = [1.5, 0.1, 0.0, 1.0];

fn rectangle(rect: Rect, reversed: bool) -> BezPath {
    let points = if reversed {
        [
            (rect.x0, rect.y0),
            (rect.x0, rect.y1),
            (rect.x1, rect.y1),
            (rect.x1, rect.y0),
        ]
    } else {
        [
            (rect.x0, rect.y0),
            (rect.x1, rect.y0),
            (rect.x1, rect.y1),
            (rect.x0, rect.y1),
        ]
    };
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for point in &points[1..] {
        path.line_to(*point);
    }
    path.close_path();
    path
}

fn render(
    draw: BezPath,
    rule: FillRule,
    clip: impl cherenkov::Shape + 'static,
    second: Option<BezPath>,
) -> Vec<[f32; 4]> {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("CPU engine");
    let surface = engine
        .surface(Offscreen::new((20, 12), OffscreenFormat::LinearF32), || {})
        .expect("linear offscreen surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|canvas| {
            canvas.clip(cherenkov::Fixed(clip), |canvas| {
                let fill = |canvas: &mut Recorder| match rule {
                    FillRule::NonZero => canvas.fill(draw, WorkingColor::new(COLOR)),
                    FillRule::EvenOdd => canvas.fill(EvenOdd(draw), WorkingColor::new(COLOR)),
                };
                if let Some(second) = second {
                    canvas.clip(second, fill);
                } else {
                    fill(canvas);
                }
            });
        }));
    });
    engine.render(FrameTime::now()).expect("render");
    surface.readback().expect("readback").pixels
}

fn sample(pixels: &[[f32; 4]], x: usize, y: usize, coverage: f32, tolerance: f32) {
    let actual = pixels[y * WIDTH + x];
    for (channel, (&actual, color)) in actual.iter().zip(COLOR).enumerate() {
        let expected = color * coverage;
        assert!(
            actual.is_finite() && (actual - expected).abs() < tolerance,
            "pixel ({x}, {y}) channel {channel}: got {actual}, expected {expected}"
        );
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "the test surface is 20 by 12 pixels"
)]
fn rectangle_pixels(pixels: &[[f32; 4]], intersection: Rect) {
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let left = (x as f64).max(intersection.x0);
            let right = ((x + 1) as f64).min(intersection.x1);
            let top = (y as f64).max(intersection.y0);
            let bottom = ((y + 1) as f64).min(intersection.y1);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "expected coverage is in [0, 1]"
            )]
            let coverage = ((right - left).max(0.0) * (bottom - top).max(0.0)) as f32;
            sample(pixels, x, y, coverage, 1e-5);
        }
    }
}

#[test]
fn rectangle_intersection_is_independent_of_both_contour_directions() {
    for draw_reversed in [false, true] {
        for clip_reversed in [false, true] {
            let pixels = render(
                rectangle(Rect::new(5.0, -5.0, 15.0, 5.0), draw_reversed),
                FillRule::NonZero,
                rectangle(Rect::new(0.0, 0.0, 10.0, 10.0), clip_reversed),
                None,
            );
            for (x, y, alpha) in [(7, 2, 1.0), (2, 2, 0.0), (12, 2, 0.0), (7, 7, 0.0)] {
                sample(&pixels, x, y, alpha, 0.001);
            }
        }
    }
}

#[test]
fn fractional_intersections_preserve_hdr_p3_under_both_fill_rules() {
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        for draw_reversed in [false, true] {
            for clip_reversed in [false, true] {
                let pixels = render(
                    rectangle(Rect::new(5.25, -5.0, 15.0, 5.75), draw_reversed),
                    rule,
                    rectangle(Rect::new(0.0, 0.0, 10.75, 10.0), clip_reversed),
                    None,
                );
                rectangle_pixels(&pixels, Rect::new(5.25, 0.0, 10.75, 5.75));
            }
        }
    }
}

#[test]
fn holes_and_duplicate_contours_keep_their_fill_semantics() {
    for outer_reversed in [false, true] {
        let outer = rectangle(Rect::new(0.0, 0.0, 12.0, 12.0), outer_reversed);
        for (rule, inner_reversed, middle) in [
            (FillRule::NonZero, !outer_reversed, 0.0),
            (FillRule::NonZero, outer_reversed, 1.0),
            (FillRule::EvenOdd, outer_reversed, 0.0),
        ] {
            let mut draw = outer.clone();
            draw.extend(rectangle(Rect::new(4.0, 4.0, 8.0, 8.0), inner_reversed));
            let pixels = render(
                draw,
                rule,
                rectangle(Rect::new(1.0, 1.0, 11.0, 11.0), true),
                None,
            );
            sample(&pixels, 2, 2, 1.0, 1e-5);
            sample(&pixels, 5, 5, middle, 1e-5);
            sample(&pixels, 0, 0, 0.0, 1e-5);
        }
        let mut duplicate = outer.clone();
        duplicate.extend(outer);
        for (rule, middle) in [(FillRule::NonZero, 1.0), (FillRule::EvenOdd, 0.0)] {
            let pixels = render(
                duplicate.clone(),
                rule,
                rectangle(Rect::new(1.0, 1.0, 11.0, 11.0), true),
                None,
            );
            sample(&pixels, 5, 5, middle, 1e-5);
        }
    }
}

#[test]
fn clip_holes_and_oppositely_directed_disjoint_draws_stay_filled() {
    let mut clip = rectangle(Rect::new(0.0, 0.0, 12.0, 12.0), false);
    clip.extend(rectangle(Rect::new(4.0, 4.0, 8.0, 8.0), true));
    let pixels = render(
        rectangle(Rect::new(0.0, 0.0, 12.0, 12.0), true),
        FillRule::NonZero,
        clip,
        None,
    );
    sample(&pixels, 2, 2, 1.0, 1e-5);
    sample(&pixels, 5, 5, 0.0, 1e-5);
    let mut draw = rectangle(Rect::new(1.0, 1.0, 4.0, 4.0), false);
    draw.extend(rectangle(Rect::new(8.0, 1.0, 11.0, 4.0), true));
    let pixels = render(
        draw,
        FillRule::NonZero,
        rectangle(Rect::new(0.0, 0.0, 12.0, 12.0), true),
        None,
    );
    sample(&pixels, 2, 2, 1.0, 1e-5);
    sample(&pixels, 9, 2, 1.0, 1e-5);
    sample(&pixels, 6, 2, 0.0, 1e-5);
}

#[test]
fn even_odd_clip_rule_is_applied_before_boundary_normalization() {
    let outer = rectangle(Rect::new(0.0, 0.0, 12.0, 12.0), false);
    let mut hole = outer.clone();
    hole.extend(rectangle(Rect::new(4.0, 4.0, 8.0, 8.0), false));
    let pixels = render(
        rectangle(Rect::new(0.0, 0.0, 12.0, 12.0), true),
        FillRule::NonZero,
        EvenOdd(hole),
        None,
    );
    sample(&pixels, 2, 2, 1.0, 1e-5);
    sample(&pixels, 5, 5, 0.0, 1e-5);
    let mut duplicate = outer.clone();
    duplicate.extend(outer);
    let pixels = render(
        rectangle(Rect::new(0.0, 0.0, 12.0, 12.0), true),
        FillRule::NonZero,
        EvenOdd(duplicate),
        None,
    );
    sample(&pixels, 2, 2, 0.0, 1e-5);
    sample(&pixels, 5, 5, 0.0, 1e-5);
}

#[test]
fn self_crossings_and_repeated_clips_use_region_intersections() {
    let mut bow_tie = BezPath::new();
    bow_tie.move_to((2.0, 2.0));
    bow_tie.line_to((10.0, 10.0));
    bow_tie.line_to((10.0, 2.0));
    bow_tie.line_to((2.0, 10.0));
    bow_tie.close_path();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let pixels = render(
            bow_tie.clone(),
            rule,
            rectangle(Rect::new(1.0, 1.0, 11.0, 11.0), false),
            None,
        );
        sample(&pixels, 3, 5, 1.0, 1e-5);
        sample(&pixels, 8, 5, 1.0, 1e-5);
        sample(&pixels, 5, 2, 0.0, 1e-5);
    }
    let pixels = render(
        rectangle(Rect::new(1.0, 1.0, 13.0, 13.0), false),
        FillRule::NonZero,
        rectangle(Rect::new(0.0, 4.0, 14.0, 10.0), true),
        Some(rectangle(Rect::new(5.0, 0.0, 9.0, 14.0), false)),
    );
    rectangle_pixels(&pixels, Rect::new(5.0, 4.0, 9.0, 10.0));
}
