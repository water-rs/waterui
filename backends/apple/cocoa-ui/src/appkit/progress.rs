//! The `AppKit` progress indicator: an `NSProgressIndicator` in bar or
//! spinning style.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSProgressIndicator` subclass and calls
//! `objc2` bindings marked unsafe because `AppKit` control APIs are
//! main-thread only — which the `MainThreadOnly` thread kind and
//! [`MainThreadMarker`] constructor guarantee.

use std::fmt;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSControlSize, NSProgressIndicator, NSProgressIndicatorStyle};
use objc2_core_foundation::CGRect;
use objc2_foundation::NSObjectProtocol;

use crate::geometry::Size;
use crate::progress::ProgressVariant;

/// An `NSProgressIndicator` used as a value leaf.
pub struct ProgressIvars {}

impl fmt::Debug for ProgressIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProgressIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSProgressIndicator`'s designated initializer is
    // `initWithFrame:`, which `Progress::new` calls, and the class does not
    // implement `Drop`.
    #[unsafe(super(NSProgressIndicator))]
    #[name = "CocoaUiProgress"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ProgressIvars]
    #[derive(Debug)]
    /// An `NSProgressIndicator` with a switchable variant.
    pub struct Progress;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSProgressIndicator`
    // subclass.
    unsafe impl NSObjectProtocol for Progress {}
);

impl Progress {
    /// A determinate linear indicator on the range `0..=1`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ProgressIvars {});
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.set_range(0.0, 1.0);
        this.setDisplayedWhenStopped(true);
        this
    }

    /// Switches between the bar and the spinning style.
    pub fn set_variant(&self, variant: ProgressVariant) {
        match variant {
            ProgressVariant::Linear => self.setStyle(NSProgressIndicatorStyle::Bar),
            ProgressVariant::Circular => self.setStyle(NSProgressIndicatorStyle::Spinning),
        }
    }

    /// How large the control is drawn — `NSControl.controlSize`.
    pub fn set_control_size(&self, size: crate::slider::ControlSize) {
        let size = match size {
            crate::slider::ControlSize::Mini => NSControlSize::Mini,
            crate::slider::ControlSize::Small => NSControlSize::Small,
            crate::slider::ControlSize::Regular => NSControlSize::Regular,
            crate::slider::ControlSize::Large => NSControlSize::Large,
        };
        self.setControlSize(size);
    }

    /// The value the indicator reports.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.doubleValue()
    }

    /// Sets the value the indicator reports.
    pub fn set_value(&self, value: f64) {
        self.setDoubleValue(value);
    }

    /// Sets the inclusive range the value travels over.
    pub fn set_range(&self, start: f64, end: f64) {
        self.setMinValue(start);
        self.setMaxValue(end);
    }

    /// Whether the indicator animates instead of reporting a value.
    pub fn set_indeterminate(&self, indeterminate: bool) {
        self.setIndeterminate(indeterminate);
    }

    /// Starts the indeterminate animation.
    pub fn start_animation(&self) {
        // SAFETY: `startAnimation:` spins the receiver on the main thread;
        // `nil` sender is the documented "no target" call.
        unsafe { self.startAnimation(None) };
    }

    /// Stops the indeterminate animation.
    pub fn stop_animation(&self) {
        // SAFETY: `stopAnimation:` halts the receiver's animation; `nil`
        // sender is the documented "no target" call.
        unsafe { self.stopAnimation(None) };
    }

    /// The control's intrinsic size — what a measure pass reports.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let size = self.intrinsicContentSize();
        Size::new(size.width, size.height)
    }
}
