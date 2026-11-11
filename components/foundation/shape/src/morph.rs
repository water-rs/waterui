//! Morphing between the built-in shapes, drawn through Canvas.
//!
//! Every built-in [`ShapeKind`] is one parameterized shape: a box, four
//! elliptical corner radii and a superellipse exponent that shapes every
//! corner (2 is a circular or elliptical arc). A morph interpolates those
//! parameters and fills the path they describe, as one live shape operand of
//! a [`Canvas`], so every backend draws the same silhouette at every
//! progress, in square and non-square boxes alike.

use core::time::Duration;

use nami::{Computed, SignalExt as _, signal::IntoComputed};
use waterui_core::easing::EasingCurve;
use waterui_core::layout::{Point, Size, StretchAxis};
use waterui_core::{Environment, View};
use waterui_graphics::canvas::{Canvas, Path, PathBuilder};
use waterui_graphics::color::Color;

use crate::ShapeKind;

/// The exponent of a circular or elliptical corner arc.
const ARC_EXPONENT: f32 = 2.0;

/// One built-in shape resolved against a box.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Silhouette {
    /// The box the shape's outline spans, in points: the whole bounds, or the
    /// centred square a circle inscribes.
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    /// Corner radii `(horizontal, vertical)` in points, top-left, top-right,
    /// bottom-right, bottom-left. Adjacent radii along an edge never sum past
    /// the edge, and the interpolation of two such shapes keeps that.
    corners: [(f32, f32); 4],
    /// The superellipse exponent every corner is cut with.
    exponent: f32,
}

