//! Geometric angle type with explicit unit constructors and conversions.

use core::f32::consts::{FRAC_PI_2, PI, TAU};
use core::fmt;
use core::ops::{
    Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, RemAssign, Sub, SubAssign,
};

use crate::easing::Interpolatable;

/// An angle, stored internally in radians.
///
/// Construct values with explicit units via [`Angle::degrees`],
/// [`Angle::radians`], or [`Angle::turns`], and convert back via
/// [`Angle::to_degrees`], [`Angle::to_radians`], or [`Angle::to_turns`].
/// Positive angles represent clockwise rotation in top-left screen coordinates.
#[derive(Clone, Copy, Default, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(transparent)]
pub struct Angle {
    radians: f32,
}

impl Angle {
    /// Zero angle (`0` radians / `0°`).
    pub const ZERO: Self = Self { radians: 0.0 };

    /// One quarter turn (`π / 2` radians / `90°`).
    pub const QUARTER: Self = Self { radians: FRAC_PI_2 };

    /// One half turn (`π` radians / `180°`).
    pub const HALF: Self = Self { radians: PI };

    /// One full turn (`2π` radians / `360°`).
    pub const FULL: Self = Self { radians: TAU };

    /// Creates an angle from a value in radians.
    #[must_use]
    pub const fn radians(radians: f32) -> Self {
        Self { radians }
    }

    /// Creates an angle from a value in degrees.
    #[must_use]
    pub const fn degrees(degrees: f32) -> Self {
        Self {
            radians: degrees.to_radians(),
        }
    }

    /// Creates an angle from a fraction or multiple of full turns (`1.0` = `360°` = `2π`).
    #[must_use]
    pub const fn turns(turns: f32) -> Self {
        Self {
            radians: turns * TAU,
        }
    }

    /// Returns this angle in radians.
    #[must_use]
    pub const fn to_radians(self) -> f32 {
        self.radians
    }

    /// Returns this angle in degrees.
    #[must_use]
    pub const fn to_degrees(self) -> f32 {
        self.radians.to_degrees()
    }

    /// Returns this angle in full turns (`1.0` = `360°` = `2π`).
    #[must_use]
    pub const fn to_turns(self) -> f32 {
        self.radians / TAU
    }

    /// Returns `true` if the underlying angle value is neither infinite nor `NaN`.
    #[must_use]
    pub const fn is_finite(self) -> bool {
        self.radians.is_finite()
    }

    /// Returns the non-negative magnitude of this angle.
    #[must_use]
    pub const fn abs(self) -> Self {
        Self {
            radians: self.radians.abs(),
        }
    }

    /// Clamps this angle between `min` and `max`.
    #[must_use]
    pub const fn clamp(self, min: Self, max: Self) -> Self {
        Self {
            radians: self.radians.clamp(min.radians, max.radians),
        }
    }

    /// Returns the smaller of `self` and `other`.
    #[must_use]
    pub const fn min(self, other: Self) -> Self {
        Self {
            radians: self.radians.min(other.radians),
        }
    }

    /// Returns the larger of `self` and `other`.
    #[must_use]
    pub const fn max(self, other: Self) -> Self {
        Self {
            radians: self.radians.max(other.radians),
        }
    }

    /// Computes the sine of this angle.
    #[must_use]
    pub fn sin(self) -> f32 {
        self.radians.sin()
    }

    /// Computes the cosine of this angle.
    #[must_use]
    pub fn cos(self) -> f32 {
        self.radians.cos()
    }

    /// Computes the tangent of this angle.
    #[must_use]
    pub fn tan(self) -> f32 {
        self.radians.tan()
    }

    /// Simultaneously computes the sine and cosine of this angle, `(sin, cos)`.
    #[must_use]
    pub fn sin_cos(self) -> (f32, f32) {
        self.radians.sin_cos()
    }

    /// Computes the least non-negative remainder of `self` modulo `rhs`.
    #[must_use]
    pub fn rem_euclid(self, rhs: Self) -> Self {
        Self {
            radians: self.radians.rem_euclid(rhs.radians),
        }
    }
}

impl fmt::Debug for Angle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Angle({}°)", self.to_degrees())
    }
}

impl fmt::Display for Angle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}°", self.to_degrees())
    }
}

impl Add for Angle {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            radians: self.radians + rhs.radians,
        }
    }
}

impl AddAssign for Angle {
    fn add_assign(&mut self, rhs: Self) {
        self.radians += rhs.radians;
    }
}

