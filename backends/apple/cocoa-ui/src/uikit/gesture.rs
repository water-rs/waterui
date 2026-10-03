//! Gesture recognizers: a Rust closure as a `UIGestureRecognizer`'s target.
//!
//! Each constructor builds a concrete recognizer, points it at a target
//! object owning the handler closure, and attaches it to a view. The
//! returned [`GestureAttachment`] owns the target, the recognizer and any
//! delegate the configuration needed; dropping it detaches the target so a
//! recognizer that outlives the attachment cannot fire into freed state.
//!
//! `UIKit` does not retain a recognizer's target, so the attachment is the
//! only owner. Keep it for as long as the recognizer should respond.
//!
//! # Safety
//!
//! The `unsafe` here defines the target and delegate classes and drives the
//! target/action pair `UIGestureRecognizer` declares. The selector has the
//! signature `UIKit` sends, and every call is a main-thread call: the
//! classes are `MainThreadOnly` and the recognizers attach to views.

use std::fmt;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::sel;
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::NSObjectProtocol;
use objc2_ui_kit::{
    UIEvent, UIGestureRecognizer, UIGestureRecognizerDelegate, UIGestureRecognizerState,
    UILongPressGestureRecognizer, UIPanGestureRecognizer, UIPinchGestureRecognizer,
    UIRotationGestureRecognizer, UITapGestureRecognizer, UIView,
};

use crate::callback::guarded;
use crate::gesture::{ButtonMask, GestureState};

fn state_of(recognizer: &UIGestureRecognizer) -> GestureState {
    match recognizer.state() {
        UIGestureRecognizerState::Began => GestureState::Began,
        UIGestureRecognizerState::Changed => GestureState::Changed,
        UIGestureRecognizerState::Ended => GestureState::Ended,
        UIGestureRecognizerState::Cancelled => GestureState::Cancelled,
        UIGestureRecognizerState::Failed => GestureState::Failed,
        _ => GestureState::Possible,
    }
}

struct TargetIvars {
    handler: Rc<dyn Fn(GestureState)>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the class holds a
    // main-thread closure and does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiGestureTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    struct Target;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        /// The action every recognizer this module creates sends: the
        /// recognizer, whose state the handler reads.
        #[unsafe(method(invoke:))]
        fn invoke(&self, recognizer: &UIGestureRecognizer) {
            guarded("CocoaUiGestureTarget invoke:", || {
                (self.ivars().handler)(state_of(recognizer));
            });
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, handler: Rc<dyn Fn(GestureState)>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { handler });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target").finish_non_exhaustive()
    }
}

struct FilterIvars {
    buttons: ButtonMask,
}

define_class!(
    /// Rejects events whose `UIEvent.buttonMask` does not intersect the
    /// gesture's button mask, for recognizers that cannot carry the mask
    /// themselves: `UIGestureRecognizer.buttonMask` is read-only and only
    /// `UITapGestureRecognizer` exposes `buttonMaskRequired`. Touches report
    /// `.primary`, so a primary mask still accepts a finger.
    ///
    // SAFETY: `NSObject` has no subclassing requirements; the class holds a
    // mask and does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiGestureButtonFilter"]
    #[thread_kind = MainThreadOnly]
    #[ivars = FilterIvars]
    struct ButtonFilter;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for ButtonFilter {}

    // SAFETY: `gestureRecognizer:shouldReceiveEvent:` has the signature
    // `UIGestureRecognizerDelegate` declares.
    unsafe impl UIGestureRecognizerDelegate for ButtonFilter {
        #[unsafe(method(gestureRecognizer:shouldReceiveEvent:))]
        fn should_receive_event(&self, _recognizer: &UIGestureRecognizer, event: &UIEvent) -> bool {
            guarded("CocoaUiGestureButtonFilter shouldReceiveEvent:", || {
                ButtonMask::from_bits(u8::try_from(event.buttonMask().0 & 0xff).unwrap_or_default())
                    .intersects(self.ivars().buttons)
            })
        }
    }
);

impl ButtonFilter {
    fn new(mtm: MainThreadMarker, buttons: ButtonMask) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FilterIvars { buttons });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

impl fmt::Debug for ButtonFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ButtonFilter").finish_non_exhaustive()
    }
}

/// A recognizer attached to a view, together with the objects that own its
/// handler and button filter.
///
/// Keep the value for as long as the recognizer should respond — it lives in
/// the leaf's `KeepAlive`. Dropping it detaches the target and delegate from
/// the recognizer; it does not remove the recognizer from its view.
#[must_use = "dropping the attachment detaches the recognizer's target"]
#[derive(Debug)]
pub struct GestureAttachment {
    target: Retained<Target>,
    recognizer: Weak<UIGestureRecognizer>,
    /// The event-button filter, where the recognizer cannot carry the mask
    /// itself; kept only for ownership.
    #[allow(dead_code)]
    filter: Option<Retained<ButtonFilter>>,
}