impl Silhouette {
    /// `kind` resolved against a box of `size`, whose normalized radii are
    /// fractions of the shorter side and whose fixed radii are points.
    ///
    /// # Panics
    ///
    /// On [`ShapeKind::CustomPath`], which has no parameters.
    fn of(kind: ShapeKind, size: Size) -> Self {
        let (width, height) = (size.width.max(0.0), size.height.max(0.0));
        let shorter_edge = width.min(height);
        let half = shorter_edge / 2.0;
        let normalized = |radius: f32| radius.clamp(0.0, 0.5) * shorter_edge;
        let fixed = |radius: f32| radius.clamp(0.0, half);
        let round = |radii: [f32; 4]| radii.map(|radius| (radius, radius));
        let bounds = |corners| Self {
            left: 0.0,
            top: 0.0,
            right: width,
            bottom: height,
            corners,
            exponent: ARC_EXPONENT,
        };
        match kind {
            ShapeKind::Rect => bounds([(0.0, 0.0); 4]),
            ShapeKind::Circle => {
                let (left, top) = ((width - shorter_edge) / 2.0, (height - shorter_edge) / 2.0);
                Self {
                    left,
                    top,
                    right: left + shorter_edge,
                    bottom: top + shorter_edge,
                    corners: round([half; 4]),
                    exponent: ARC_EXPONENT,
                }
            }
            ShapeKind::Ellipse => bounds([(width / 2.0, height / 2.0); 4]),
            ShapeKind::Capsule => bounds(round([half; 4])),
            ShapeKind::RoundedRect { corner_radius } => {
                bounds(round([normalized(corner_radius); 4]))
            }
            ShapeKind::UnevenRoundedRect {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => bounds(round([
                normalized(top_left),
                normalized(top_right),
                normalized(bottom_right),
                normalized(bottom_left),
            ])),
            ShapeKind::FixedRoundedRect { corner_radius } => {
                bounds(round([fixed(corner_radius); 4]))
            }
            ShapeKind::FixedUnevenRoundedRect {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => bounds(round([
                fixed(top_left),
                fixed(top_right),
                fixed(bottom_right),
                fixed(bottom_left),
            ])),
            ShapeKind::CustomPath => {
                panic!("a morph interpolates built-in shapes; a custom path has no parameters")
            }
        }
    }

    /// The shape `t` of the way from `self` to `other`, parameter by
    /// parameter.
    fn lerp(self, other: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| (b - a).mul_add(t, a);
        let mut corners = self.corners;
        for (corner, target) in corners.iter_mut().zip(other.corners) {
            *corner = (mix(corner.0, target.0), mix(corner.1, target.1));
        }
        Self {
            left: mix(self.left, other.left),
            top: mix(self.top, other.top),
            right: mix(self.right, other.right),
            bottom: mix(self.bottom, other.bottom),
            corners,
            exponent: mix(self.exponent, other.exponent),
        }
    }

    /// The clockwise outline.
    fn path(&self) -> Path {
        let Self {
            left,
            top,
            right,
            bottom,
            corners: [tl, tr, br, bl],
            exponent,
        } = *self;
        // The cubic through a quarter's end points whose midpoint lies on the
        // superellipse: 0.5523 for an arc, towards 1 as the corner squares.
        let handle = 8.0_f32.mul_add((-1.0 / exponent).exp2(), -4.0) / 3.0;
        let corner = |builder: PathBuilder,
                      center: Point,
                      (rx, ry): (f32, f32),
                      outward: (f32, f32),
                      clockwise_from_edge: bool| {
            // The quarter from the point on the horizontal edge to the point
            // on the vertical one, as a cubic in the corner's own axes.
            let at = |u: f32, v: f32| {
                Point::new(
                    (outward.0 * rx).mul_add(u, center.x),
                    (outward.1 * ry).mul_add(v, center.y),
                )
            };
            let (edge, control_edge, control_side, vertical_end) =
                (at(0.0, 1.0), at(handle, 1.0), at(1.0, handle), at(1.0, 0.0));
            if clockwise_from_edge {
                builder.cubic_to(control_edge, control_side, vertical_end)
            } else {
                builder.cubic_to(control_side, control_edge, edge)
            }
        };
        let top_left_center = Point::new(left + tl.0, top + tl.1);
        let top_right_center = Point::new(right - tr.0, top + tr.1);
        let bottom_right_center = Point::new(right - br.0, bottom - br.1);
        let bottom_left_center = Point::new(left + bl.0, bottom - bl.1);
        let builder = Path::builder()
            .move_to(Point::new(top_left_center.x, top))
            .line_to(Point::new(top_right_center.x, top));
        let builder = corner(builder, top_right_center, tr, (1.0, -1.0), true)
            .line_to(Point::new(right, bottom_right_center.y));
        let builder = corner(builder, bottom_right_center, br, (1.0, 1.0), false)
            .line_to(Point::new(bottom_left_center.x, bottom));
        let builder = corner(builder, bottom_left_center, bl, (-1.0, 1.0), true)
            .line_to(Point::new(left, top_left_center.y));
        corner(builder, top_left_center, tl, (-1.0, -1.0), false)
            .close()
            .build()
    }
}

/// Configuration for shape morph animations.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct MorphAnimation {
    /// Duration of one forward morph cycle.
    duration: Duration,
    /// Easing curve applied to normalized cycle progress.
    easing: EasingCurve,
    /// Whether the animation repeats after reaching the end.
    repeat: bool,
    /// Whether repeating animation should play in reverse every other cycle.
    autoreverse: bool,
}

impl MorphAnimation {
    /// Duration of one forward morph cycle.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    /// Easing curve applied to normalized cycle progress.
    #[must_use]
    pub const fn easing(&self) -> EasingCurve {
        self.easing
    }

    /// Whether the animation repeats after reaching the end.
    #[must_use]
    pub const fn repeat(&self) -> bool {
        self.repeat
    }

    /// Whether repeating animation should play in reverse every other cycle.
    #[must_use]
    pub const fn autoreverse(&self) -> bool {
        self.autoreverse
    }
}

impl Default for MorphAnimation {
    fn default() -> Self {
        Self {
            duration: Duration::from_millis(900),
            easing: EasingCurve::EASE_IN_OUT,
            repeat: true,
            autoreverse: true,
        }
    }
}

impl MorphAnimation {
    /// Creates a one-shot morph animation.
    #[must_use]
    pub const fn once(duration: Duration, easing: EasingCurve) -> Self {
        Self {
            duration,
            easing,
            repeat: false,
            autoreverse: false,
        }
    }