impl Sub for Angle {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            radians: self.radians - rhs.radians,
        }
    }
}

impl SubAssign for Angle {
    fn sub_assign(&mut self, rhs: Self) {
        self.radians -= rhs.radians;
    }
}

impl Neg for Angle {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self {
            radians: -self.radians,
        }
    }
}

impl Mul<f32> for Angle {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self {
            radians: self.radians * rhs,
        }
    }
}

impl Mul<Angle> for f32 {
    type Output = Angle;

    fn mul(self, rhs: Angle) -> Self::Output {
        Angle {
            radians: self * rhs.radians,
        }
    }
}

impl MulAssign<f32> for Angle {
    fn mul_assign(&mut self, rhs: f32) {
        self.radians *= rhs;
    }
}

impl Div<f32> for Angle {
    type Output = Self;

    fn div(self, rhs: f32) -> Self::Output {
        Self {
            radians: self.radians / rhs,
        }
    }
}

impl DivAssign<f32> for Angle {
    fn div_assign(&mut self, rhs: f32) {
        self.radians /= rhs;
    }
}

impl Div for Angle {
    type Output = f32;

    fn div(self, rhs: Self) -> Self::Output {
        self.radians / rhs.radians
    }
}

impl Rem for Angle {
    type Output = Self;

    fn rem(self, rhs: Self) -> Self::Output {
        Self {
            radians: self.radians % rhs.radians,
        }
    }
}

impl RemAssign for Angle {
    fn rem_assign(&mut self, rhs: Self) {
        self.radians %= rhs.radians;
    }
}

nami::impl_constant!(Angle);

impl Interpolatable for Angle {
    const LANES: usize = 1;

    fn write_lanes(&self, lanes: &mut [f64]) {
        lanes[0] = f64::from(self.radians);
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "an f32 angle's lane narrows back to f32"
    )]
    fn from_lanes(lanes: &[f64]) -> Self {
        Self {
            radians: lanes[0] as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use core::f32::consts::{FRAC_PI_2, PI, TAU};

    use nami::{Binding, Signal};

    use super::Angle;
    use crate::easing::Interpolatable;
    use crate::reactive::signal::IntoComputed;

    #[test]
    fn unit_constructors_and_constants() {
        assert!((Angle::ZERO.to_radians()).abs() < f32::EPSILON);
        assert!((Angle::QUARTER.to_radians() - FRAC_PI_2).abs() < f32::EPSILON);
        assert!((Angle::HALF.to_radians() - PI).abs() < f32::EPSILON);
        assert!((Angle::FULL.to_radians() - TAU).abs() < f32::EPSILON);

        assert!((Angle::degrees(90.0).to_radians() - FRAC_PI_2).abs() < 1e-6);
        assert!((Angle::degrees(180.0).to_turns() - 0.5).abs() < 1e-6);
        assert!((Angle::turns(0.25).to_degrees() - 90.0).abs() < 1e-5);
        assert!((Angle::radians(PI).to_degrees() - 180.0).abs() < 1e-5);
    }

    #[test]
    fn arithmetic_and_interpolation() {
        let a = Angle::degrees(30.0);
        let b = Angle::degrees(60.0);
        assert!(((a + b).to_degrees() - 90.0).abs() < 1e-5);
        assert!(((b - a).to_degrees() - 30.0).abs() < 1e-5);
        assert!(((-a).to_degrees() + 30.0).abs() < 1e-5);
        assert!(((a * 3.0).to_degrees() - 90.0).abs() < 1e-5);
        assert!(((3.0 * a).to_degrees() - 90.0).abs() < 1e-5);
        assert!(((b / 2.0).to_degrees() - 30.0).abs() < 1e-5);
        assert!((b / a - 2.0).abs() < 1e-6);

        let mid = Angle::ZERO.lerp(&Angle::HALF, 0.5);
        assert!((mid.to_radians() - FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn signal_and_into_computed() {
        fn accept(angle: impl IntoComputed<Angle>) -> nami::Computed<Angle> {
            angle.into_computed()
        }

        let constant_computed = accept(Angle::degrees(45.0));
        assert!((constant_computed.snapshot().to_degrees() - 45.0).abs() < 1e-5);

        let binding = Binding::container(Angle::ZERO);
        let reactive_computed = accept(binding.clone());
        assert!((reactive_computed.snapshot().to_radians()).abs() < f32::EPSILON);
        binding.set(Angle::QUARTER);
        assert!((reactive_computed.snapshot().to_degrees() - 90.0).abs() < 1e-5);
    }
}
