//! The timing math every `WaterUI` animation evaluates through: the cubic
//! Bézier timing curve and the physical spring.
//!
//! `waterui-core` and the Cherenkov engine (`cherenkov-record`) both sample
//! curves and springs here, and neither depends on the other, so an
//! animation reaches the same value at the same elapsed time whichever side
//! evaluates it. Every quantity is an `f64` and time is in seconds.
//!
//! A track moves a value, decomposed into scalar lanes, from where it was to
//! a target. [`Timing::sample_lanes`] evaluates one `t` seconds in:
//!
//! ```
//! use waterui_easing::{Spring, Timing};
//!
//! let timing = Timing::Spring(Spring::from_physics(100.0, 10.0));
//! let (mut position, mut velocity) = ([0.0; 2], [0.0; 2]);
//! // From (0, 0) at rest towards (100, -40), a quarter second in.
//! let done = timing.sample_lanes(
//!     &[0.0, 0.0],
//!     &[0.0, 0.0],
//!     &[100.0, -40.0],
//!     0.25,
//!     &mut position,
//!     &mut velocity,
//! );
//! assert!(!done);
//! assert!(position[0] > 50.0 && position[1] < -20.0);
//! ```
//!
//! The crate has no dependencies. It needs `std` for the exponential and
//! trigonometric functions the spring solution uses.

use core::f64::consts::TAU;
use core::time::Duration;

/// The distance from rest, and the speed per second, within which a spring
/// has settled, in the units of the lane it moves.
///
/// Every evaluator decides settling by this one value: a spring track ends,
/// reporting its target exactly, once every lane stays this close to rest
/// ([`Spring::settling_duration`]). It is the threshold the Cherenkov engine
/// has always settled its springs at.
pub const SETTLE_TOLERANCE: f64 = 1e-3;

/// How close, in normalized time, the Bézier solve brings `x(u)` to the
/// time it inverts. A millisecond-scale animation resolves time to about
/// `1e-6`; the solve stays three orders below that.
const SOLVE_TOLERANCE: f64 = 1e-9;

/// The most steps the Bézier solve takes. Each step at least halves the
/// bracket around the solution, so 64 steps narrow `[0, 1]` below `f64`
/// resolution whatever the control points.
const SOLVE_STEPS: u32 = 64;

/// Below this `dx/du`, the curve's tangent is treated as vertical: it has no
/// finite rate, and the slope is reported as zero rather than an unbounded
/// velocity a retarget would inherit.
const VERTICAL_TANGENT: f64 = 1e-9;

/// The last step, in seconds, the critically damped settling solve takes:
/// a nanosecond, the resolution of a `Duration`.
const SETTLE_TIME_TOLERANCE: f64 = 1e-9;

/// How far a damping ratio may sit from one and still take the critically
/// damped solution: the under- and overdamped solutions divide by
/// `√|ζ² − 1|`, which loses all precision as the ratio approaches one.
const CRITICAL_BAND: f64 = 1e-9;

/// A cubic Bézier timing curve from `(0, 0)` to `(1, 1)` with the control
/// points `(x1, y1)` and `(x2, y2)`, as CSS `cubic-bezier()` and Core
/// Animation's `CAMediaTimingFunction` define it.
///
/// The curve maps normalized time `x` to progress `y`. It is a function of
/// time only when `x1` and `x2` lie in `[0, 1]`; [`is_valid`](Self::is_valid)
/// states that contract.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier {
    /// The first control point's time coordinate, in `[0, 1]`.
    pub x1: f64,
    /// The first control point's progress coordinate.
    pub y1: f64,
    /// The second control point's time coordinate, in `[0, 1]`.
    pub x2: f64,
    /// The second control point's progress coordinate.
    pub y2: f64,
}

