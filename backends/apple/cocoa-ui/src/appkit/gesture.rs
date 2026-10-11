//! Gesture recognizers: a Rust closure as an `NSGestureRecognizer`'s target.
//!
//! Each constructor builds a concrete recognizer, points it at a target
//! object owning the handler closure, and attaches it to a view. The
//! returned [`GestureAttachment`] owns the target and the recognizer;
//! dropping it detaches the target so a recognizer that outlives the
//! attachment cannot fire into freed state.
//!
//! `AppKit` does not retain a recognizer's target — `target` is weak — so
//! the attachment is the only owner. Keep it for as long as the recognizer
//! should respond, and drop it when the view leaves the hierarchy.
//!
//! # Safety
//!
//! The `unsafe` here defines the target class and drives the target/action
//! pair `NSGestureRecognizer` declares. The selector has the signature
//! `AppKit` sends, and every call is a main-thread call: the class is
//! `MainThreadOnly` and the recognizers are attached to views, which are
//! main-thread objects.

use std::fmt;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::NSObject;
use objc2::sel;
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSClickGestureRecognizer, NSGestureRecognizer, NSGestureRecognizerState,
    NSMagnificationGestureRecognizer, NSPanGestureRecognizer, NSPressGestureRecognizer,
    NSRotationGestureRecognizer, NSView,
};
use objc2_foundation::NSObjectProtocol;

use crate::callback::guarded;
use crate::gesture::{ButtonMask, GestureState};

fn state_of(recognizer: &NSGestureRecognizer) -> GestureState {
    match recognizer.state() {
        NSGestureRecognizerState::Began => GestureState::Began,
        NSGestureRecognizerState::Changed => GestureState::Changed,
        NSGestureRecognizerState::Ended => GestureState::Ended,
        NSGestureRecognizerState::Cancelled => GestureState::Cancelled,
        NSGestureRecognizerState::Failed => GestureState::Failed,
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
        fn invoke(&self, recognizer: &NSGestureRecognizer) {
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

/// A recognizer attached to a view, together with the target that owns its
/// handler.
///
/// Keep the value for as long as the recognizer should respond — it lives in
/// the leaf's `KeepAlive`. Dropping it detaches the target from the
/// recognizer; it does not remove the recognizer from its view.
#[must_use = "dropping the attachment detaches the recognizer's target"]
#[derive(Debug)]
pub struct GestureAttachment {
    target: Retained<Target>,
    recognizer: Weak<NSGestureRecognizer>,
}

impl GestureAttachment {
    /// The recognizer this attachment owns.
    #[must_use]
    pub fn recognizer(&self) -> Option<Retained<NSGestureRecognizer>> {
        self.recognizer.load()
    }
}

impl Drop for GestureAttachment {
    fn drop(&mut self) {
        let Some(recognizer) = self.recognizer.load() else {
            return;
        };
        let ours = recognizer.target().is_some_and(|target| {
            Retained::as_ptr(&target) == Retained::as_ptr(&self.target).cast()
        });
        if ours {
            // SAFETY: clearing the weak target/action pair, only while it is
            // still this target.
            unsafe {
                recognizer.setTarget(None);
                recognizer.setAction(None);
            }
        }
    }
}

fn attach<R>(
    view: &NSView,
    recognizer: Retained<R>,
    handler: Rc<dyn Fn(GestureState)>,
) -> GestureAttachment
where
    R: ClassType<Super = NSGestureRecognizer> + MainThreadOnly + 'static,
{
    let mtm = recognizer.mtm();
    let target = Target::new(mtm, handler);
    let recognizer: Retained<NSGestureRecognizer> = recognizer.into_super();
    // SAFETY: `setTarget`/`setAction` are the documented weak-reference pair;
    // the attachment owns the target the recognizer points at.
    unsafe {
        recognizer.setTarget(Some(&*target));
        recognizer.setAction(Some(sel!(invoke:)));
    }
    view.addGestureRecognizer(&recognizer);
    GestureAttachment {
        target,
        recognizer: Weak::new(&recognizer),
    }
}

/// A click recognizer on `view`: `clicks` consecutive presses (at least one)
/// from a pointer button in `buttons`.
///
/// `NSClickGestureRecognizer.buttonMask` follows the same DOM `buttons` bit
/// order [`ButtonMask`] uses, so the mask applies verbatim.
pub fn click(
    view: &NSView,
    clicks: usize,
    buttons: ButtonMask,
    handler: impl Fn(GestureState) + 'static,
) -> GestureAttachment {
    let mtm = view.mtm();
    let recognizer = NSClickGestureRecognizer::new(mtm);
    recognizer.setNumberOfClicksRequired(clicks.max(1).cast_signed());
    recognizer.setButtonMask(usize::from(buttons.bits()));
    attach(view, recognizer, Rc::new(handler))
}

/// A press recognizer on `view`: a press of at least `minimum_seconds`
/// from a pointer button in `buttons`.
pub fn press(
    view: &NSView,
    minimum_seconds: f64,
    buttons: ButtonMask,
    handler: impl Fn(GestureState) + 'static,
) -> GestureAttachment {
    let mtm = view.mtm();
    let recognizer = NSPressGestureRecognizer::new(mtm);
    recognizer.setMinimumPressDuration(minimum_seconds);
    recognizer.setButtonMask(usize::from(buttons.bits()));
    attach(view, recognizer, Rc::new(handler))
}

/// A pan recognizer on `view`, from a pointer button in `buttons`.
pub fn pan(
    view: &NSView,
    buttons: ButtonMask,
    handler: impl Fn(GestureState) + 'static,
) -> GestureAttachment {
    let mtm = view.mtm();
    let recognizer = NSPanGestureRecognizer::new(mtm);
    recognizer.setButtonMask(usize::from(buttons.bits()));
    attach(view, recognizer, Rc::new(handler))
}

/// A magnification recognizer on `view`.
pub fn magnification(view: &NSView, handler: impl Fn(GestureState) + 'static) -> GestureAttachment {
    let mtm = view.mtm();
    attach(
        view,
        NSMagnificationGestureRecognizer::new(mtm),
        Rc::new(handler),
    )
}

/// A rotation recognizer on `view`.
pub fn rotation(view: &NSView, handler: impl Fn(GestureState) + 'static) -> GestureAttachment {
    let mtm = view.mtm();
    attach(
        view,
        NSRotationGestureRecognizer::new(mtm),
        Rc::new(handler),
    )
}