impl GestureAttachment {
    /// The recognizer this attachment owns.
    #[must_use]
    pub fn recognizer(&self) -> Option<Retained<UIGestureRecognizer>> {
        self.recognizer.load()
    }
}

impl Drop for GestureAttachment {
    fn drop(&mut self) {
        let Some(recognizer) = self.recognizer.load() else {
            return;
        };
        if let Some(filter) = &self.filter {
            let ours = recognizer.delegate().is_some_and(|delegate| {
                Retained::as_ptr(&delegate) == Retained::as_ptr(filter).cast()
            });
            if ours {
                // Clearing the delegate this attachment installed.
                recognizer.setDelegate(None);
            }
        }
        // SAFETY: `removeTarget:action:` is the documented inverse of the
        // `addTarget:action:` in `attach`.
        unsafe {
            recognizer.removeTarget_action(Some(&*self.target), Some(sel!(invoke:)));
        }
    }
}

fn attach<R>(
    view: &UIView,
    recognizer: Retained<R>,
    buttons: Option<ButtonMask>,
    handler: Rc<dyn Fn(GestureState)>,
) -> GestureAttachment
where
    R: ClassType<Super = UIGestureRecognizer> + MainThreadOnly + 'static,
{
    let mtm = recognizer.mtm();
    let target = Target::new(mtm, handler);
    let recognizer: Retained<UIGestureRecognizer> = recognizer.into_super();
    // SAFETY: UIKit stores the target unretained; the attachment owns it and
    // `Drop` removes it while the recognizer may still be alive.
    unsafe { recognizer.addTarget_action(&target, sel!(invoke:)) };
    let filter = buttons.map(|buttons| {
        let filter = ButtonFilter::new(mtm, buttons);
        // Installing the filter as the recognizer's delegate; the
        // attachment owns it.
        recognizer.setDelegate(Some(ProtocolObject::from_ref(&*filter)));
        filter
    });
    view.addGestureRecognizer(&recognizer);
    GestureAttachment {
        target,
        recognizer: Weak::new(&recognizer),
        filter,
    }
}

/// A tap recognizer on `view`: `taps` consecutive taps (at least one) from a
/// pointer button in `buttons`.
///
/// `buttonMaskRequired` follows the same DOM `buttons` bit order
/// [`ButtonMask`] uses, so the mask applies verbatim.
pub fn tap(
    view: &UIView,
    taps: usize,
    buttons: ButtonMask,
    handler: impl Fn(GestureState) + 'static,
) -> GestureAttachment {
    let mtm = view.mtm();
    let recognizer = UITapGestureRecognizer::new(mtm);
    recognizer.setNumberOfTapsRequired(taps.max(1));
    recognizer.setButtonMaskRequired(objc2_ui_kit::UIEventButtonMask(buttons.bits() as isize));
    attach(view, recognizer, None, Rc::new(handler))
}

/// A long-press recognizer on `view`: a press of at least
/// `minimum_seconds` from a pointer button in `buttons`.
pub fn long_press(
    view: &UIView,
    minimum_seconds: f64,
    buttons: ButtonMask,
    handler: impl Fn(GestureState) + 'static,
) -> GestureAttachment {
    let mtm = view.mtm();
    let recognizer = UILongPressGestureRecognizer::new(mtm);
    recognizer.setMinimumPressDuration(minimum_seconds);
    attach(view, recognizer, Some(buttons), Rc::new(handler))
}

/// A pan recognizer on `view`, from a pointer button in `buttons`.
pub fn pan(
    view: &UIView,
    buttons: ButtonMask,
    handler: impl Fn(GestureState) + 'static,
) -> GestureAttachment {
    let mtm = view.mtm();
    attach(
        view,
        UIPanGestureRecognizer::new(mtm),
        Some(buttons),
        Rc::new(handler),
    )
}

/// A pinch recognizer on `view`.
pub fn pinch(view: &UIView, handler: impl Fn(GestureState) + 'static) -> GestureAttachment {
    let mtm = view.mtm();
    attach(
        view,
        UIPinchGestureRecognizer::new(mtm),
        None,
        Rc::new(handler),
    )
}

/// A rotation recognizer on `view`.
pub fn rotation(view: &UIView, handler: impl Fn(GestureState) + 'static) -> GestureAttachment {
    let mtm = view.mtm();
    attach(
        view,
        UIRotationGestureRecognizer::new(mtm),
        None,
        Rc::new(handler),
    )
}
