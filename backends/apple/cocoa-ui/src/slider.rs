//! Shared types for the slider control twins.
//!
//! The platform sliders live in [`crate::appkit::Slider`] and
//! [`crate::uikit::Slider`]; this module carries the values their
//! signatures share.

/// How large the slider is drawn.
///
/// `AppKit` honours every value through `NSControl.ControlSize`; `UIKit`
/// has no slider size classes, so [`crate::uikit::Slider::set_control_size`]
/// accepts the value and ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlSize {
    /// The most compact size.
    Mini,
    /// A compact size.
    Small,
    /// The standard size.
    Regular,
    /// A prominent size.
    Large,
}

/// How a programmatic value change transitions to the new value.
///
/// `UIKit` animates `setValue(_:animated:)`-style changes inside a
/// `UIView` animation block of the given duration and ignores the timing
/// curve; `AppKit` runs the change inside an `NSAnimationContext` with the
/// duration and, for [`ValueAnimation::Bezier`], the cubic timing function.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueAnimation {
    /// A fixed-duration transition with the platform's default timing.
    Duration(f64),
    /// A cubic-bezier-timed transition: the duration in seconds and the two
    /// control points of the timing function.
    Bezier {
        /// The transition duration in seconds.
        duration: f64,
        /// The cubic timing function's control points `[x1, y1, x2, y2]`.
        control_points: [f32; 4],
    },
}

impl ValueAnimation {
    /// The transition duration in seconds.
    #[must_use]
    pub const fn duration(&self) -> f64 {
        match *self {
            Self::Duration(duration) | Self::Bezier { duration, .. } => duration,
        }
    }
}
