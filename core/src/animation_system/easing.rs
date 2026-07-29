//! Unified easing system for `WaterUI` animations.
//!
//! This module provides the core easing curve types used across all `WaterUI`
//! components for consistent animation behavior. GPU renderers can use these
//! curves for shader-based interpolation, while native animations use them
//! for system animation configuration.
//!
//! # Design
//!
//! The easing system uses only two variants:
//! - `CubicBezier`: For all bezier-representable curves (linear, ease-in, ease-out, etc.)
//! - `Spring`: For physics-based spring animations that overshoot and oscillate
//!
//! Standard curves are provided as constants with CSS-standard bezier control points.

use core::time::Duration;

/// Declarative easing curve with only two variants.
///
/// Standard curves (linear, ease-in, ease-out, ease-in-out) are cubic bezier
/// curves with predefined control points. Custom bezier curves can be created
/// with any control points.
///
/// Spring animations cannot be represented by bezier curves due to their
/// oscillating nature, so they have a separate variant.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EasingCurve {
    /// Cubic bezier curve with control points (x1, y1, x2, y2).
    ///
    /// The curve starts at (0, 0) and ends at (1, 1). The control points
    /// define the shape of the curve between these endpoints.
    ///
    /// Standard CSS bezier curves are provided as constants.
    CubicBezier(f32, f32, f32, f32),
    /// Spring physics animation.
    ///
    /// Spring animations cannot be represented by bezier curves because they
    /// can overshoot the target value and oscillate before settling.
    Spring {
        /// Stiffness of the spring (higher = faster oscillation).
        stiffness: f32,
        /// Damping factor (higher = less bounce/oscillation).
        damping: f32,
    },
}

impl EasingCurve {
    /// Linear interpolation - constant velocity from start to finish.
    /// CSS: `linear` or `cubic-bezier(0, 0, 1, 1)`
    pub const LINEAR: Self = Self::CubicBezier(0.0, 0.0, 1.0, 1.0);

    /// Ease-in - starts slow and accelerates.
    /// CSS: `ease-in` or `cubic-bezier(0.42, 0, 1, 1)`
    pub const EASE_IN: Self = Self::CubicBezier(0.42, 0.0, 1.0, 1.0);

    /// Ease-out - starts fast and decelerates.
    /// CSS: `ease-out` or `cubic-bezier(0, 0, 0.58, 1)`
    pub const EASE_OUT: Self = Self::CubicBezier(0.0, 0.0, 0.58, 1.0);

    /// Ease-in-out - starts slow, speeds up, then slows down.
    /// CSS: `ease-in-out` or `cubic-bezier(0.42, 0, 0.58, 1)`
    pub const EASE_IN_OUT: Self = Self::CubicBezier(0.42, 0.0, 0.58, 1.0);

    /// Default ease curve (CSS `ease`).
    /// CSS: `ease` or `cubic-bezier(0.25, 0.1, 0.25, 1)`
    pub const EASE: Self = Self::CubicBezier(0.25, 0.1, 0.25, 1.0);

    /// Creates a custom cubic bezier curve.
    #[must_use]
    pub const fn bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self::CubicBezier(x1, y1, x2, y2)
    }

    /// Creates a spring animation with the given stiffness and damping.
    #[must_use]
    pub const fn spring(stiffness: f32, damping: f32) -> Self {
        Self::Spring { stiffness, damping }
    }

    /// Apply easing to a normalized time value t in [0, 1].
    ///
    /// Returns the eased progress value, which for most curves is also in [0, 1],
    /// but spring animations may temporarily overshoot (return values > 1 or < 0).
    #[must_use]
    pub fn ease(&self, t: f32) -> f32 {
        match self {
            Self::CubicBezier(x1, y1, x2, y2) => cubic_bezier_ease(t, *x1, *y1, *x2, *y2),
            Self::Spring { stiffness, damping } => spring_ease(t, *stiffness, *damping),
        }
    }

    /// Returns `true` if this is a spring animation.
    #[must_use]
    pub const fn is_spring(&self) -> bool {
        matches!(self, Self::Spring { .. })
    }
}

