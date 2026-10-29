//! A live reading of where the keyboard's top edge is.
//!
//! The window's `keyboardLayoutGuide` follows the keyboard through its
//! show, hide and resize animations, but a layout guide has no presentation
//! layer to sample. A [`KeyboardBand`] pins a one-point view to the guide's
//! top edge: while `UIKit` animates the guide, the tracker's presentation
//! layer carries the in-flight position, which a frame clock can read once a
//! frame — the per-frame inset a self-drawn view lays out against.
//!
//! # Safety
//!
//! The `unsafe` here reads the tracker layer's presentation layer, a plain
//! property read of the layer's in-flight state.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_ui_kit::{NSLayoutConstraint, UIKeyboardWillChangeFrameNotification, UIView};

use crate::notification::{NotificationName, NotificationObserver, observe_with_notification};

/// A hidden view tracking the top edge of the keyboard layout guide.
#[derive(Debug)]
pub struct KeyboardBand {
    tracker: Retained<UIView>,
}

impl KeyboardBand {
    /// Pins a tracker view to `parent`'s keyboard layout guide.
    ///
    /// Until the keyboard shows, the guide's top sits at the bottom edge of
    /// `parent`'s safe area.
    #[must_use]
    pub fn attach(mtm: MainThreadMarker, parent: &UIView) -> Self {
        let tracker = UIView::initWithFrame(
            UIView::alloc(mtm),
            CGRect::new(CGPoint::ZERO, CGSize::new(1.0, 1.0)),
        );
        tracker.setUserInteractionEnabled(false);
        tracker.setAlpha(0.0);
        tracker.setTranslatesAutoresizingMaskIntoConstraints(false);
        parent.addSubview(&tracker);
        let guide = parent.keyboardLayoutGuide();
        let constraints = [
            tracker
                .topAnchor()
                .constraintEqualToAnchor(&guide.topAnchor()),
            tracker
                .leadingAnchor()
                .constraintEqualToAnchor(&parent.leadingAnchor()),
            tracker.widthAnchor().constraintEqualToConstant(1.0),
            tracker.heightAnchor().constraintEqualToConstant(1.0),
        ];
        let constraints = objc2_foundation::NSArray::from_retained_slice(&constraints);
        NSLayoutConstraint::activateConstraints(&constraints, mtm);
        Self { tracker }
    }

    /// The guide's top edge in the parent's coordinate space, as it is
    /// drawn right now: mid-animation it is the in-flight position.
    #[must_use]
    pub fn top(&self) -> f64 {
        let layer = self.tracker.layer();
        // SAFETY: `presentationLayer` only reads the layer's in-flight state.
        let presented = unsafe { layer.presentationLayer() };
        presented
            .map_or_else(|| layer.frame(), |presented| presented.frame())
            .origin
            .y
    }

    /// Calls `handler` with the animation's duration, in seconds, every time
    /// the keyboard begins to show, hide or resize, until the returned guard
    /// is dropped.
    ///
    /// A panic in `handler` aborts the process (see the
    /// [crate documentation](crate)).
    pub fn observe_changes(
        &self,
        mtm: MainThreadMarker,
        handler: impl Fn(f64) + 'static,
    ) -> NotificationObserver {
        let tracker = objc2::rc::Weak::new(&*self.tracker);
        observe_with_notification(
            mtm,
            // SAFETY: `UIKit` exports the notification name as a constant for
            // the process's lifetime.
            &NotificationName::framework(unsafe { UIKeyboardWillChangeFrameNotification }),
            move |notification| {
                let Some(window) = tracker.load().and_then(|view| view.window()) else {
                    return;
                };
                handler(super::keyboard::change(notification, &window).duration);
            },
        )
    }
}