    /// The eased progress `elapsed` after the animation started.
    fn sample(self, elapsed: Duration) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let raw = elapsed.as_secs_f32() / self.duration.as_secs_f32();
        let cycle = if self.repeat {
            let base = raw.fract();
            // The cycle index is a whole number: odd cycles run backwards.
            if self.autoreverse && raw.floor().rem_euclid(2.0) > 0.5 {
                1.0 - base
            } else {
                base
            }
        } else {
            raw.clamp(0.0, 1.0)
        };
        self.easing.ease(cycle).clamp(0.0, 1.0)
    }
}

/// A morphing filled shape view.
///
/// Its progress follows an explicit signal, or the canvas clock through the
/// [`MorphAnimation`]. The silhouette is one live path operand of a
/// [`Canvas`]; it re-derives from the box and the progress and nothing else
/// records again.
#[derive(Debug, Clone)]
pub struct MorphShape {
    from: ShapeKind,
    to: ShapeKind,
    fill: Color,
    animation: MorphAnimation,
    progress: Option<Computed<f32>>,
}

impl MorphShape {
    /// A morph from `from` to `to`.
    ///
    /// # Panics
    ///
    /// When either shape is [`ShapeKind::CustomPath`]: a path has no
    /// parameters to interpolate.
    pub(crate) fn new(from: ShapeKind, to: ShapeKind, fill: Color) -> Self {
        for kind in [from, to] {
            assert!(
                kind != ShapeKind::CustomPath,
                "a morph interpolates built-in shapes; a custom path has no parameters"
            );
        }
        Self {
            from,
            to,
            fill,
            animation: MorphAnimation::default(),
            progress: None,
        }
    }

    /// Sets explicit animation configuration.
    #[must_use]
    pub const fn animation(mut self, animation: MorphAnimation) -> Self {
        self.animation = animation;
        self
    }

    /// Sets the cycle duration (keeps other animation options unchanged).
    #[must_use]
    pub const fn duration(mut self, duration: Duration) -> Self {
        self.animation.duration = duration;
        self
    }

    /// Sets easing (keeps other animation options unchanged).
    #[must_use]
    pub const fn easing(mut self, easing: EasingCurve) -> Self {
        self.animation.easing = easing;
        self
    }

    /// Enables/disables repeating.
    #[must_use]
    pub const fn repeat(mut self, repeat: bool) -> Self {
        self.animation.repeat = repeat;
        self
    }

    /// Enables/disables autoreverse for repeating animations.
    #[must_use]
    pub const fn autoreverse(mut self, autoreverse: bool) -> Self {
        self.animation.autoreverse = autoreverse;
        self
    }

    /// Overrides animated progress with an explicit reactive progress signal `[0, 1]`.
    ///
    /// When set, this takes precedence over the time-based animation config.
    #[must_use]
    pub fn progress(mut self, progress: impl IntoComputed<f32>) -> Self {
        self.progress = Some(progress.into_computed());
        self
    }
}

impl View for MorphShape {
    fn body(self, _env: &Environment) -> impl View {
        let Self {
            from,
            to,
            fill,
            animation,
            progress,
        } = self;
        Canvas::new(move |cx| {
            let progress = progress.unwrap_or_else(|| {
                // A one-shot morph's clock holds at its end, so the canvas
                // stops presenting frames once it has played.
                let clock = if animation.repeat {
                    cx.time()
                } else {
                    cx.time_until(animation.duration)
                };
                clock
                    .map(move |elapsed| animation.sample(elapsed))
                    .computed()
            });
            let outline = cx
                .size()
                .zip(&progress)
                .map(move |(size, progress)| {
                    Silhouette::of(from, size)
                        .lerp(Silhouette::of(to, size), progress.clamp(0.0, 1.0))
                        .path()
                })
                .computed();
            cx.fill(outline, fill);
        })
    }

    /// A canvas without an intrinsic size fills both axes.
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

#[cfg(test)]
mod tests {
    use waterui_core::layout::Rect;

    use super::*;

    const WIDE: Size = Size::new(200.0, 100.0);