impl CubicBezier {
    /// Constant progress: CSS `linear`.
    pub const LINEAR: Self = Self::new(0.0, 0.0, 1.0, 1.0);
    /// CSS `ease`: `cubic-bezier(0.25, 0.1, 0.25, 1)`.
    pub const EASE: Self = Self::new(0.25, 0.1, 0.25, 1.0);
    /// CSS `ease-in`: `cubic-bezier(0.42, 0, 1, 1)`.
    pub const EASE_IN: Self = Self::new(0.42, 0.0, 1.0, 1.0);
    /// CSS `ease-out`: `cubic-bezier(0, 0, 0.58, 1)`.
    pub const EASE_OUT: Self = Self::new(0.0, 0.0, 0.58, 1.0);
    /// CSS `ease-in-out`: `cubic-bezier(0.42, 0, 0.58, 1)`.
    pub const EASE_IN_OUT: Self = Self::new(0.42, 0.0, 0.58, 1.0);

    /// The curve with control points `(x1, y1)` and `(x2, y2)`.
    #[must_use]
    pub const fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// Whether every control point is finite and `x1` and `x2` lie in
    /// `[0, 1]`, so the curve is a function of time.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.x1.is_finite()
            && self.y1.is_finite()
            && self.x2.is_finite()
            && self.y2.is_finite()
            && self.x1 >= 0.0
            && self.x1 <= 1.0
            && self.x2 >= 0.0
            && self.x2 <= 1.0
    }

    /// The progress at normalized time `x` and its rate `dy/dx`.
    ///
    /// Time before the start is progress `0`, time after the end progress
    /// `1`, and the rate at and beyond either end is `0`.
    #[must_use]
    pub fn sample(&self, x: f64) -> (f64, f64) {
        if x <= 0.0 {
            return (0.0, 0.0);
        }
        if x >= 1.0 {
            return (1.0, 0.0);
        }
        // `y(u) = x(u)` for every `u`: the curve is the identity.
        #[expect(
            clippy::float_cmp,
            reason = "only exactly equal control points make the curve the identity"
        )]
        if self.x1 == self.y1 && self.x2 == self.y2 {
            return (x, 1.0);
        }
        let u = self.solve(x);
        let dx = derivative(u, self.x1, self.x2);
        let rate = if dx.abs() < VERTICAL_TANGENT {
            0.0
        } else {
            derivative(u, self.y1, self.y2) / dx
        };
        (coordinate(u, self.y1, self.y2), rate)
    }

    /// The progress at normalized time `x`: [`sample`](Self::sample)'s
    /// first value.
    #[must_use]
    pub fn value(&self, x: f64) -> f64 {
        self.sample(x).0
    }

    /// The curve parameter `u` whose time coordinate is `x`: Newton's
    /// method kept inside a bracket that every step narrows, bisecting
    /// whenever a Newton step would leave it. `x(u)` increases on `[0, 1]`
    /// for a valid curve, so the bracket always holds the solution.
    fn solve(&self, x: f64) -> f64 {
        let (mut low, mut high, mut u) = (0.0_f64, 1.0_f64, x);
        for _ in 0..SOLVE_STEPS {
            let error = coordinate(u, self.x1, self.x2) - x;
            if error.abs() <= SOLVE_TOLERANCE {
                break;
            }
            if error > 0.0 {
                high = u;
            } else {
                low = u;
            }
            let newton = u - error / derivative(u, self.x1, self.x2);
            u = if newton > low && newton < high {
                newton
            } else {
                low.midpoint(high)
            };
        }
        u
    }
}

/// One coordinate of the curve at parameter `u`, for the control-point
/// coordinates `p1` and `p2`: `3(1−u)²u·p1 + 3(1−u)u²·p2 + u³`.
fn coordinate(u: f64, p1: f64, p2: f64) -> f64 {
    let v = 1.0 - u;
    (3.0 * v * v * u).mul_add(p1, (3.0 * v * u * u).mul_add(p2, u * u * u))
}

/// The derivative of [`coordinate`] in `u`:
/// `3(1−u)²·p1 + 6(1−u)u·(p2−p1) + 3u²·(1−p2)`.
fn derivative(u: f64, p1: f64, p2: f64) -> f64 {
    let v = 1.0 - u;
    (3.0 * v * v).mul_add(p1, (6.0 * v * u).mul_add(p2 - p1, 3.0 * u * u * (1.0 - p2)))
}

