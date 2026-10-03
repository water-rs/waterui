//! Keyboard focus: which view in a window is the first responder.
//!
//! A [`FocusTarget`] marks the one view inside a rendered subtree that takes
//! and reports keyboard focus. Controls that can hold focus install one —
//! [`install`] records it on the view — and a wrapper that manages focus
//! finds it by walking the subtree with [`targets_in`].
//!
//! Focus changes are reported two ways: imperative calls
//! ([`become_first_responder`], [`resign_first_responder`]) ask the platform
//! to move focus, while the target's own observation — the framework's
//! editing-begin/editing-end notifications, filtered to the target's view —
//! tells a subscriber when the platform moved focus itself.
//!
//! # Safety
//!
//! The `unsafe` here reads `AppKit`/`UIKit` first-responder state on views
//! the caller guarantees are alive; all are main-thread reads. The registry
//! is `thread_local`, so it can only be touched on the thread it was created
//! on — the main thread, where every view call here happens.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr;
use std::rc::{Rc, Weak};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;

use crate::PlatformView;
use crate::notification::{NotificationName, NotificationObserver, observe_object};

type FocusHandler = Rc<dyn Fn(bool)>;

/// The per-target state: its subscribers, and the notification
/// registrations that feed them.
struct Inner {
    /// The view whose focus this target reports.
    view: Retained<PlatformView>,
    /// The subscribers [`FocusTarget::on_change`] registered, by id.
    observers: RefCell<Vec<(u64, FocusHandler)>>,
    /// The id the next subscriber takes.
    next: Cell<u64>,
    /// The editing notifications feeding `observers`, kept registered for
    /// the target's life. A cell because each observer's closure captures a
    /// `Weak` to this `Inner`, so they can only register after it exists.
    notifications: RefCell<Vec<NotificationObserver>>,
}

impl core::fmt::Debug for Inner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FocusTargetInner")
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

impl Inner {
    /// Tells every subscriber the platform gave or took `focus`.
    fn emit(&self, focus: bool) {
        for (_, handler) in &*self.observers.borrow() {
            handler(focus);
        }
    }
}

/// The editing-begin notifications a target's view posts on this platform,
/// with the focus value each reports.
#[cfg(target_os = "macos")]
fn editing_notifications() -> [(NotificationName, bool); 2] {
    use objc2_app_kit::{
        NSControlTextDidBeginEditingNotification, NSControlTextDidEndEditingNotification,
    };
    // SAFETY: both extern statics are `NSString` constants AppKit owns for
    // the process's lifetime; reading them cannot alias mutable state.
    [
        (
            NotificationName::framework(unsafe { NSControlTextDidBeginEditingNotification }),
            true,
        ),
        (
            NotificationName::framework(unsafe { NSControlTextDidEndEditingNotification }),
            false,
        ),
    ]
}

/// The editing-begin notifications a target's view posts on this platform:
/// `UITextField`'s begin/end pair — the kit's text-input controls are all
/// `UITextField`s.
#[cfg(target_os = "ios")]
fn editing_notifications() -> [(NotificationName, bool); 2] {
    use objc2_ui_kit::{
        UITextFieldTextDidBeginEditingNotification, UITextFieldTextDidEndEditingNotification,
    };
    // SAFETY: both extern statics are `NSString` constants UIKit owns for
    // the process's lifetime; reading them cannot alias mutable state.
    [
        (
            NotificationName::framework(unsafe { UITextFieldTextDidBeginEditingNotification }),
            true,
        ),
        (
            NotificationName::framework(unsafe { UITextFieldTextDidEndEditingNotification }),
            false,
        ),
    ]
}

/// Whether `view` currently holds keyboard focus.
///
/// `AppKit`: the window's first responder is the view, or the view is a
/// control a field editor is editing. `UIKit`: the view is the first
/// responder.
#[must_use]
pub fn is_first_responder(view: &PlatformView) -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSControl, NSResponder};
        let responder: &NSResponder = view;
        let editing = view
            .downcast_ref::<NSControl>()
            .is_some_and(|control| control.currentEditor().is_some());
        editing
            || view.window().is_some_and(|window| {
                window
                    .firstResponder()
                    .is_some_and(|first| ptr::eq(&raw const *first, responder))
            })
    }
    #[cfg(target_os = "ios")]
    {
        view.isFirstResponder()
    }
}

/// Asks the platform to give `view` keyboard focus, answering whether it
/// accepted. `false` when the view is in no window or the window refused.
#[must_use]
pub fn become_first_responder(view: &PlatformView) -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSResponder;
        let Some(window) = view.window() else {
            return false;
        };
        let responder: &NSResponder = view;
        window.makeFirstResponder(Some(responder))
    }
    #[cfg(target_os = "ios")]
    {
        view.window().is_some() && view.becomeFirstResponder()
    }
}

