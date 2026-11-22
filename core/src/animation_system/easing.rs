//! Easing curves and the lane decomposition `WaterUI` animations move values
//! through.
//!
//! Every curve and spring evaluates through `waterui-easing`, the evaluator
//! the Cherenkov engine shares, so an animation reaches the same value at the
//! same elapsed time whether core or the engine samples it. Its types are
//! re-exported here: [`CubicBezier`] and [`Spring`] are the evaluated forms
//! of a timing curve and a physical spring, and [`Timing`] is how a track
//! moves.
//!
//! [`EasingCurve`] is the authored timing curve: a cubic Bézier over
//! normalized time, with the CSS-standard curves as constants. A spring is
//! not a curve over normalized time — it runs in real seconds until it
//! settles — so it is an [`Animation`](crate::animation::Animation), never an
//! easing curve.

use core::time::Duration;

use smallvec::SmallVec;
pub use waterui_easing::{CubicBezier, SETTLE_TOLERANCE, Spring, Timing};

/// A cubic Bézier timing curve from `(0, 0)` to `(1, 1)` with the control
/// points `(x1, y1)` and `(x2, y2)`.
///
/// It maps normalized time to progress. The standard CSS curves are
/// constants; [`bezier`](Self::bezier) builds any other.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EasingCurve {
    /// The first control point's time coordinate, in `[0, 1]`.
    pub x1: f32,
    /// The first control point's progress coordinate.
    pub y1: f32,
    /// The second control point's time coordinate, in `[0, 1]`.
    pub x2: f32,
    /// The second control point's progress coordinate.
    pub y2: f32,
}

impl EasingCurve {
    /// Linear interpolation - constant velocity from start to finish.
    /// CSS: `linear` or `cubic-bezier(0, 0, 1, 1)`
    pub const LINEAR: Self = Self::bezier(0.0, 0.0, 1.0, 1.0);

    /// Ease-in - starts slow and accelerates.
    /// CSS: `ease-in` or `cubic-bezier(0.42, 0, 1, 1)`
    pub const EASE_IN: Self = Self::bezier(0.42, 0.0, 1.0, 1.0);

    /// Ease-out - starts fast and decelerates.
    /// CSS: `ease-out` or `cubic-bezier(0, 0, 0.58, 1)`
    pub const EASE_OUT: Self = Self::bezier(0.0, 0.0, 0.58, 1.0);

    /// Ease-in-out - starts slow, speeds up, then slows down.
    /// CSS: `ease-in-out` or `cubic-bezier(0.42, 0, 0.58, 1)`
    pub const EASE_IN_OUT: Self = Self::bezier(0.42, 0.0, 0.58, 1.0);

    /// Default ease curve (CSS `ease`).
    /// CSS: `ease` or `cubic-bezier(0.25, 0.1, 0.25, 1)`
    pub const EASE: Self = Self::bezier(0.25, 0.1, 0.25, 1.0);

    /// Creates a custom cubic bezier curve.
    ///
    /// Every control point must be finite and `x1` and `x2` must lie in
    /// `[0, 1]`. A debug build asserts this; a consumer calls
    /// [`validated`](Self::validated) to turn an invalid curve into the
    /// default one.
    #[must_use]
    pub const fn bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        debug_assert!(
            bezier_is_valid(x1, y1, x2, y2),
            "EasingCurve::bezier requires finite control points with x1/x2 in [0, 1]"
        );
        Self { x1, y1, x2, y2 }
    }

    /// This curve when its control points are valid, otherwise the default
    /// curve.
    ///
    /// A consumer calls it where it takes an `EasingCurve` from app code. An
    /// invalid curve is a contract violation: a debug build panics and a
    /// release build logs it and uses [`EasingCurve::default`].
    #[must_use]
    pub fn validated(self) -> Self {
        if bezier_is_valid(self.x1, self.y1, self.x2, self.y2) {
            self
        } else {
            crate::contract_violation!("invalid easing curve parameters: {self:?}");
            Self::default()
        }
    }

    /// The curve in the shared evaluator's terms.
    #[must_use]
    pub fn cubic_bezier(&self) -> CubicBezier {
        cubic_bezier(self.x1, self.y1, self.x2, self.y2)
    }

    /// The progress at normalized time `t`: `0` at and before the start,
    /// `1` at and after the end.
    #[must_use]
    pub fn ease(&self, t: f64) -> f64 {
        self.cubic_bezier().value(t)
    }
}

impl Default for EasingCurve {
    fn default() -> Self {
        Self::EASE_IN_OUT
    }
}

/// The shared evaluator's curve for `f32` control points.
pub(super) fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> CubicBezier {
    CubicBezier::new(f64::from(x1), f64::from(y1), f64::from(x2), f64::from(y2))
}

