//! The `UIKit` color view: a `UIView` painting one fill color.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIView` subclass and calls `objc2` bindings
//! marked unsafe because `UIKit` view APIs are main-thread only — which the
//! `MainThreadOnly` thread kind and [`MainThreadMarker`] constructor
//! guarantee.

use std::fmt;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CGRect;
use objc2_foundation::NSObjectProtocol;
use objc2_ui_kit::{UIColor, UIView};

/// A [`ColorView`] carries no state beyond its background color.
pub struct ColorViewIvars {}

impl fmt::Debug for ColorViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorViewIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `UIView`'s designated initializer is `initWithFrame:`, which
    // `ColorView::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UIView))]
    #[name = "CocoaUiColorView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ColorViewIvars]
    #[derive(Debug)]
    /// A `UIView` whose whole content is a background color.
    pub struct ColorView;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIView` subclass.
    unsafe impl NSObjectProtocol for ColorView {}
);

impl ColorView {
    /// A color fill view with no color set yet.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ColorViewIvars {});
        // SAFETY: `initWithFrame:` is `UIView`'s designated initializer.
        unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] }
    }

    /// Fills the view with `color`; `None` clears the fill.
    pub fn set_color(&self, color: Option<&UIColor>) {
        self.setBackgroundColor(color);
    }
}