/// Asks the platform to take keyboard focus from `view`'s window, answering
/// whether it accepted. `false` when the view is in no window or the window
/// refused.
#[must_use]
pub fn resign_first_responder(view: &PlatformView) -> bool {
    #[cfg(target_os = "macos")]
    {
        let Some(window) = view.window() else {
            return false;
        };
        window.makeFirstResponder(None)
    }
    #[cfg(target_os = "ios")]
    {
        view.resignFirstResponder()
    }
}

/// Whether `view` is attached to a window — a precondition for focus.
#[must_use]
pub fn in_window(view: &PlatformView) -> bool {
    view.window().is_some()
}

/// A view marked as its subtree's focus anchor: the one view a focus
/// manager drives and reads.
///
/// Dropping the target un-marks the view — the registry only weakly
/// references it, and each target's owner (the leaf that rendered the
/// control) keeps it alive for the control's mounted life.
pub struct FocusTarget {
    inner: Rc<Inner>,
}

impl core::fmt::Debug for FocusTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FocusTarget")
            .field("inner", &self.inner)
            .finish()
    }
}

/// A [`FocusTarget::on_change`] subscription; dropping it unsubscribes.
#[derive(Debug)]
pub struct FocusObservation {
    inner: Rc<Inner>,
    id: u64,
}

impl Drop for FocusObservation {
    fn drop(&mut self) {
        self.inner
            .observers
            .borrow_mut()
            .retain(|(id, _)| *id != self.id);
    }
}

impl FocusTarget {
    /// The platform view this target drives.
    #[must_use]
    pub fn view(&self) -> &PlatformView {
        &self.inner.view
    }

    /// Whether the target's view currently holds keyboard focus.
    #[must_use]
    pub fn has_focus(&self) -> bool {
        is_first_responder(&self.inner.view)
    }

    /// Asks the platform to focus the target's view — see
    /// [`become_first_responder`].
    #[must_use]
    pub fn request_focus(&self) -> bool {
        become_first_responder(&self.inner.view)
    }

    /// Asks the platform to unfocus the target's view — see
    /// [`resign_first_responder`].
    #[must_use]
    pub fn clear_focus(&self) -> bool {
        resign_first_responder(&self.inner.view)
    }

    /// Calls `handler` each time the platform gives or takes the target's
    /// focus, until the returned observation is dropped.
    #[must_use]
    pub fn on_change(&self, handler: impl Fn(bool) + 'static) -> FocusObservation {
        let id = self.inner.next.replace(self.inner.next.get() + 1);
        self.inner
            .observers
            .borrow_mut()
            .push((id, Rc::new(handler)));
        FocusObservation {
            inner: Rc::clone(&self.inner),
            id,
        }
    }
}

// The targets installed on live views, keyed by the view's address.
// Entries die with their target: the `Weak` fails to upgrade once the
// leaf that owned the target has dropped it.
thread_local! {
    static TARGETS: RefCell<HashMap<*const PlatformView, Weak<Inner>>> =
        RefCell::new(HashMap::new());
}

/// Marks `view` as its subtree's focus anchor and answers the installed
/// [`FocusTarget`], which the caller keeps for the control's mounted life.
///
/// The target watches the platform's editing-begin/editing-end
/// notifications on `view`, so subscribers see the focus changes the user
/// causes as well as the ones [`FocusTarget::request_focus`] asks for.
#[must_use]
pub fn install(mtm: MainThreadMarker, view: &PlatformView) -> FocusTarget {
    let inner = Rc::new(Inner {
        view: Retained::from(view),
        observers: RefCell::new(Vec::new()),
        next: Cell::new(0),
        notifications: RefCell::new(Vec::new()),
    });

    // The target listens for the view's own editing notifications: the
    // object filter keeps every other field's begin/end out of this
    // target's emit.
    inner
        .notifications
        .borrow_mut()
        .extend(editing_notifications().into_iter().map(|(name, focus)| {
            observe_object(mtm, &name, AsRef::<AnyObject>::as_ref(view), {
                let inner = Rc::downgrade(&inner);
                move || {
                    if let Some(inner) = inner.upgrade() {
                        inner.emit(focus);
                    }
                }
            })
        }));

    TARGETS.with_borrow_mut(|targets| {
        targets.insert(ptr::from_ref(view), Rc::downgrade(&inner));
    });
    FocusTarget { inner }
}

/// Every live focus anchor in `root`'s subtree, in traversal order —
/// the first-responder walk a focus search performs.
#[must_use]
pub fn targets_in(root: &PlatformView) -> Vec<FocusTarget> {
    let mut found = Vec::new();
    collect(root, &mut found);
    found
}

/// The depth-first walk [`targets_in`] runs.
fn collect(view: &PlatformView, found: &mut Vec<FocusTarget>) {
    TARGETS.with_borrow_mut(|targets| {
        if let Some(inner) = targets.get(&ptr::from_ref(view)).and_then(Weak::upgrade) {
            found.push(FocusTarget { inner });
        } else {
            // A dead entry keyed at this address can only confuse a later
            // view allocated at the same one.
            targets.remove(&ptr::from_ref(view));
        }
    });
    for subview in &view.subviews() {
        collect(&subview, found);
    }
}