impl Default for EasingCurve {
    fn default() -> Self {
        Self::EASE_IN_OUT
    }
}

/// Cubic bezier easing implementation.
///
/// Uses Newton-Raphson iteration to find the t parameter for a given x,
/// then evaluates the bezier curve at that t to get y.
fn cubic_bezier_ease(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    const EPSILON: f32 = 0.0001;

    // Handle edge cases
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }

    // Linear case - no need for iteration
    if (x1 - y1).abs() < 0.0001 && (x2 - y2).abs() < 0.0001 {
        return t;
    }

    // First try Newton-Raphson for fast convergence.
    let mut guess = t;
    let mut converged = false;
    for _ in 0..8 {
        let x = bezier_sample(guess, x1, x2) - t;
        if x.abs() < EPSILON {
            converged = true;
            break;
        }
        let dx = bezier_derivative(guess, x1, x2);
        if dx.abs() < 0.000_001 {
            break;
        }
        let next = guess - x / dx;
        if !(0.0..=1.0).contains(&next) {
            break;
        }
        guess = next;
    }

    // Fall back to binary subdivision when Newton stalls (flat derivative,
    // poor initial guess, or highly skewed control points).
    if !converged {
        let mut low = 0.0;
        let mut high = 1.0;
        guess = t.clamp(0.0, 1.0);
        for _ in 0..16 {
            let sample = bezier_sample(guess, x1, x2);
            let delta = sample - t;
            if delta.abs() < EPSILON {
                break;
            }
            if delta > 0.0 {
                high = guess;
            } else {
                low = guess;
            }
            guess = f32::midpoint(low, high);
        }
    }

    // Clamp to valid range before sampling y.
    guess = guess.clamp(0.0, 1.0);

    // Return y value at the found t
    bezier_sample(guess, y1, y2)
}

/// Sample a cubic bezier curve at parameter t.
/// The curve goes through (0, 0) at t=0 and (1, 1) at t=1.
#[inline]
fn bezier_sample(t: f32, p1: f32, p2: f32) -> f32 {
    // B(t) = 3(1-t)²t·P1 + 3(1-t)t²·P2 + t³
    let t2 = t * t;
    let t3 = t2 * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    (3.0 * mt2 * t).mul_add(p1, (3.0 * mt * t2).mul_add(p2, t3))
}

/// Derivative of the bezier curve at parameter t.
#[inline]
fn bezier_derivative(t: f32, p1: f32, p2: f32) -> f32 {
    // B'(t) = 3(1-t)²·P1 + 6(1-t)t·(P2-P1) + 3t²·(1-P2)
    let t2 = t * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    (3.0 * mt2).mul_add(p1, (6.0 * mt * t).mul_add(p2 - p1, 3.0 * t2 * (1.0 - p2)))
}

/// Spring easing implementation using damped harmonic oscillator.
///
/// The spring starts at 0 and settles toward 1, potentially overshooting
/// and oscillating based on the stiffness and damping parameters.
fn spring_ease(t: f32, stiffness: f32, damping: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    if !stiffness.is_finite() || !damping.is_finite() || stiffness <= 0.0 || damping < 0.0 {
        // Invalid spring parameters should be rejected by constructors, but keep
        // easing numerically stable for deserialized or externally-provided data.
        return t;
    }

    // Damped harmonic oscillator
    // x(t) = 1 - e^(-ζωt) * (cos(ωd*t) + (ζω/ωd)*sin(ωd*t))
    // where ω = sqrt(stiffness), ζ = damping / (2*sqrt(stiffness))
    // ωd = ω * sqrt(1 - ζ²) for underdamped case

    let omega = stiffness.sqrt();
    let zeta = damping / (2.0 * omega);

    if zeta >= 1.0 {
        // Critically damped or overdamped - no oscillation
        let decay = (-omega * zeta * t).exp();
        1.0 - decay * (omega * zeta).mul_add(t, 1.0)
    } else {
        // Underdamped - oscillates
        let omega_d = omega * (1.0 - zeta * zeta).sqrt();
        let decay = (-zeta * omega * t).exp();
        let cos_part = (omega_d * t).cos();
        let sin_part = (zeta * omega / omega_d) * (omega_d * t).sin();
        1.0 - decay * (cos_part + sin_part)
    }
}