/// Whether bezier control points are finite with `x1` and `x2` in `[0, 1]`:
/// [`CubicBezier::is_valid`] for `f32` control points; the casts widen
/// exactly, and `f64::from` is not callable in a const fn.
pub(super) const fn bezier_is_valid(x1: f32, y1: f32, x2: f32, y2: f32) -> bool {
    CubicBezier::new(x1 as f64, y1 as f64, x2 as f64, y2 as f64).is_valid()
}

/// The lanes a value moves through: inline for values of up to eight
/// scalars.
pub(super) type Lanes = SmallVec<[f64; 8]>;

/// A value decomposed into scalar `f64` lanes, so an animation can move it
/// lane by lane through the shared evaluator.
///
/// The animation system evaluates a curve or a spring on each lane: a curve
/// interpolates every lane by the same progress, and a spring carries each
/// lane's velocity into a retarget. Numbers, tuples and arrays of
/// decomposable values implement it.
pub trait Interpolatable: Clone {
    /// How many lanes the value decomposes into.
    const LANES: usize;

    /// Writes the value's lanes into `lanes`, which holds exactly
    /// [`LANES`](Self::LANES) of them.
    fn write_lanes(&self, lanes: &mut [f64]);

    /// The value whose lanes are `lanes`, as
    /// [`write_lanes`](Self::write_lanes) writes them.
    fn from_lanes(lanes: &[f64]) -> Self;

    /// Linear interpolation, lane by lane: `self + (other - self) * t`.
    #[must_use]
    fn lerp(&self, other: &Self, t: f64) -> Self {
        let mut from = lanes_of(self);
        let to = lanes_of(other);
        for (lane, target) in from.iter_mut().zip(&to) {
            *lane = (target - *lane).mul_add(t, *lane);
        }
        Self::from_lanes(&from)
    }
}

/// `value`'s lanes.
pub(super) fn lanes_of<T: Interpolatable>(value: &T) -> Lanes {
    let mut lanes = Lanes::from_elem(0.0, T::LANES);
    value.write_lanes(&mut lanes);
    lanes
}

impl Interpolatable for f32 {
    const LANES: usize = 1;

    fn write_lanes(&self, lanes: &mut [f64]) {
        lanes[0] = f64::from(*self);
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "an f32 value's lane narrows back to f32"
    )]
    fn from_lanes(lanes: &[f64]) -> Self {
        lanes[0] as Self
    }
}

impl Interpolatable for f64 {
    const LANES: usize = 1;

    fn write_lanes(&self, lanes: &mut [f64]) {
        lanes[0] = *self;
    }

    fn from_lanes(lanes: &[f64]) -> Self {
        lanes[0]
    }
}

impl<A: Interpolatable, B: Interpolatable> Interpolatable for (A, B) {
    const LANES: usize = A::LANES + B::LANES;

    fn write_lanes(&self, lanes: &mut [f64]) {
        let (a, b) = lanes.split_at_mut(A::LANES);
        self.0.write_lanes(a);
        self.1.write_lanes(b);
    }

    fn from_lanes(lanes: &[f64]) -> Self {
        let (a, b) = lanes.split_at(A::LANES);
        (A::from_lanes(a), B::from_lanes(b))
    }
}

impl<A: Interpolatable, B: Interpolatable, C: Interpolatable> Interpolatable for (A, B, C) {
    const LANES: usize = A::LANES + B::LANES + C::LANES;

    fn write_lanes(&self, lanes: &mut [f64]) {
        let (a, rest) = lanes.split_at_mut(A::LANES);
        let (b, c) = rest.split_at_mut(B::LANES);
        self.0.write_lanes(a);
        self.1.write_lanes(b);
        self.2.write_lanes(c);
    }

    fn from_lanes(lanes: &[f64]) -> Self {
        let (a, rest) = lanes.split_at(A::LANES);
        let (b, c) = rest.split_at(B::LANES);
        (A::from_lanes(a), B::from_lanes(b), C::from_lanes(c))
    }
}

impl<A: Interpolatable, B: Interpolatable, C: Interpolatable, D: Interpolatable> Interpolatable
    for (A, B, C, D)
{
    const LANES: usize = A::LANES + B::LANES + C::LANES + D::LANES;

    fn write_lanes(&self, lanes: &mut [f64]) {
        let (a, rest) = lanes.split_at_mut(A::LANES);
        let (b, rest) = rest.split_at_mut(B::LANES);
        let (c, d) = rest.split_at_mut(C::LANES);
        self.0.write_lanes(a);
        self.1.write_lanes(b);
        self.2.write_lanes(c);
        self.3.write_lanes(d);
    }

    fn from_lanes(lanes: &[f64]) -> Self {
        let (a, rest) = lanes.split_at(A::LANES);
        let (b, rest) = rest.split_at(B::LANES);
        let (c, d) = rest.split_at(C::LANES);
        (
            A::from_lanes(a),
            B::from_lanes(b),
            C::from_lanes(c),
            D::from_lanes(d),
        )
    }
}

impl<T: Interpolatable, const N: usize> Interpolatable for [T; N] {
    const LANES: usize = N * T::LANES;

