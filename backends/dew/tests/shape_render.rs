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

use kurbo::{Affine, BezPath, Circle as KurboCircle, Rect, RoundedRect, Shape as _};
use peniko::Brush;
use waterui::prelude::*;
use waterui::shape::{Capsule, Circle, Path, RoundedRectangle, ShapeExt};
use waterui_dew::{Clip, ClipRegion, DisplayList, DrawCommand, render_view_png};

mod support;

const PATH_TOLERANCE: f64 = 0.05;

fn display_srgb(red: u8, green: u8, blue: u8) -> peniko::Color {
    let resolved = ResolvedColor::from_srgb(Srgb::new_u8(red, green, blue));
    let srgb = resolved.to_srgb_with_headroom();
    peniko::Color::new([srgb.red, srgb.green, srgb.blue, resolved.opacity])
}

fn render_scene<V: View>(build: impl Fn() -> V + 'static, width: u32, height: u32) -> DisplayList {
    let mut renderer = support::test_renderer();
    renderer.render_tree(
        AnyView::new(build()),
        &support::test_environment(),
        f64::from(width),
        f64::from(height),
    )
}

/// A root shape emits exactly the backend background followed by its fill.
fn content_fill(list: &DisplayList) -> (&BezPath, Affine, Option<&Clip>) {
    assert_eq!(
        list.commands().len(),
        2,
        "a root shape emits one background and one content fill"
    );
    match list.commands()[1].command() {
        DrawCommand::FillPath {
            path,
            transform,
            brush: Brush::Solid(color),
            clip,
        } => {
            assert_eq!(
                *color,
                display_srgb(255, 0, 0),
                "the content command keeps its exact fill"
            );
            (path, *transform, clip.as_ref())
        }
        command => panic!("expected a solid shape fill, got {command:?}"),
    }
}

fn assert_untransformed_path(actual: &BezPath, transform: Affine, expected: &BezPath) {
    assert_eq!(transform, Affine::IDENTITY);
    assert_eq!(actual, expected);
}

fn fill_red(shape: impl ShapeExt) -> impl View {
    shape.fill(Color::srgb(255, 0, 0))
}

/// A circle is inscribed in its bounds: on a 120×60 view it is a 60px-wide
/// disc in the middle, not an ellipse filling the box.
#[test]
fn a_circle_is_inscribed_rather_than_stretched() {
    let list = render_scene(|| fill_red(Circle), 120, 60);
    let (path, transform, clip) = content_fill(&list);
    assert!(clip.is_none());
    let expected = KurboCircle::new((60.0, 30.0), 30.0).to_path(PATH_TOLERANCE);
    assert_untransformed_path(path, transform, &expected);
}

/// A capsule's corners are circular arcs of the shorter side's half, so a
/// 120×60 capsule has a flat top edge from x = 30 to x = 90. Corners derived
/// from the per-axis command list would sweep to x = 60 and leave (35, 2)
/// outside the shape.
#[test]
fn capsule_corners_are_circular() {
    let list = render_scene(|| fill_red(Capsule), 120, 60);
    let (path, transform, clip) = content_fill(&list);
    assert!(clip.is_none());
    let expected =
        RoundedRect::from_rect(Rect::new(0.0, 0.0, 120.0, 60.0), 30.0).to_path(PATH_TOLERANCE);
    assert_untransformed_path(path, transform, &expected);
}

/// A rounded rectangle's radius is a fraction of the **shorter** side: 0.5 on
/// a 200×40 view is 20px, not 100px.
#[test]
fn rounded_rectangle_radius_follows_the_shorter_side() {
    let list = render_scene(|| fill_red(RoundedRectangle::new(0.5)), 200, 40);
    let (path, transform, clip) = content_fill(&list);
    assert!(clip.is_none());
    let expected =
        RoundedRect::from_rect(Rect::new(0.0, 0.0, 200.0, 40.0), 20.0).to_path(PATH_TOLERANCE);
    assert_untransformed_path(path, transform, &expected);
}

/// A clip resolves the same geometry as a fill. Clipping a filled rect to a
/// circle must mask exactly the pixels the circle would have painted.
#[test]
fn a_circle_clip_masks_what_the_circle_fill_paints() {
    let list = render_scene(|| Color::srgb(255, 0, 0).clip(Circle), 120, 60);
    let (_, transform, clip) = content_fill(&list);
    assert_eq!(transform, Affine::IDENTITY);
    let clip = clip.expect("the color fill retains the circle clip");
    let [ClipRegion::Shape { path, bounds }] = clip.regions() else {
        panic!("a circle clip is one retained shape region")
    };
    let expected = KurboCircle::new((60.0, 30.0), 30.0).to_path(PATH_TOLERANCE);
    assert_eq!(path.as_ref(), &expected);
    assert_eq!(*bounds, Rect::new(30.0, 0.0, 90.0, 60.0));
}

/// A capsule clip is a stadium, not a disc: the whole mid-height band survives.
#[test]
fn a_capsule_clip_keeps_the_middle_band() {
    let list = render_scene(|| Color::srgb(255, 0, 0).clip(Capsule), 120, 60);
    let (_, _, clip) = content_fill(&list);
    let clip = clip.expect("the color fill retains the capsule clip");
    let [ClipRegion::Shape { path, bounds }] = clip.regions() else {
        panic!("a capsule clip is one retained shape region")
    };
    let expected =
        RoundedRect::from_rect(Rect::new(0.0, 0.0, 120.0, 60.0), 30.0).to_path(PATH_TOLERANCE);
    assert_eq!(path.as_ref(), &expected);
    assert_eq!(*bounds, expected.bounding_box());
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
    let list = render_scene(move || fill_red(triangle.clone()), 120, 60);
    let (path, transform, clip) = content_fill(&list);
    assert!(clip.is_none());
    let mut expected = BezPath::new();
    expected.move_to((60.0, 0.0));
    expected.line_to((120.0, 60.0));
    expected.line_to((0.0, 60.0));
    expected.close_path();
    assert_untransformed_path(path, transform, &expected);
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
    std::fs::write(support::export_path("shapes", "review"), png)
        .expect("export visual review PNG");
}
