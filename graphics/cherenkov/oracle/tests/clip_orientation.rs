//! Analytic geometry checks through the complete reference renderer.

use std::path::Path;

use cherenkov_oracle::{F32Image, Renderer};
use cherenkov_scene::{Color, ColorSpace, FillRule, Scene, Shape};
use kurbo::{BezPath, Rect};

fn rectangle(rect: Rect, reversed: bool) -> BezPath {
    let mut corners = [
        (rect.x0, rect.y0),
        (rect.x1, rect.y0),
        (rect.x1, rect.y1),
        (rect.x0, rect.y1),
    ];
    if reversed {
        corners.reverse();
    }
    polygon(&corners)
}

fn polygon(points: &[(f64, f64)]) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for point in &points[1..] {
        path.line_to(*point);
    }
    path.close_path();
    path
}

fn render(draw: BezPath, rule: FillRule, clip: BezPath) -> F32Image {
    let scene = Scene::builder(16, 16)
        .clear(Color::new(ColorSpace::LinearP3, [0.0; 4]))
        .clip(Shape::Path { path: clip })
        .fill_rule(
            Shape::Path { path: draw },
            rule,
            Color::new(ColorSpace::LinearP3, [1.5, 0.1, 0.0, 1.0]).into(),
        )
        .build();
    Renderer::new(16, 16)
        .render(&scene, Path::new(env!("CARGO_MANIFEST_DIR")))
        .expect("reference render")
}

fn pixel_area(rect: Rect, x: f64, y: f64) -> f64 {
    (rect.x1.min(x + 1.0) - rect.x0.max(x)).max(0.0)
        * (rect.y1.min(y + 1.0) - rect.y0.max(y)).max(0.0)
}

fn assert_coverage(image: &F32Image, expected: impl Fn(f64, f64) -> f64) {
    for y in 0..16_u32 {
        for x in 0..16_u32 {
            let coverage = expected(f64::from(x), f64::from(y));
            let pixel = image.pixels[(y * 16 + x) as usize];
            for (actual, color) in pixel.into_iter().zip([1.5, 0.1, 0.0, 1.0]) {
                assert!(
                    actual.is_finite() && coverage.mul_add(-color, f64::from(actual)).abs() < 1e-5,
                    "pixel ({x},{y}): {pixel:?}, expected coverage {coverage}"
                );
            }
        }
    }
}

#[test]
fn rectangle_intersection_is_independent_of_each_contour_direction() {
    let draw = Rect::new(5.25, -5.0, 15.0, 5.75);
    let clip = Rect::new(0.0, 0.0, 10.75, 10.0);
    let intersection = Rect::new(5.25, 0.0, 10.75, 5.75);
    for draw_reversed in [false, true] {
        for clip_reversed in [false, true] {
            for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                let image = render(
                    rectangle(draw, draw_reversed),
                    rule,
                    rectangle(clip, clip_reversed),
                );
                assert_coverage(&image, |x, y| pixel_area(intersection, x, y));
            }
        }
    }
}

#[test]
fn coincident_and_touching_boundaries_do_not_change_the_filled_region() {
    let draw = Rect::new(2.0, 2.0, 8.0, 8.0);
    for clip in [draw, Rect::new(8.0, 2.0, 12.0, 8.0)] {
        for draw_reversed in [false, true] {
            for clip_reversed in [false, true] {
                let image = render(
                    rectangle(draw, draw_reversed),
                    FillRule::NonZero,
                    rectangle(clip, clip_reversed),
                );
                assert_coverage(&image, |x, y| pixel_area(draw.intersect(clip), x, y));
            }
        }
    }
}

#[test]
fn holes_and_duplicate_contours_keep_their_fill_rule() {
    let outer = Rect::new(1.0, 1.0, 13.0, 13.0);
    let inner = Rect::new(4.0, 4.0, 10.0, 10.0);
    let clip = Rect::new(2.0, 0.0, 12.0, 14.0);
    for reversed in [false, true] {
        for hole_reversed in [false, true] {
            let mut path = rectangle(outer, reversed);
            path.extend(rectangle(inner, hole_reversed).elements().iter().copied());
            for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                let image = render(path.clone(), rule, rectangle(clip, !reversed));
                let has_hole = reversed != hole_reversed || matches!(rule, FillRule::EvenOdd);
                assert_coverage(&image, |x, y| {
                    pixel_area(outer.intersect(clip), x, y)
                        - if has_hole {
                            pixel_area(inner.intersect(clip), x, y)
                        } else {
                            0.0
                        }
                });
            }
        }
        let mut duplicate = rectangle(outer, reversed);
        duplicate.extend(rectangle(outer, reversed).elements().iter().copied());
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            let image = render(duplicate.clone(), rule, rectangle(clip, !reversed));
            assert_coverage(&image, |x, y| {
                if matches!(rule, FillRule::EvenOdd) {
                    0.0
                } else {
                    pixel_area(outer.intersect(clip), x, y)
                }
            });
        }
    }
}

#[test]
fn opposite_disjoint_contours_are_not_normalized_by_total_signed_area() {
    let first = Rect::new(1.0, 2.0, 5.0, 8.0);
    let second = Rect::new(9.0, 2.0, 13.0, 8.0);
    let mut path = rectangle(first, false);
    path.extend(rectangle(second, true).elements().iter().copied());
    let image = render(
        path,
        FillRule::NonZero,
        rectangle(Rect::new(0.0, 0.0, 16.0, 16.0), true),
    );
    assert_coverage(&image, |x, y| {
        pixel_area(first, x, y) + pixel_area(second, x, y)
    });
}

#[test]
fn a_self_crossing_shape_is_split_where_its_own_edges_cross() {
    let path = polygon(&[(2.0, 2.0), (10.0, 10.0), (10.0, 2.0), (2.0, 10.0)]);
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let image = render(
            path.clone(),
            rule,
            rectangle(Rect::new(0.0, 0.0, 16.0, 16.0), true),
        );
        let area: f64 = image.pixels.iter().map(|p| f64::from(p[3])).sum();
        assert!((area - 32.0).abs() < 1e-6, "bow-tie area {area}");
        assert!((image.pixels[5 * 16 + 3][3] - 1.0).abs() < 1e-6);
        assert!((image.pixels[5 * 16 + 8][3] - 1.0).abs() < 1e-6);
        assert!(image.pixels[2 * 16 + 5][3].abs() < 1e-6);
    }
}