/// Trait for types that can be linearly interpolated.
///
/// This is used by the animation system to interpolate between values.
/// Most numeric types implement this automatically via the blanket impl.
pub trait Interpolatable: Clone {
    /// Linear interpolation: `self + (other - self) * t`
    #[must_use]
    fn lerp(&self, other: &Self, t: f32) -> Self;
}

// Implement for f32
impl Interpolatable for f32 {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

// Implement for f64
impl Interpolatable for f64 {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        self + (other - self) * Self::from(t)
    }
}

// Implement for tuples
impl<A: Interpolatable, B: Interpolatable> Interpolatable for (A, B) {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        (self.0.lerp(&other.0, t), self.1.lerp(&other.1, t))
    }
}

impl<A: Interpolatable, B: Interpolatable, C: Interpolatable> Interpolatable for (A, B, C) {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        (
            self.0.lerp(&other.0, t),
            self.1.lerp(&other.1, t),
            self.2.lerp(&other.2, t),
        )
    }
}

impl<A: Interpolatable, B: Interpolatable, C: Interpolatable, D: Interpolatable> Interpolatable
    for (A, B, C, D)
{
    fn lerp(&self, other: &Self, t: f32) -> Self {
        (
            self.0.lerp(&other.0, t),
            self.1.lerp(&other.1, t),
            self.2.lerp(&other.2, t),
            self.3.lerp(&other.3, t),
        )
    }
}

// Implement for arrays
impl<T: Interpolatable + Copy, const N: usize> Interpolatable for [T; N] {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let mut result = *self;
        for i in 0..N {
            result[i] = self[i].lerp(&other[i], t);
        }
        result
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
        assert!((curve.ease(0.0) - 0.0).abs() < 0.001);
        assert!((curve.ease(0.5) - 0.5).abs() < 0.001);
        assert!((curve.ease(1.0) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_ease_in() {
        let curve = EasingCurve::EASE_IN;
        assert!((curve.ease(0.0) - 0.0).abs() < 0.001);
        assert!((curve.ease(1.0) - 1.0).abs() < 0.001);
        // Ease-in should be slower at the start
        assert!(curve.ease(0.5) < 0.5);
    }

    #[test]
    fn test_ease_out() {
        let curve = EasingCurve::EASE_OUT;
        assert!((curve.ease(0.0) - 0.0).abs() < 0.001);
        assert!((curve.ease(1.0) - 1.0).abs() < 0.001);
        // Ease-out should be faster at the start
        assert!(curve.ease(0.5) > 0.5);
    }

    #[test]
    fn test_ease_in_out() {
        let curve = EasingCurve::EASE_IN_OUT;
        assert!((curve.ease(0.0) - 0.0).abs() < 0.001);
        assert!((curve.ease(1.0) - 1.0).abs() < 0.001);
        // Ease-in-out should be roughly 0.5 at 0.5
        assert!((curve.ease(0.5) - 0.5).abs() < 0.1);
    }

    #[test]
    fn test_spring_settles_to_one() {
        let curve = EasingCurve::spring(100.0, 10.0);
        assert!((curve.ease(0.0) - 0.0).abs() < 0.001);
        assert!((curve.ease(1.0) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_bezier_solver_handles_extreme_control_points() {
        let curve = EasingCurve::bezier(0.0, 1.0, 1.0, 0.0);
        for step in 0_u16..=100 {
            let t = f32::from(step) / 100.0;
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
}
