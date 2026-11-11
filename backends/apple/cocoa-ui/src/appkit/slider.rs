//! The `AppKit` slider: an `NSSlider` driven continuously.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSSlider` subclass, forwards to
//! `NSSlider`'s own implementation of the methods it overrides, and calls
//! `objc2` bindings marked unsafe because `AppKit` control and animation
//! APIs are main-thread only — which the `MainThreadOnly` thread kind and
//! [`MainThreadMarker`] constructor guarantee.

use std::fmt;

use objc2::rc::{Retained, Weak};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSAnimatablePropertyContainer, NSColor, NSControl, NSControlSize, NSSlider,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSObjectProtocol, NSString};

use crate::ActionTarget;
use crate::slider::{ControlSize, ValueAnimation};

/// A continuous `NSSlider` used as a value leaf.
pub struct SliderIvars {}

impl fmt::Debug for SliderIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SliderIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSSlider`'s designated initializer is `initWithFrame:`, which
    // `Slider::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSSlider))]
    #[name = "CocoaUiSlider"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SliderIvars]
    #[derive(Debug)]
    /// A continuous `NSSlider` with an installable value-change action.
    pub struct Slider;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSSlider` subclass.
    unsafe impl NSObjectProtocol for Slider {}
);

impl Slider {
    /// A slider on the range `0..=1`, reporting every intermediate value.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SliderIvars {});
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setContinuous(true);
        this
    }

    /// The value the thumb currently reports.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.doubleValue()
    }

    /// Sets the value, transitioning with `animation` when given.
    ///
    /// An animated change runs the assignment inside an
    /// `NSAnimationContext` group of the animation's duration — and cubic
    /// timing function for [`ValueAnimation::Bezier`] — with implicit
    /// animations enabled.
    pub fn set_value(&self, value: f64, animation: Option<ValueAnimation>) {
        match animation {
            None => self.setDoubleValue(value),
            Some(animation) => {
                let control_points = match animation {
                    ValueAnimation::Duration(_) => None,
                    ValueAnimation::Bezier { control_points, .. } => Some(control_points),
                };
                let this = Weak::new(self);
                crate::core_animation::animate(animation.duration(), control_points, move || {
                    if let Some(slider) = this.load() {
                        slider.animator().setDoubleValue(value);
                    }
                });
            }
        }
    }

    /// Sets the inclusive range the thumb travels over.
    pub fn set_range(&self, start: f64, end: f64) {
        self.setMinValue(start);
        self.setMaxValue(end);
    }

    /// Moves the thumb in increments of `step` from the start of the current
    /// range, or continuously for `None`: one tick mark per increment, the
    /// thumb held to them. Set the range first.
    pub fn set_step(&self, step: Option<f64>) {
        let ticks = step.map_or(0, |step| {
            Self::tick_marks(self.minValue(), self.maxValue(), step)
        });
        self.setNumberOfTickMarks(ticks);
        self.setAllowsTickMarkValuesOnly(step.is_some());
    }

    /// The tick marks `step` makes on `start..=end`, the two ends included.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a slider has a handful of steps, far below `isize::MAX`"
    )]
    fn tick_marks(start: f64, end: f64, step: f64) -> isize {
        ((end - start) / step).round() as isize + 1
    }

    /// Whether the slider responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// The colour the filled part of the track draws with.
    pub fn set_track_tint(&self, color: &NSColor) {
        self.setTrackFillColor(Some(color));
    }

    /// How large the control is drawn.
    pub fn set_control_size(&self, size: ControlSize) {
        let size = match size {
            ControlSize::Mini => NSControlSize::Mini,
            ControlSize::Small => NSControlSize::Small,
            ControlSize::Regular => NSControlSize::Regular,
            ControlSize::Large => NSControlSize::Large,
        };
        self.setControlSize(size);
    }

    /// Calls `handler` with the new value each time the user moves the
    /// thumb. The returned target must be kept for as long as the control
    /// should respond; dropping it detaches the target.
    pub fn install_action(&self, handler: impl Fn(f64) + 'static) -> ActionTarget {
        let control: &NSControl = self;
        let this = Weak::new(self);
        ActionTarget::new(control, move |_mtm| {
            if let Some(slider) = this.load() {
                handler(slider.value());
            }
        })
    }

    /// The track's intrinsic height — what a measure pass reports.
    #[must_use]
    pub fn intrinsic_height(&self) -> f64 {
        self.intrinsicContentSize().height
    }

    /// Names the control to a screen reader and, on macOS, as its tooltip;
    /// `None` leaves it unnamed. Setting a label marks the view an
    /// accessibility element.
    pub fn set_accessibility_label(&self, label: Option<&str>) {
        self.setAccessibilityElement(label.is_some());
        self.setAccessibilityLabel(label.map(NSString::from_str).as_deref());
        self.setToolTip(label.map(NSString::from_str).as_deref());
    }
}

#[cfg(test)]
mod tests {
    use super::Slider;

    #[test]
    fn a_step_marks_both_ends_of_the_range() {
        assert_eq!(Slider::tick_marks(0.0, 10.0, 2.5), 5);
        assert_eq!(Slider::tick_marks(-1.0, 1.0, 0.5), 5);
        assert_eq!(Slider::tick_marks(0.0, 1.0, 1.0), 2);
    }
}
