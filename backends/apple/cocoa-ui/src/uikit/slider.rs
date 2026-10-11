//! The `UIKit` slider: a `UISlider` driving a continuous value.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UISlider` subclass and calls `objc2`
//! bindings marked unsafe because `UIKit` control APIs are main-thread
//! only — which the `MainThreadOnly` thread kind and [`MainThreadMarker`]
//! constructor guarantee.

use std::fmt;

use objc2::rc::{Retained, Weak};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CGRect;
use objc2_foundation::NSObjectProtocol;
use objc2_ui_kit::{NSObjectUIAccessibility, UIControl, UISlider};

use crate::ActionTarget;
use crate::action::ControlEvents;
use crate::slider::{ControlSize, ValueAnimation};

/// A continuous `UISlider` used as a value leaf.
pub struct SliderIvars {}

impl fmt::Debug for SliderIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SliderIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `UISlider`'s designated initializer is `initWithFrame:`, which
    // `Slider::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UISlider))]
    #[name = "CocoaUiSlider"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SliderIvars]
    #[derive(Debug)]
    /// A continuous `UISlider` with an installable value-change action.
    pub struct Slider;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UISlider` subclass.
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
        let this: &UISlider = self;
        f64::from(this.value())
    }

    /// Sets the value, sliding the thumb to it when `animation` is given.
    ///
    /// `UISlider` owns the transition: any animation maps to its `animated:`
    /// argument and its parameters are ignored.
    pub fn set_value(&self, value: f64, animation: Option<ValueAnimation>) {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "UISlider values are f32; the range and value are display magnitudes"
        )]
        let value = value as f32;
        self.setValue_animated(value, animation.is_some());
    }

    /// Sets the inclusive range the thumb travels over.
    pub fn set_range(&self, start: f64, end: f64) {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "UISlider range bounds are f32; slider ranges are display magnitudes"
        )]
        self.setMinimumValue(start as f32);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "UISlider range bounds are f32; slider ranges are display magnitudes"
        )]
        self.setMaximumValue(end as f32);
    }

    /// Whether the slider responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// How large the control is drawn.
    ///
    /// `UIKit` has no slider size classes: the value is ignored.
    pub const fn set_control_size(&self, _size: ControlSize) {}

    /// Calls `handler` with the new value each time the user moves the
    /// thumb. The returned target must be kept for as long as the control
    /// should respond; dropping it detaches the target.
    pub fn install_action(&self, handler: impl Fn(f64) + 'static) -> ActionTarget {
        let control: &UIControl = self;
        let this = Weak::new(self);
        ActionTarget::new(control, ControlEvents::VALUE_CHANGED, move |_mtm| {
            if let Some(slider) = this.load() {
                handler(slider.value());
            }
        })
    }

    /// The track's intrinsic height — the height `UISlider` itself
    /// reports through `intrinsicContentSize`.
    #[must_use]
    pub fn intrinsic_height(&self) -> f64 {
        self.intrinsicContentSize().height
    }

    /// Names the control to a screen reader; `None` leaves it unnamed.
    /// Setting a label marks the view an accessibility element.
    pub fn set_accessibility_label(&self, label: Option<&str>) {
        let mtm = MainThreadMarker::from(self);
        self.setIsAccessibilityElement(label.is_some(), mtm);
        self.setAccessibilityLabel(
            label.map(objc2_foundation::NSString::from_str).as_deref(),
            mtm,
        );
    }
}
