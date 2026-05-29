//! Strongly-typed chart parameter helpers.
//!
//! These types provide a more ergonomic and safer API surface than raw `f32`
//! by validating invariants once at construction time.

/// Error returned when chart style/config values are invalid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChartParamError {
    /// Value must be finite (`NaN`/`inf` are rejected).
    NonFinite {
        /// Parameter name.
        param: &'static str,
        /// Provided value.
        value: f32,
    },
    /// Value must be strictly greater than zero.
    NonPositive {
        /// Parameter name.
        param: &'static str,
        /// Provided value.
        value: f32,
    },
    /// Value is out of the allowed closed range.
    OutOfRange {
        /// Parameter name.
        param: &'static str,
        /// Provided value.
        value: f32,
        /// Inclusive minimum.
        min: f32,
        /// Inclusive maximum.
        max: f32,
    },
    /// Parameters must satisfy `end > start`.
    InvalidOrder {
        /// Start value.
        start: f32,
        /// End value.
        end: f32,
    },
}

impl core::fmt::Display for ChartParamError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::NonFinite { param, value } => {
                write!(f, "{param} must be finite, got {value}")
            }
            Self::NonPositive { param, value } => {
                write!(f, "{param} must be > 0, got {value}")
            }
            Self::OutOfRange {
                param,
                value,
                min,
                max,
            } => {
                write!(f, "{param} must be in [{min}, {max}], got {value}")
            }
            Self::InvalidOrder { start, end } => {
                write!(f, "requires end > start, got start={start}, end={end}")
            }
        }
    }
}

impl std::error::Error for ChartParamError {}

/// A finite `f32` that must be strictly greater than zero.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct PositiveF32(f32);

impl PositiveF32 {
    /// Validates and creates a positive scalar.
    ///
    /// # Errors
    ///
    /// Returns [`ChartParamError`] when `value` is non-finite or is not strictly positive.
    pub fn try_new(value: f32) -> Result<Self, ChartParamError> {
        if !value.is_finite() {
            return Err(ChartParamError::NonFinite {
                param: "value",
                value,
            });
        }
        if value <= 0.0 {
            return Err(ChartParamError::NonPositive {
                param: "value",
                value,
            });
        }
        Ok(Self(value))
    }

    /// Returns the inner `f32`.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl From<f32> for PositiveF32 {
    /// Fail-fast convenience for chart builders so callers can write
    /// `chart.line_width(2.0)` without an explicit `PositiveF32::try_new`.
    /// Panics with a descriptive message on `NaN`, infinity, or non-positive
    /// input. Use [`PositiveF32::try_new`] when the failure must be handled.
    fn from(value: f32) -> Self {
        Self::try_new(value).unwrap_or_else(|err| panic!("PositiveF32 from f32: {err}"))
    }
}

/// A finite `f32` constrained to `[0.0, 1.0]`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct UnitInterval(f32);

impl UnitInterval {
    /// Validates and creates a unit-interval scalar.
    ///
    /// # Errors
    ///
    /// Returns [`ChartParamError`] when `value` is non-finite or is outside `0.0..=1.0`.
    pub fn try_new(value: f32) -> Result<Self, ChartParamError> {
        if !value.is_finite() {
            return Err(ChartParamError::NonFinite {
                param: "value",
                value,
            });
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(ChartParamError::OutOfRange {
                param: "value",
                value,
                min: 0.0,
                max: 1.0,
            });
        }
        Ok(Self(value))
    }

    /// Returns the inner `f32`.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl From<f32> for UnitInterval {
    /// Fail-fast convenience for chart builders. Panics on `NaN`, infinity,
    /// or values outside `[0.0, 1.0]`.
    fn from(value: f32) -> Self {
        Self::try_new(value).unwrap_or_else(|err| panic!("UnitInterval from f32: {err}"))
    }
}

/// Donut inner radius ratio for pie charts.
///
/// Restricted to `[0.0, 0.95]` to guarantee a visible ring thickness.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct DonutInnerRadius(f32);

impl DonutInnerRadius {
    /// Validates and creates a donut inner-radius ratio.
    ///
    /// # Errors
    ///
    /// Returns [`ChartParamError`] when `value` is non-finite or is outside `0.0..=0.95`.
    pub fn try_new(value: f32) -> Result<Self, ChartParamError> {
        if !value.is_finite() {
            return Err(ChartParamError::NonFinite {
                param: "inner_radius",
                value,
            });
        }
        if !(0.0..=0.95).contains(&value) {
            return Err(ChartParamError::OutOfRange {
                param: "inner_radius",
                value,
                min: 0.0,
                max: 0.95,
            });
        }
        Ok(Self(value))
    }

    /// Returns the inner radius ratio.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl From<f32> for DonutInnerRadius {
    /// Fail-fast convenience for chart builders. Panics on `NaN`, infinity,
    /// or values outside `[0.0, 0.95]`.
    fn from(value: f32) -> Self {
        Self::try_new(value).unwrap_or_else(|err| panic!("DonutInnerRadius from f32: {err}"))
    }
}

/// Gauge arc angle span in radians (`end > start`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcAngles {
    start_radians: f32,
    end_radians: f32,
}