/// A damped harmonic oscillator on a unit mass, in real seconds.
///
/// `response` is the undamped period and `damping` the damping ratio `ζ`:
/// `1` is critically damped, below it the spring overshoots, above it the
/// spring is overdamped. [`from_physics`](Self::from_physics) builds one from
/// a stiffness and a damping coefficient, and
/// [`stiffness`](Self::stiffness) and
/// [`damping_coefficient`](Self::damping_coefficient) give them back for
/// platform animators that take those.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    /// The undamped period, in seconds: `2π / ω₀`.
    pub response: f64,
    /// The damping ratio `ζ`; `1` is critically damped.
    pub damping: f64,
}

impl Spring {
    /// A critically damped spring with a half-second response.
    #[must_use]
    pub const fn smooth() -> Self {
        Self {
            response: 0.5,
            damping: 1.0,
        }
    }

    /// A spring with slight overshoot and a half-second response.
    #[must_use]
    pub const fn snappy() -> Self {
        Self {
            response: 0.5,
            damping: 0.85,
        }
    }

    /// A spring with visible bounce and a half-second response.
    #[must_use]
    pub const fn bouncy() -> Self {
        Self {
            response: 0.5,
            damping: 0.7,
        }
    }

    /// The spring with `stiffness` `k` and damping coefficient `c` on a
    /// unit mass: `ω₀ = √k`, `ζ = c / (2√k)`.
    #[must_use]
    pub fn from_physics(stiffness: f64, damping: f64) -> Self {
        let omega = stiffness.sqrt();
        Self {
            response: TAU / omega,
            damping: damping / (2.0 * omega),
        }
    }

    /// The natural angular frequency `ω₀ = 2π / response`, in radians per
    /// second.
    #[must_use]
    pub fn angular_frequency(&self) -> f64 {
        TAU / self.response
    }

    /// The stiffness on a unit mass: `ω₀²`.
    #[must_use]
    pub fn stiffness(&self) -> f64 {
        let omega = self.angular_frequency();
        omega * omega
    }

    /// The damping coefficient on a unit mass: `2ζω₀`.
    #[must_use]
    pub fn damping_coefficient(&self) -> f64 {
        2.0 * self.damping * self.angular_frequency()
    }

