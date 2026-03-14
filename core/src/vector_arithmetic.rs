//! VectorArithmetic trait for types that can be linearly interpolated.
//!
//! This module provides the foundation for animation interpolation. The native
//! animation system uses these traits to interpolate between values when
//! animating properties.
//!
//! # Design
//!
//! Uses Rust's existing `std::ops` traits (Add, Sub, Mul) with a blanket impl.
//! Types just need to implement these traits to be animatable.
//!
//! # Examples
//!
//! ```rust
//! use waterui_core::vector_arithmetic::VectorArithmetic;
//!
//! // f64 already implements VectorArithmetic via blanket impl
//! let a = 0.0_f64;
//! let b = 1.0_f64;
//! let mid = a.lerp(&b, 0.5);
//! assert!((mid - 0.5).abs() < 0.001);
//! ```

use core::ops::{Add, Mul, Sub};

/// Types that can be linearly interpolated.
///
/// The native animation system uses this to interpolate between values.
/// Any type implementing Add, Sub, and Mul<f64> automatically gets this trait.
pub trait VectorArithmetic:
    Clone + Default + Send + 'static + Add<Output = Self> + Sub<Output = Self> + Mul<f64, Output = Self>
{
    /// Linear interpolation: self + (other - self) * t
    #[must_use]
    fn lerp(&self, other: &Self, t: f64) -> Self {
        self.clone() + (other.clone() - self.clone()) * t
    }
}

// Blanket impl for types with the right traits
impl<T> VectorArithmetic for T where
    T: Clone + Default + Send + 'static + Add<Output = T> + Sub<Output = T> + Mul<f64, Output = T>
{
}

/// Pair for composing two animatable values together.
///
/// Useful when you need to animate multiple related values as a unit.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct AnimatablePair<A, B>(pub A, pub B);

impl<A: Add<Output = A>, B: Add<Output = B>> Add for AnimatablePair<A, B> {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0, self.1 + other.1)
    }
}

impl<A: Sub<Output = A>, B: Sub<Output = B>> Sub for AnimatablePair<A, B> {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0, self.1 - other.1)
    }
}

impl<A: Mul<f64, Output = A>, B: Mul<f64, Output = B>> Mul<f64> for AnimatablePair<A, B> {
    type Output = Self;

    fn mul(self, scalar: f64) -> Self {
        Self(self.0 * scalar, self.1 * scalar)
    }
}

// ============================================================================
// Point2 - 2D position/size wrapper
// ============================================================================

/// A 2D point or size that can be animated.
///
/// Wraps `[f32; 2]` to provide `VectorArithmetic` implementation.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Point2(pub [f32; 2]);

impl Point2 {
    /// Creates a new Point2 from x and y coordinates.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self([x, y])
    }

    /// Returns the x coordinate.
    #[must_use]
    pub const fn x(&self) -> f32 {
        self.0[0]
    }

    /// Returns the y coordinate.
    #[must_use]
    pub const fn y(&self) -> f32 {
        self.0[1]
    }
}

impl From<[f32; 2]> for Point2 {
    fn from(arr: [f32; 2]) -> Self {
        Self(arr)
    }
}

impl From<Point2> for [f32; 2] {
    fn from(p: Point2) -> Self {
        p.0
    }
}

impl Add for Point2 {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self([self.0[0] + other.0[0], self.0[1] + other.0[1]])
    }
}

impl Sub for Point2 {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self([self.0[0] - other.0[0], self.0[1] - other.0[1]])
    }
}

impl Mul<f64> for Point2 {
    type Output = Self;

    fn mul(self, scalar: f64) -> Self {
        let s = scalar as f32;
        Self([self.0[0] * s, self.0[1] * s])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_f64_lerp() {
        let a = 0.0_f64;
        let b = 1.0_f64;
        assert!((a.lerp(&b, 0.0) - 0.0).abs() < 0.001);
        assert!((a.lerp(&b, 0.5) - 0.5).abs() < 0.001);
        assert!((a.lerp(&b, 1.0) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_point2_lerp() {
        let a = Point2::new(0.0, 0.0);
        let b = Point2::new(1.0, 2.0);
        let mid = a.lerp(&b, 0.5);
        assert!((mid.x() - 0.5).abs() < 0.001);
        assert!((mid.y() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_animatable_pair() {
        let a = AnimatablePair(0.0_f64, Point2::new(0.0, 0.0));
        let b = AnimatablePair(1.0_f64, Point2::new(2.0, 4.0));
        let mid = a.lerp(&b, 0.5);
        assert!((mid.0 - 0.5).abs() < 0.001);
        assert!((mid.1.x() - 1.0).abs() < 0.001);
        assert!((mid.1.y() - 2.0).abs() < 0.001);
    }
}
