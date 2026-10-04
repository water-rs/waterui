use kurbo::{Affine, Circle, Ellipse, Rect, RoundedRect, RoundedRectRadii};
use objc2_quartz_core::CACornerMask;

use cherenkov::{ContinuousRect, ShapeData};

use super::{LayerClip, LayerPlanes};
use crate::render::planes::Compositor;

fn rect() -> Rect {
    Rect::new(10.0, 20.0, 110.0, 70.0)
}

#[test]
fn motionless_placement_needs_no_animation_transaction() {
    let mut state = super::MotionState::default();
    assert!(!state.update(&[]));
    for _ in 0..3 {
        state.placed(false);
        assert!(!state.update(&[]));
    }
}

#[test]
fn a_rebuilt_motion_tree_reinstalls_unchanged_tracks() {
    let mut state = super::MotionState::default();
    assert!(!state.update(&[]));
    state.placed(true);
    assert!(state.update(&[]));
    state.placed(false);
    assert!(!state.update(&[]));
}

#[test]
fn a_rect_is_a_layer_clip() {
    assert_eq!(
        LayerClip::of(&ShapeData::Rect(rect())),
        Some(LayerClip {
            rect: rect(),
            radius: 0.0,
            corners: CACornerMask::empty(),
            continuous: false,
        })
    );
}

/// A layer rounds the corners it masks with one radius: square corners mix
/// with rounded ones, two radii do not.
#[test]
fn rounded_corners_share_one_radius_or_are_square() {
    let mixed = RoundedRect::from_rect(rect(), RoundedRectRadii::new(8.0, 0.0, 8.0, 0.0));
    assert_eq!(
        LayerClip::of(&ShapeData::RoundedRect(mixed)),
        Some(LayerClip {
            rect: rect(),
            radius: 8.0,
            corners: CACornerMask::LayerMinXMinYCorner | CACornerMask::LayerMaxXMaxYCorner,
            continuous: false,
        })
    );
    let two = RoundedRect::from_rect(rect(), RoundedRectRadii::new(8.0, 4.0, 8.0, 4.0));
    assert_eq!(LayerClip::of(&ShapeData::RoundedRect(two)), None);
}

#[test]
fn a_radius_beyond_half_the_short_side_is_not_a_layer_clip() {
    let fits = RoundedRect::from_rect(rect(), 25.0);
    assert!(LayerClip::of(&ShapeData::RoundedRect(fits)).is_some());
    let over = ContinuousRect {
        rect: rect(),
        radii: RoundedRectRadii::from_single_radius(26.0),
        smoothing: 0.0,
    };
    assert_eq!(LayerClip::of(&ShapeData::Continuous(over)), None);
}

#[test]
fn circles_and_round_ellipses_are_layer_clips_ovals_are_not() {
    let circle = LayerClip::of(&ShapeData::Circle(Circle::new((50.0, 50.0), 10.0)))
        .expect("a circle is a fully rounded square");
    assert_eq!(circle.rect, Rect::new(40.0, 40.0, 60.0, 60.0));
    assert!((circle.radius - 10.0).abs() < f64::EPSILON);
    let round = Ellipse::new((50.0, 50.0), (10.0, 10.0), 0.7);
    let round = LayerClip::of(&ShapeData::Ellipse(round)).expect("a rotated circle");
    assert!((round.radius - 10.0).abs() < 1e-9);
    let oval = Ellipse::new((50.0, 50.0), (10.0, 6.0), 0.0);
    assert_eq!(LayerClip::of(&ShapeData::Ellipse(oval)), None);
}

/// Continuous corners are expressible at the system's own smoothing
/// (`cornerCurve = continuous`) or at zero (circular), nowhere else.
#[test]
fn continuous_corners_need_the_system_smoothing() {
    let at = |smoothing| {
        LayerClip::of(&ShapeData::Continuous(ContinuousRect {
            rect: rect(),
            radii: RoundedRectRadii::from_single_radius(12.0),
            smoothing,
        }))
    };
    assert!(at(ContinuousRect::DEFAULT_SMOOTHING).is_some_and(|c| c.continuous));
    assert!(at(0.0).is_some_and(|c| !c.continuous));
    assert_eq!(at(0.3), None);
}

#[test]
fn paths_are_not_layer_clips() {
    let path = ShapeData::Path {
        elements: vec![
            kurbo::PathEl::MoveTo((0.0, 0.0).into()),
            kurbo::PathEl::ClosePath,
        ]
        .into(),
        rule: cherenkov::FillRule::NonZero,
    };
    assert_eq!(LayerClip::of(&path), None);
    assert!(!LayerPlanes::expresses_clip(&path));
}

#[test]
fn every_finite_affine_is_expressible() {
    assert!(LayerPlanes::expresses_transform(
        Affine::rotate(0.4) * Affine::skew(0.2, 0.0)
    ));
    assert!(!LayerPlanes::expresses_transform(Affine::scale(f64::NAN)));
}