impl ArcAngles {
    /// Creates arc angles from radians.
    ///
    /// # Errors
    ///
    /// Returns [`ChartParamError`] when either angle is non-finite or `end <= start`.
    pub fn try_radians(start: f32, end: f32) -> Result<Self, ChartParamError> {
        if !start.is_finite() {
            return Err(ChartParamError::NonFinite {
                param: "start",
                value: start,
            });
        }
        if !end.is_finite() {
            return Err(ChartParamError::NonFinite {
                param: "end",
                value: end,
            });
        }
        if end <= start {
            return Err(ChartParamError::InvalidOrder { start, end });
        }
        Ok(Self {
            start_radians: start,
            end_radians: end,
        })
    }

    /// Creates arc angles from degrees.
    ///
    /// # Errors
    ///
    /// Returns [`ChartParamError`] when either angle is non-finite or `end <= start`.
    pub fn try_degrees(start: f32, end: f32) -> Result<Self, ChartParamError> {
        Self::try_radians(start.to_radians(), end.to_radians())
    }

    /// Fail-fast factory for arc angles in radians. Panics on invalid input.
    ///
    /// Use [`ArcAngles::try_radians`] when the failure must be handled.
    ///
    /// # Panics
    ///
    /// Panics when either angle is non-finite or `end <= start`.
    #[must_use]
    pub fn from_radians(start: f32, end: f32) -> Self {
        Self::try_radians(start, end).unwrap_or_else(|err| panic!("ArcAngles from_radians: {err}"))
    }

    /// Fail-fast factory for arc angles in degrees. Panics on invalid input.
    ///
    /// # Panics
    ///
    /// Panics when either angle is non-finite or `end <= start`.
    #[must_use]
    pub fn from_degrees(start: f32, end: f32) -> Self {
        Self::try_degrees(start, end).unwrap_or_else(|err| panic!("ArcAngles from_degrees: {err}"))
    }

    /// Arc start angle in radians.
    #[must_use]
    pub const fn start_radians(self) -> f32 {
        self.start_radians
    }

    /// Arc end angle in radians.
    #[must_use]
    pub const fn end_radians(self) -> f32 {
        self.end_radians
    }
}

/// Gauge ring radii (`0.0 <= inner < outer <= 0.5`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GaugeRadii {
    inner: f32,
    outer: f32,
}

impl GaugeRadii {
    /// Creates validated gauge radii.
    ///
    /// # Errors
    ///
    /// Returns [`ChartParamError`] when either radius is non-finite or the
    /// radii do not satisfy `0.0 <= inner < outer <= 0.5`.
    pub fn try_new(inner: f32, outer: f32) -> Result<Self, ChartParamError> {
        if !inner.is_finite() {
            return Err(ChartParamError::NonFinite {
                param: "inner",
                value: inner,
            });
        }
        if !outer.is_finite() {
            return Err(ChartParamError::NonFinite {
                param: "outer",
                value: outer,
            });
        }
        if !(0.0..=0.5).contains(&inner) {
            return Err(ChartParamError::OutOfRange {
                param: "inner",
                value: inner,
                min: 0.0,
                max: 0.5,
            });
        }
        if !(0.0..=0.5).contains(&outer) {
            return Err(ChartParamError::OutOfRange {
                param: "outer",
                value: outer,
                min: 0.0,
                max: 0.5,
            });
        }
        if outer <= inner {
            return Err(ChartParamError::InvalidOrder {
                start: inner,
                end: outer,
            });
        }
        Ok(Self { inner, outer })
    }

    /// Fail-fast factory. Panics on invalid input. Use [`Self::try_new`] when
    /// the failure must be handled.
    ///
    /// # Panics
    ///
    /// Panics when either radius is non-finite or the radii do not satisfy
    /// `0.0 <= inner < outer <= 0.5`.
    #[must_use]
    pub fn new(inner: f32, outer: f32) -> Self {
        Self::try_new(inner, outer).unwrap_or_else(|err| panic!("GaugeRadii::new: {err}"))
    }

    /// Inner radius.
    #[must_use]
    pub const fn inner(self) -> f32 {
        self.inner
    }

    /// Outer radius.
    #[must_use]
    pub const fn outer(self) -> f32 {
        self.outer
    }
}

#[cfg(test)]
mod tests {
    use super::{ArcAngles, ChartParamError, GaugeRadii, PositiveF32, UnitInterval};

    #[test]
    fn positive_rejects_zero() {
        let err = PositiveF32::try_new(0.0).expect_err("zero should fail");
        assert_eq!(
            err,
            ChartParamError::NonPositive {
                param: "value",
                value: 0.0
            }
        );
    }

    #[test]
    fn unit_interval_rejects_out_of_range() {
        let err = UnitInterval::try_new(1.1).expect_err("out of range should fail");
        assert_eq!(
            err,
            ChartParamError::OutOfRange {
                param: "value",
                value: 1.1,
                min: 0.0,
                max: 1.0
            }
        );
    }

    #[test]
    fn arc_angles_require_end_greater_than_start() {
        let err = ArcAngles::try_radians(1.0, 0.5).expect_err("invalid order should fail");
        assert_eq!(
            err,
            ChartParamError::InvalidOrder {
                start: 1.0,
                end: 0.5
            }
        );
    }

    #[test]
    fn gauge_radii_require_valid_ordering() {
        let err = GaugeRadii::try_new(0.4, 0.2).expect_err("invalid order should fail");
        assert_eq!(
            err,
            ChartParamError::InvalidOrder {
                start: 0.4,
                end: 0.2
            }
        );
    }
}