    fn write_lanes(&self, lanes: &mut [f64]) {
        for (value, chunk) in self.iter().zip(lanes.chunks_exact_mut(T::LANES)) {
            value.write_lanes(chunk);
        }
    }

    fn from_lanes(lanes: &[f64]) -> Self {
        core::array::from_fn(|index| {
            T::from_lanes(&lanes[index * T::LANES..(index + 1) * T::LANES])
        })
    }
}

/// Animation segment with duration and easing curve.
///
/// Used to build multi-segment animations where different parts of the
/// animation can have different easing curves.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AnimationSegment {
    /// Duration of this segment.
    pub duration: Duration,
    /// Easing curve for this segment.
    pub curve: EasingCurve,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_easing() {
        let curve = EasingCurve::LINEAR;
        assert!(curve.ease(0.0).abs() < 1e-12);
        assert!((curve.ease(0.5) - 0.5).abs() < 1e-12);
        assert!((curve.ease(1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn test_ease_in() {
        let curve = EasingCurve::EASE_IN;
        assert!(curve.ease(0.0).abs() < 1e-12);
        assert!((curve.ease(1.0) - 1.0).abs() < 1e-12);
        // Ease-in should be slower at the start
        assert!(curve.ease(0.5) < 0.5);
    }

    #[test]
    fn test_ease_out() {
        let curve = EasingCurve::EASE_OUT;
        assert!(curve.ease(0.0).abs() < 1e-12);
        assert!((curve.ease(1.0) - 1.0).abs() < 1e-12);
        // Ease-out should be faster at the start
        assert!(curve.ease(0.5) > 0.5);
    }

    #[test]
    fn test_ease_in_out() {
        let curve = EasingCurve::EASE_IN_OUT;
        assert!(curve.ease(0.0).abs() < 1e-12);
        assert!((curve.ease(1.0) - 1.0).abs() < 1e-12);
        // The curve is symmetric about its midpoint.
        assert!((curve.ease(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn ease_is_the_shared_evaluator() {
        let curve = EasingCurve::bezier(0.2, 0.0, 0.0, 1.0);
        let shared = CubicBezier::new(f64::from(0.2_f32), 0.0, 0.0, 1.0);
        for step in 0_u8..=20 {
            let t = f64::from(step) / 20.0;
            assert_eq!(curve.ease(t).to_bits(), shared.value(t).to_bits(), "at {t}");
        }
    }

    #[test]
    fn validated_keeps_valid_curves() {
        assert_eq!(EasingCurve::EASE.validated(), EasingCurve::EASE);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "invalid easing curve parameters")]
    fn validated_reports_an_invalid_curve_in_debug() {
        let _ = EasingCurve {
            x1: 1.5,
            y1: 0.0,
            x2: 0.5,
            y2: 1.0,
        }
        .validated();
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn validated_replaces_an_invalid_curve_with_the_default_in_release() {
        let invalid = EasingCurve {
            x1: f32::NAN,
            y1: 0.0,
            x2: 0.5,
            y2: 1.0,
        };
        assert_eq!(invalid.validated(), EasingCurve::default());
    }

    #[test]
    fn test_bezier_solver_handles_extreme_control_points() {
        let curve = EasingCurve::bezier(0.0, 1.0, 1.0, 0.0);
        for step in 0_u16..=100 {
            let t = f64::from(step) / 100.0;
            let eased = curve.ease(t);
            assert!(eased.is_finite(), "eased must be finite at t={t}");
            assert!(
                (0.0..=1.0).contains(&eased),
                "eased out of range at t={t}: {eased}"
            );
        }
    }

    #[test]
    fn test_f32_lerp() {
        let a = 0.0_f32;
        let b = 10.0_f32;
        assert!((a.lerp(&b, 0.0) - 0.0).abs() < 0.001);
        assert!((a.lerp(&b, 0.5) - 5.0).abs() < 0.001);
        assert!((a.lerp(&b, 1.0) - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_tuple_lerp() {
        let a = (0.0_f32, 0.0_f32);
        let b = (10.0_f32, 20.0_f32);
        let mid = a.lerp(&b, 0.5);
        assert!((mid.0 - 5.0).abs() < 0.001);
        assert!((mid.1 - 10.0).abs() < 0.001);
    }

    #[test]
    fn nested_values_round_trip_through_their_lanes() {
        let value = ([1.5_f32, -2.0], (3.25_f64, 4.0_f32, [5.0_f64; 2]));
        assert_eq!(
            <([f32; 2], (f64, f32, [f64; 2]))>::LANES,
            6,
            "two, then one, one and two lanes"
        );
        let lanes = lanes_of(&value);
        assert_eq!(&lanes[..], &[1.5, -2.0, 3.25, 4.0, 5.0, 5.0]);
        assert_eq!(
            <([f32; 2], (f64, f32, [f64; 2]))>::from_lanes(&lanes),
            value
        );
    }
}