    fn bounds(path: &Path) -> Rect {
        path.bounds()
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.5,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn a_circle_is_the_inscribed_square_not_a_capsule() {
        let path = Silhouette::of(ShapeKind::Circle, WIDE).path();
        let rect = bounds(&path);
        assert_close(rect.x(), 50.0);
        assert_close(rect.width(), 100.0);
        assert_close(rect.height(), 100.0);
        assert!(path.contains(Point::new(100.0, 50.0)));
        assert!(!path.contains(Point::new(55.0, 5.0)), "no square corner");
        assert!(path.contains(Point::new(145.0, 50.0)));
        assert!(
            !path.contains(Point::new(170.0, 50.0)),
            "the circle does not stretch to the box's width"
        );
    }

    #[test]
    fn an_ellipse_fills_the_box_with_elliptical_corners() {
        let path = Silhouette::of(ShapeKind::Ellipse, WIDE).path();
        let rect = bounds(&path);
        assert_close(rect.width(), 200.0);
        assert_close(rect.height(), 100.0);
        // On the ellipse's diagonal, scaled about its centre.
        let on = |scale: f32| {
            Point::new(
                (100.0 * scale).mul_add(core::f32::consts::FRAC_1_SQRT_2, 100.0),
                (50.0 * scale).mul_add(core::f32::consts::FRAC_1_SQRT_2, 50.0),
            )
        };
        assert!(path.contains(on(0.98)));
        assert!(!path.contains(on(1.05)));
    }

    #[test]
    fn a_capsule_is_not_an_ellipse() {
        let capsule = Silhouette::of(ShapeKind::Capsule, WIDE).path();
        // The capsule's flat top runs well past an ellipse's.
        assert!(capsule.contains(Point::new(150.0, 2.0)));
        let ellipse = Silhouette::of(ShapeKind::Ellipse, WIDE).path();
        assert!(!ellipse.contains(Point::new(150.0, 2.0)));
    }

    #[test]
    fn a_morph_interpolates_the_box_and_the_radii() {
        let from = Silhouette::of(ShapeKind::Rect, WIDE);
        let to = Silhouette::of(ShapeKind::Circle, WIDE);
        let half = from.lerp(to, 0.5);
        assert_close(half.left, 25.0);
        assert_close(half.right, 175.0);
        assert_close(half.corners[0].0, 25.0);
        assert_eq!(from.lerp(to, 0.0), from);
        assert_eq!(from.lerp(to, 1.0), to);
    }

    #[test]
    fn a_larger_exponent_squares_the_corner() {
        let arc = Silhouette::of(ShapeKind::Circle, Size::new(100.0, 100.0));
        let squircle = Silhouette {
            exponent: 4.0,
            ..arc
        };
        let corner = Point::new(8.0, 8.0);
        assert!(!arc.path().contains(corner));
        assert!(squircle.path().contains(corner));
    }

    #[test]
    #[should_panic(expected = "custom path has no parameters")]
    fn a_custom_path_cannot_morph() {
        let _ = MorphShape::new(ShapeKind::CustomPath, ShapeKind::Rect, Color::srgb(0, 0, 0));
    }

    #[test]
    fn one_shot_animation_reaches_end() {
        let animation = MorphAnimation::once(Duration::from_millis(200), EasingCurve::LINEAR);
        assert!((animation.sample(Duration::ZERO) - 0.0).abs() < 1e-6);
        assert!((animation.sample(Duration::from_millis(100)) - 0.5).abs() < 1e-3);
        assert!((animation.sample(Duration::from_secs(1)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_repeating_animation_reverses_every_other_cycle() {
        let animation = MorphAnimation {
            easing: EasingCurve::LINEAR,
            ..MorphAnimation::default()
        };
        let cycle = animation.duration();
        assert!((animation.sample(cycle.mul_f32(0.25)) - 0.25).abs() < 1e-3);
        assert!((animation.sample(cycle.mul_f32(1.25)) - 0.75).abs() < 1e-3);
        assert!((animation.sample(cycle.mul_f32(2.25)) - 0.25).abs() < 1e-3);
    }
}