    /// Whether the response and the damping ratio are finite and greater
    /// than zero. An undamped spring never settles.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.response.is_finite()
            && self.response > 0.0
            && self.damping.is_finite()
            && self.damping > 0.0
    }

    /// The displacement from rest and the velocity `t` seconds after the
    /// spring was released at displacement `offset` with `velocity`.
    #[must_use]
    pub fn sample(&self, offset: f64, velocity: f64, t: f64) -> (f64, f64) {
        let omega = self.angular_frequency();
        let ratio = self.damping;
        match regime(ratio) {
            Regime::Critical => {
                // x = e^−ωt (x₀ + (v₀ + ω x₀) t).
                let slope = omega.mul_add(offset, velocity);
                let decay = (-omega * t).exp();
                let factor = slope.mul_add(t, offset);
                (
                    decay * factor,
                    decay * (omega * factor).mul_add(-1.0, slope),
                )
            }
            Regime::Under => {
                let damped = omega * ratio.mul_add(-ratio, 1.0).sqrt();
                let cosine_part = offset;
                let sine_part = (ratio * omega).mul_add(offset, velocity) / damped;
                let decay = (-ratio * omega * t).exp();
                let (sin, cos) = (damped * t).sin_cos();
                (
                    decay * cosine_part.mul_add(cos, sine_part * sin),
                    decay
                        * ((ratio * omega)
                            .mul_add(sine_part, cosine_part * damped)
                            .mul_add(
                                -sin,
                                (sine_part * damped)
                                    .mul_add(cos, -(ratio * omega * cosine_part) * cos),
                            )),
                )
            }
            Regime::Over => {
                let root = omega * ratio.mul_add(ratio, -1.0).sqrt();
                let slow = -(ratio * omega) + root;
                let fast = -(ratio * omega) - root;
                let fast_part = slow.mul_add(-offset, velocity) / (fast - slow);
                let slow_part = offset - fast_part;
                (
                    slow_part.mul_add((slow * t).exp(), fast_part * (fast * t).exp()),
                    (slow_part * slow)
                        .mul_add((slow * t).exp(), fast_part * fast * (fast * t).exp()),
                )
            }
        }
    }

    /// Seconds after which a spring released at displacement `offset` with
    /// `velocity` has settled: from then on its displacement and velocity
    /// stay within [`SETTLE_TOLERANCE`] of rest.
    ///
    /// It is where the solution's envelope crosses the tolerance, so it
    /// depends only on how the spring started, and a spring is settled at
    /// every instant from it on — unlike a test of one sample, which an
    /// oscillation can pass and then fail again. Zero when the spring starts
    /// settled.
    #[must_use]
    pub fn settling_duration(&self, offset: f64, velocity: f64) -> f64 {
        let omega = self.angular_frequency();
        let ratio = self.damping;
        let (bound, decay) = match regime(ratio) {
            Regime::Under => {
                // |x|, |v|/ω ≤ A e^−ζωt with A the oscillation's amplitude.
                let decay = ratio * omega;
                let damped = omega * ratio.mul_add(-ratio, 1.0).sqrt();
                let amplitude = offset.hypot(decay.mul_add(offset, velocity) / damped);
                (amplitude * omega.max(1.0), decay)
            }
            Regime::Over => {
                // Both modes decay at least as fast as the slow one.
                let sum = ratio + ratio.mul_add(ratio, -1.0).sqrt();
                let slow = -omega / sum;
                let fast = -omega * sum;
                let slow_part = fast.mul_add(-offset, velocity) / (slow - fast);
                let fast_part = offset - slow_part;
                (
                    (slow_part.abs() + fast_part.abs())
                        .max((slow_part * slow).abs() + (fast_part * fast).abs()),
                    -slow,
                )
            }
            Regime::Critical => {
                // |x|, |v| ≤ B (1 + t) e^−ωt, which decreases from t = 1/ω on.
                // Its crossing has no closed form: Newton's method solves
                // g(t) = ln(B/tol) + ln(1 + t) − ωt = 0. g is concave and
                // decreasing there, so after the first step every iterate
                // lies at or past the root and the iterates fall to it.
                let slope = omega.mul_add(offset, velocity);
                let bound = offset
                    .abs()
                    .max(slope.abs())
                    .max(velocity.abs())
                    .max((omega * slope).abs());
                if bound <= SETTLE_TOLERANCE {
                    return 0.0;
                }
                let excess = (bound / SETTLE_TOLERANCE).ln();
                let g = |time: f64| omega.mul_add(-time, excess + time.ln_1p());
                let mut time = 1.0 / omega;
                if g(time) <= 0.0 {
                    return time;
                }
                for _ in 0..SOLVE_STEPS {
                    let value = g(time);
                    let step = value / (1.0 / (1.0 + time) - omega);
                    time -= step;
                    if value <= 0.0 && step.abs() <= SETTLE_TIME_TOLERANCE {
                        break;
                    }
                }
                return time;
            }
        };
        (bound / SETTLE_TOLERANCE).ln().max(0.0) / decay
    }

    /// How long a unit step from rest takes to settle: the
    /// [`settling_duration`](Self::settling_duration) of displacement one,
    /// rounded up to the nanosecond so a track sampled at it is done.
    ///
    /// `None` when the spring never settles, which only an invalid spring
    /// does.
    #[must_use]
    pub fn unit_settling_duration(&self) -> Option<Duration> {
        let seconds = self.settling_duration(-1.0, 0.0);
        let duration = Duration::try_from_secs_f64(seconds).ok()?;
        Some(if duration.as_secs_f64() < seconds {
            duration + Duration::from_nanos(1)
        } else {
            duration
        })
    }
}

/// Which closed-form solution a damping ratio takes.
enum Regime {
    Under,
    Critical,
    Over,
}

fn regime(ratio: f64) -> Regime {
    if (ratio - 1.0).abs() < CRITICAL_BAND {
        Regime::Critical
    } else if ratio < 1.0 {
        Regime::Under
    } else {
        Regime::Over
    }
}

