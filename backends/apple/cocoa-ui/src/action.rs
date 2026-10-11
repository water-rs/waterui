//! Target-action: a closure installed as a control's target.
//!
//! Both frameworks reference a control's target without retaining it —
//! `NSControl.target` is assign, `UIControl` copies only the pointer — so
//! [`ActionTarget`] is the owner: the leaf keeps it for as long as the
//! control should respond, and dropping it detaches the target.
//!
//! The closure must not strongly capture the control (use
//! `objc2::rc::Weak`): the control would then own itself through its target.
//!
//! # Safety
//!
//! The `unsafe` here defines the target's class and installs and detaches
//! it. The selector has the signature `AppKit`/`UIKit` send, and every call
//! is a main-thread call — the class is `MainThreadOnly`, so the closure
//! always sees a live `MainThreadMarker`.

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObject};
use objc2::sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::NSObjectProtocol;

use crate::callback::guarded;

#[cfg(target_os = "macos")]
use objc2_app_kit::NSControl;
#[cfg(target_os = "ios")]
use objc2_ui_kit::{UIControl, UIControlEvents};

/// Which control events fire the action (`UIKit` only).
#[cfg(target_os = "ios")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlEvents(pub usize);

#[cfg(target_os = "ios")]
impl ControlEvents {
    /// A touch went down inside the control.
    pub const TOUCH_DOWN: Self = Self(1 << 0);
    /// A drag entered the control's bounds.
    pub const TOUCH_DRAG_ENTER: Self = Self(1 << 4);
    /// A drag left the control's bounds.
    pub const TOUCH_DRAG_EXIT: Self = Self(1 << 5);
    /// A touch lifted inside the control.
    pub const TOUCH_UP_INSIDE: Self = Self(1 << 6);
    /// A touch lifted outside the control.
    pub const TOUCH_UP_OUTSIDE: Self = Self(1 << 7);
    /// The system cancelled the touch.
    pub const TOUCH_CANCEL: Self = Self(1 << 8);
    /// The control's value changed (sliders, steppers, fields).
    pub const VALUE_CHANGED: Self = Self(1 << 12);
    /// The control's primary action (a button press).
    pub const PRIMARY_ACTION_TRIGGERED: Self = Self(1 << 13);
    /// A text field's contents changed while editing.
    pub const EDITING_CHANGED: Self = Self(1 << 17);
    /// Editing ended because the field's Return key was pressed.
    pub const EDITING_DID_END_ON_EXIT: Self = Self(1 << 19);
    /// Every event.
    pub const ALL: Self = Self(0x00FF_FFFF);
}

#[cfg(target_os = "ios")]
impl core::ops::BitOr for ControlEvents {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[cfg(target_os = "ios")]
impl ControlEvents {
    const fn native(self) -> UIControlEvents {
        UIControlEvents(self.0)
    }
}

struct TargetIvars {
    handler: Box<dyn Fn(MainThreadMarker)>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the class holds a
    // main-thread closure and does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiActionTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    struct Target;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        /// `IBAction`: the frameworks send `fire:` with the control as sender.
        #[unsafe(method(fire:))]
        fn fire(&self, _sender: &AnyObject) {
            guarded("CocoaUiActionTarget fire:", || {
                (self.ivars().handler)(self.mtm());
            });
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, handler: Box<dyn Fn(MainThreadMarker)>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { handler });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target").finish_non_exhaustive()
    }
}

/// A closure installed as a control's action target; owns the target object.
///
/// Keep the value for as long as the control should respond — it lives in
/// the leaf's `KeepAlive`. Dropping it detaches the target from the control.
#[must_use = "dropping the target detaches it"]
#[derive(Debug)]
pub struct ActionTarget {
    target: Retained<Target>,
    #[cfg(target_os = "macos")]
    control: Weak<NSControl>,
    #[cfg(target_os = "ios")]
    control: Weak<UIControl>,
    #[cfg(target_os = "ios")]
    events: ControlEvents,
}

impl ActionTarget {
    /// Calls `handler` on the main thread each time `control` performs its
    /// action (`NSControl`'s `sendAction:`).
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    #[cfg(target_os = "macos")]
    pub fn new(control: &NSControl, handler: impl Fn(MainThreadMarker) + 'static) -> Self {
        let mtm = MainThreadMarker::new().expect("ActionTarget::new on a non-main thread");
        let target = Target::new(mtm, Box::new(handler));
        // SAFETY: `target`/`action` are the documented weak-reference pair;
        // `ActionTarget` owns the target object the control points at.
        unsafe {
            control.setTarget(Some(&*target));
            control.setAction(Some(sel!(fire:)));
        }
        Self {
            target,
            control: Weak::new(control),
        }
    }

    /// Calls `handler` on the main thread each time `control` fires one of
    /// `events`.
    ///
    /// # Panics
    ///
    /// When not called on the main thread.
    #[cfg(target_os = "ios")]
    pub fn new(
        control: &UIControl,
        events: ControlEvents,
        handler: impl Fn(MainThreadMarker) + 'static,
    ) -> Self {
        let mtm = MainThreadMarker::new().expect("ActionTarget::new on a non-main thread");
        let target = Target::new(mtm, Box::new(handler));
        // SAFETY: UIKit stores the target unretained; `ActionTarget` owns it
        // and `Drop` removes it while the control may still be alive.
        unsafe {
            control.addTarget_action_forControlEvents(Some(&*target), sel!(fire:), events.native());
        }
        Self {
            target,
            control: Weak::new(control),
            events,
        }
    }
}

impl Drop for ActionTarget {
    fn drop(&mut self) {
        let Some(control) = self.control.load() else {
            return;
        };
        #[cfg(target_os = "macos")]
        {
            let ours = control.target().is_some_and(|target| {
                Retained::as_ptr(&target) == Retained::as_ptr(&self.target).cast()
            });
            if ours {
                // SAFETY: clearing the weak target/action pair, only when it
                // is still this target — a later `ActionTarget` may have
                // replaced it.
                unsafe {
                    control.setTarget(None);
                    control.setAction(None);
                }
            }
        }
        #[cfg(target_os = "ios")]
        // SAFETY: `removeTarget:action:forControlEvents:` is the documented
        // inverse of the `addTarget` in `new`.
        unsafe {
            control.removeTarget_action_forControlEvents(
                Some(&*self.target),
                Some(sel!(fire:)),
                self.events.native(),
            );
        }
    }
}