/// How a track moves from where it was to its target: along a timing curve
/// over a fixed duration, or by a spring.
#[derive(Clone, Copy, Debug, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every evaluator realizes each kind of motion; a new kind must fail their build instead of falling into a wildcard arm"
)]
pub enum Timing {
    /// A cubic Bézier timing curve over `duration` seconds.
    Curve {
        /// The timing curve.
        curve: CubicBezier,
        /// How long the curve runs, in seconds.
        duration: f64,
    },
    /// A spring, running until it settles.
    Spring(Spring),
}

impl Timing {
    /// Evaluates a track `t` seconds in, lane by lane, writing each lane's
    /// position and velocity per second; returns whether the track is done.
    ///
    /// The track started at `from` moving at `velocity` and moves toward
    /// `target`. A curve ignores the starting velocity and is done once its
    /// duration has passed; a spring carries it and is done once every lane
    /// has settled, from the longest lane's
    /// [`settling_duration`](Spring::settling_duration) on. Done is a
    /// function of `t` alone, and once done a track stays done: it reports
    /// its target exactly and zero velocity.
    ///
    /// # Panics
    ///
    /// When `velocity`, `target`, `position` or `rate` is shorter than
    /// `from`.
    pub fn sample_lanes(
        &self,
        from: &[f64],
        velocity: &[f64],
        target: &[f64],
        t: f64,
        position: &mut [f64],
        rate: &mut [f64],
    ) -> bool {
        let lanes = from.len();
        debug_assert!(
            velocity.len() == lanes
                && target.len() == lanes
                && position.len() == lanes
                && rate.len() == lanes,
            "every lane slice of a track has the same length"
        );
        let done = match *self {
            Self::Curve { curve, duration } => {
                let progress = if duration <= 0.0 { 1.0 } else { t / duration };
                let (value, slope) = curve.sample(progress);
                let speed = if duration > 0.0 {
                    slope / duration
                } else {
                    0.0
                };
                for lane in 0..lanes {
                    let delta = target[lane] - from[lane];
                    position[lane] = delta.mul_add(value, from[lane]);
                    rate[lane] = delta * speed;
                }
                progress >= 1.0
            }
            Self::Spring(spring) => {
                let mut settled = true;
                for lane in 0..lanes {
                    let start = from[lane] - target[lane];
                    let (offset, speed) = spring.sample(start, velocity[lane], t);
                    position[lane] = target[lane] + offset;
                    rate[lane] = speed;
                    settled &= t >= spring.settling_duration(start, velocity[lane]);
                }
                settled
            }
        };
        if done {
            position[..lanes].copy_from_slice(&target[..lanes]);
            rate[..lanes].fill(0.0);
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether a lane `offset` from rest moving at `velocity` is within the
    /// tolerance in both.
    fn is_settled(offset: f64, velocity: f64) -> bool {
        offset.abs() <= SETTLE_TOLERANCE && velocity.abs() <= SETTLE_TOLERANCE
    }

    /// A unit-distance step from rest along `timing`, `t` seconds in.
    fn step(timing: Timing, t: f64) -> (f64, f64, bool) {
        let (mut position, mut rate) = ([0.0], [0.0]);
        let done = timing.sample_lanes(&[0.0], &[0.0], &[1.0], t, &mut position, &mut rate);
        (position[0], rate[0], done)
    }

    #[test]
    fn curves_hit_their_endpoints_exactly() {
        for curve in [
            CubicBezier::LINEAR,
            CubicBezier::EASE,
            CubicBezier::EASE_IN,
            CubicBezier::EASE_OUT,
            CubicBezier::EASE_IN_OUT,
        ] {
            assert_eq!(curve.sample(0.0), (0.0, 0.0));
            assert_eq!(curve.sample(-1.0), (0.0, 0.0));
            assert_eq!(curve.sample(1.0), (1.0, 0.0));
            assert_eq!(curve.sample(2.0), (1.0, 0.0));
        }
    }

    #[test]
    fn linear_is_the_identity() {
        for x in [0.125, 0.25, 0.5, 0.875] {
            assert_eq!(CubicBezier::LINEAR.sample(x), (x, 1.0));
        }
    }

    #[test]
    fn the_solve_inverts_time_within_its_tolerance() {
        let curves = [
            CubicBezier::EASE,
            CubicBezier::EASE_IN_OUT,
            CubicBezier::new(0.0, 1.0, 1.0, 0.0),
            CubicBezier::new(1.0, 0.0, 0.0, 1.0),
            CubicBezier::new(0.2, 0.0, 0.0, 1.0),
            CubicBezier::new(0.68, -0.6, 0.32, 1.6),
        ];
        for curve in curves {
            for step in 1..100 {
                let x = f64::from(step) / 100.0;
                let u = curve.solve(x);
                let error = coordinate(u, curve.x1, curve.x2) - x;
                assert!(
                    error.abs() <= SOLVE_TOLERANCE,
                    "{curve:?} at {x}: x(u) misses by {error}"
                );
                let (value, rate) = curve.sample(x);
                assert!(value.is_finite() && rate.is_finite(), "{curve:?} at {x}");
            }
        }
    }

    #[test]
    fn the_rate_is_the_curve_slope() {
        let curve = CubicBezier::EASE_IN_OUT;
        let h = 1e-6;
        for x in [0.1, 0.3, 0.5, 0.7, 0.9] {
            let numeric = (curve.value(x + h) - curve.value(x - h)) / (2.0 * h);
            let (_, rate) = curve.sample(x);
            assert!((numeric - rate).abs() < 1e-5, "{x}: {numeric} vs {rate}");
        }
    }

    #[test]
    fn physical_parameters_round_trip() {
        let spring = Spring::from_physics(100.0, 10.0);
        assert!((spring.angular_frequency() - 10.0).abs() < 1e-12);
        assert!((spring.damping - 0.5).abs() < 1e-12);
        assert!((spring.stiffness() - 100.0).abs() < 1e-9);
        assert!((spring.damping_coefficient() - 10.0).abs() < 1e-12);
    }

    #[test]
    fn validity_requires_a_positive_finite_response_and_damping() {
        assert!(Spring::smooth().is_valid());
        assert!(!Spring::from_physics(100.0, 0.0).is_valid());
        assert!(!Spring::from_physics(0.0, 10.0).is_valid());
        assert!(!Spring::from_physics(f64::NAN, 10.0).is_valid());
        assert!(CubicBezier::EASE.is_valid());
        assert!(!CubicBezier::new(1.5, 0.0, 0.5, 1.0).is_valid());
        assert!(!CubicBezier::new(0.5, f64::INFINITY, 0.5, 1.0).is_valid());
    }

    #[test]
    fn every_regime_solves_the_oscillator() {
        // x″ = −k x − c x′ on a unit mass, checked by finite differences.
        for damping in [0.3, 1.0, 2.5] {
            let spring = Spring {
                response: 0.4,
                damping,
            };
            let (stiffness, coefficient) = (spring.stiffness(), spring.damping_coefficient());
            let step = 1e-5;
            for time in [0.01, 0.1, 0.3, 0.8] {
                let (offset, velocity) = spring.sample(-1.0, 3.0, time);
                let (offset_after, velocity_after) = spring.sample(-1.0, 3.0, time + step);
                let (offset_before, velocity_before) = spring.sample(-1.0, 3.0, time - step);
                let numeric_velocity = (offset_after - offset_before) / (2.0 * step);
                let numeric_acceleration = (velocity_after - velocity_before) / (2.0 * step);
                let acceleration = (-stiffness).mul_add(offset, -coefficient * velocity);
                assert!(
                    (numeric_velocity - velocity).abs() < 1e-4,
                    "ζ {damping} at {time}: velocity"
                );
                assert!(
                    (numeric_acceleration - acceleration).abs() < 1e-2,
                    "ζ {damping} at {time}: acceleration"
                );
            }
            let (x0, v0) = spring.sample(-1.0, 3.0, 0.0);
            assert!(
                (x0 + 1.0).abs() < 1e-12 && (v0 - 3.0).abs() < 1e-12,
                "ζ {damping} at t = 0"
            );
        }
    }

    #[test]
    fn a_critically_damped_step_never_overshoots() {
        let timing = Timing::Spring(Spring::smooth());
        for frame in 0..480 {
            let (position, ..) = step(timing, f64::from(frame) / 120.0);
            assert!(position <= 1.0, "overshot to {position}");
        }
    }

    #[test]
    fn a_settled_spring_reports_its_target_exactly() {
        let timing = Timing::Spring(Spring::bouncy());
        let mut frame = 0;
        let (position, rate) = loop {
            let (position, rate, done) = step(timing, f64::from(frame) / 120.0);
            if done {
                break (position, rate);
            }
            frame += 1;
            assert!(frame < 1200, "the spring settles within ten seconds");
        };
        assert_eq!((position, rate), (1.0, 0.0));
    }

    #[test]
    fn the_settling_duration_bounds_every_later_sample() {
        let springs = [
            Spring::smooth(),
            Spring::snappy(),
            Spring::bouncy(),
            Spring::from_physics(100.0, 10.0),
            Spring::from_physics(400.0, 60.0),
            Spring::from_physics(2.0, 0.5),
        ];
        for spring in springs {
            for (offset, velocity) in [(-1.0, 0.0), (250.0, -900.0), (0.0, 40.0)] {
                let settle = spring.settling_duration(offset, velocity);
                assert!(settle.is_finite() && settle > 0.0, "{spring:?}");
                for frame in 0..600 {
                    let t = settle + f64::from(frame) / 60.0;
                    let (x, v) = spring.sample(offset, velocity, t);
                    assert!(
                        is_settled(x, v),
                        "{spring:?} from ({offset}, {velocity}) at {t}: ({x}, {v})"
                    );
                }
            }
            assert!(spring.settling_duration(0.0, 0.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn a_spring_is_done_from_its_settling_duration_on_and_stays_done() {
        for spring in [
            Spring::smooth(),
            Spring::bouncy(),
            Spring::from_physics(100.0, 10.0),
            Spring::from_physics(150.0, 60.0),
        ] {
            let timing = Timing::Spring(spring);
            let settle = spring.settling_duration(-1.0, 0.0);
            let mut was_done = false;
            for frame in 0..1200 {
                let t = f64::from(frame) / 240.0;
                let (.., done) = step(timing, t);
                assert_eq!(done, t >= settle, "{spring:?} at {t}");
                assert!(!was_done || done, "{spring:?} came undone at {t}");
                was_done = done;
            }
            let unit = spring
                .unit_settling_duration()
                .expect("a valid spring settles");
            assert!(unit.as_secs_f64() >= settle);
            assert!(
                step(timing, unit.as_secs_f64()).2,
                "{spring:?} done at its duration"
            );
        }
        assert_eq!(
            Spring::from_physics(100.0, 0.0).unit_settling_duration(),
            None
        );
    }

    #[test]
    fn a_retargeted_spring_is_continuous_in_position_and_velocity() {
        let timing = Timing::Spring(Spring::snappy());
        let (mut position, mut rate) = ([0.0], [0.0]);
        timing.sample_lanes(&[0.0], &[0.0], &[10.0], 0.1, &mut position, &mut rate);
        // Retarget from the sampled state: the new track starts there.
        let (mut next, mut next_rate) = ([0.0], [0.0]);
        timing.sample_lanes(&position, &rate, &[30.0], 0.0, &mut next, &mut next_rate);
        assert!((next[0] - position[0]).abs() < 1e-12);
        assert!((next_rate[0] - rate[0]).abs() < 1e-9);
    }

    #[test]
    fn a_curve_track_runs_its_duration_and_lands_exactly() {
        let timing = Timing::Curve {
            curve: CubicBezier::EASE_IN_OUT,
            duration: 0.3,
        };
        let (mut position, mut rate) = ([0.0; 2], [0.0; 2]);
        let target = [7.25, -3.0];
        assert!(!timing.sample_lanes(
            &[1.0, 2.0],
            &[0.0; 2],
            &target,
            0.15,
            &mut position,
            &mut rate
        ));
        assert!(
            (position[0] - 4.125).abs() < 1e-6,
            "eased midpoint {position:?}"
        );
        assert!(timing.sample_lanes(
            &[1.0, 2.0],
            &[0.0; 2],
            &target,
            0.3,
            &mut position,
            &mut rate
        ));
        assert_eq!((position, rate), (target, [0.0; 2]));
    }
}
