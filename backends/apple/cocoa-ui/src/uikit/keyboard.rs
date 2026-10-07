//! The keyboard region: what a `UIKit` keyboard notification carries.
//!
//! `UIKit` posts `UIKeyboardWillShowNotification`,
//! `UIKeyboardWillChangeFrameNotification` and `UIKeyboardWillHideNotification`
//! with a `userInfo` holding the frame the keyboard ends at — in screen
//! coordinates — and the duration and curve of the animation `UIKit` plays to
//! get there. A consumer that mirrors the keyboard's motion applies its own
//! change inside a `UIView` animation with the same parameters.
//!
//! # Safety
//!
//! The `unsafe` here reads `userInfo` by messaging the dictionary directly —
//! the keys are constants `UIKit` exports — and unpacks the `NSValue` the
//! frame key names into a `CGRect`, a copy `NSValue` documents as typed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_core_foundation::{CGPoint, CGRect};
use objc2_foundation::{NSNotification, NSNumber, NSString, NSValue};
use objc2_ui_kit::{
    UICoordinateSpace, UIKeyboardAnimationCurveUserInfoKey, UIKeyboardAnimationDurationUserInfoKey,
    UIKeyboardFrameEndUserInfoKey, UIKeyboardWillChangeFrameNotification,
    UIKeyboardWillHideNotification, UIKeyboardWillShowNotification, UIScrollView,
    UITextFieldTextDidBeginEditingNotification, UITextViewTextDidBeginEditingNotification, UIView,
    UIViewAnimationOptions,
};

use crate::notification::{NotificationName, NotificationObserver, observe_with_notification};

/// The frame and animation a keyboard notification reports.
#[derive(Debug, Clone, Copy)]
pub struct KeyboardChange {
    /// The keyboard's end frame in screen coordinates.
    pub frame: CGRect,
    /// The animation's duration in seconds.
    pub duration: f64,
    /// The `UIViewAnimationCurve` raw value; `curve << 16` is the matching
    /// `UIViewAnimationOptions` curve option.
    pub curve: usize,
}

/// One `userInfo` lookup: the dictionary's value for `key`, `None` when the
/// key is absent.
fn user_info(note: &NSNotification, key: &'static NSString) -> Option<Retained<AnyObject>> {
    let info = note.userInfo()?;
    // SAFETY: `userInfo` is the dictionary `UIKit` filled for the
    // notification; `key` is one of the constants it exports.
    unsafe { msg_send![&*info, objectForKey: key] }
}

/// The change `notification` reports, or `None` when it is not a keyboard
/// notification (a `userInfo` without a frame end).
pub fn change(notification: &NSNotification) -> Option<KeyboardChange> {
    // SAFETY: the frame-end value is an `NSValue` wrapping a `CGRect`, the
    // duration and curve values `NSNumber`s — the types `UIKit` documents
    // for each key.
    let frame: CGRect = unsafe {
        user_info(notification, UIKeyboardFrameEndUserInfoKey)?
            .downcast_ref::<NSValue>()?
            .get()
    };
    // SAFETY: `UIKit` exports the duration and curve keys as `NSString`
    // constants.
    let duration = user_info(notification, unsafe {
        UIKeyboardAnimationDurationUserInfoKey
    })
    .and_then(|value| value.downcast_ref::<NSNumber>().map(NSNumber::doubleValue))
    .unwrap_or(0.0);
    // SAFETY: same constants, exported by `UIKit`.
    let curve = user_info(notification, unsafe { UIKeyboardAnimationCurveUserInfoKey })
        .and_then(|value| value.downcast_ref::<NSNumber>().map(NSNumber::as_usize))
        .unwrap_or(0);
    Some(KeyboardChange {
        frame,
        duration,
        curve,
    })
}

/// A scroll surface's keyboard tracking (layout-spec §7.1 "scroll
/// surfaces").
///
/// The depth the keyboard band covers inside the surface's own window
/// frame becomes the bottom content inset — beyond the `safeAreaInsets`
/// the band already carries — so the content's tail scrolls up clear of
/// the keyboard region, and a text field gaining focus while the
/// keyboard is up is scrolled the minimum distance that brings its
/// frame clear. Every change applies inside the `UIView` animation
/// `UIKit` plays for the keyboard's own motion.
///
/// `ScrollView` and `TableView` hold one in their ivars and rebind it as
/// they move between windows — owned state, never global.
pub struct KeyboardTracking {
    /// The keyboard's frame in the window's coordinates, last reported
    /// by a keyboard notification.
    frame: Cell<CGRect>,
    /// The keyboard and focus observers, live while the surface sits in
    /// a window.
    observers: RefCell<Vec<NotificationObserver>>,
}

impl std::fmt::Debug for KeyboardTracking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyboardTracking").finish_non_exhaustive()
    }
}

impl Default for KeyboardTracking {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardTracking {
    /// An idle tracker: no observers, no frame.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            frame: Cell::new(CGRect::ZERO),
            observers: RefCell::new(Vec::new()),
        }
    }

    /// The tracked keyboard frame — `CGRect::ZERO` when no keyboard is
    /// reported.
    #[must_use]
    pub const fn frame(&self) -> CGRect {
        self.frame.get()
    }

    /// Drops the observers and forgets the frame: the surface left its
    /// window, or its handlers were cleared.
    pub fn clear(&self) {
        self.observers.borrow_mut().clear();
        self.frame.set(CGRect::ZERO);
    }

    /// Installs the observers driving `scroll`'s keyboard behaviour.
    ///
    /// Rebinding is explicit: callers clear first (`didMoveToWindow`),
    /// so a surface that moves windows never holds two sets of
    /// registrations.
    pub fn observe(self: &Rc<Self>, scroll: &UIScrollView, mtm: MainThreadMarker) {
        let mut observers = self.observers.borrow_mut();
        // SAFETY: `UIKit` exports the keyboard notification names as
        // constants for the process's lifetime.
        for name in unsafe {
            [
                UIKeyboardWillShowNotification,
                UIKeyboardWillChangeFrameNotification,
                UIKeyboardWillHideNotification,
            ]
        } {
            let weak = Weak::new(scroll);
            let tracking = Rc::clone(self);
            observers.push(observe_with_notification(
                mtm,
                &NotificationName::framework(name),
                move |note| {
                    if let Some(scroll) = weak.load() {
                        tracking.apply_keyboard_change(&scroll, mtm, note);
                    }
                },
            ));
        }
        // SAFETY: same constants, exported by `UIKit`.
        for name in unsafe {
            [
                UITextFieldTextDidBeginEditingNotification,
                UITextViewTextDidBeginEditingNotification,
            ]
        } {
            let weak = Weak::new(scroll);
            let tracking = Rc::clone(self);
            observers.push(observe_with_notification(
                mtm,
                &NotificationName::framework(name),
                move |_note| {
                    if let Some(scroll) = weak.load() {
                        tracking.scroll_focused_clear(&scroll, true);
                    }
                },
            ));
        }
    }

    /// Whether another scroll surface sits above `scroll` — a nested
    /// surface's subtree sees no keyboard region (the outer surface owns
    /// its safe-area contract), so it neither insets nor scrolls for the
    /// keyboard itself: the field clears once, through the innermost
    /// surface that sees the band.
    fn nested_in_scroll(scroll: &UIScrollView) -> bool {
        let mut ancestor = scroll.superview();
        while let Some(view) = ancestor {
            if view.downcast_ref::<UIScrollView>().is_some() {
                return true;
            }
            ancestor = view.superview();
        }
        false
    }

    /// The depth of the keyboard band inside `scroll`'s window frame on
    /// the bottom edge — the inset the content needs to scroll the tail
    /// clear of the keyboard region.
    fn keyboard_cover(&self, scroll: &UIScrollView) -> f64 {
        let keyboard = self.frame.get();
        if keyboard.size.width <= 0.0 || keyboard.size.height <= 0.0 {
            return 0.0;
        }
        let frame = scroll.convertRect_toView(scroll.bounds(), None);
        let bottom = frame.origin.y + frame.size.height;
        let horizontal = keyboard.origin.x < frame.origin.x + frame.size.width
            && keyboard.origin.x + keyboard.size.width > frame.origin.x;
        if !horizontal || keyboard.origin.y + keyboard.size.height < bottom - 0.5 {
            return 0.0;
        }
        (bottom - keyboard.origin.y).clamp(0.0, frame.size.height)
    }

    /// Applies a keyboard notification: the band depth becomes the bottom
    /// content inset beyond `safeAreaInsets`, and a focused field inside
    /// is scrolled clear — all inside the animation `UIKit` plays for the
    /// change.
    fn apply_keyboard_change(
        self: &Rc<Self>,
        scroll: &UIScrollView,
        mtm: MainThreadMarker,
        note: &NSNotification,
    ) {
        if Self::nested_in_scroll(scroll) {
            return;
        }
        let Some(window) = scroll.window() else {
            return;
        };
        let Some(change) = change(note) else {
            return;
        };
        let space = window.screen().coordinateSpace();
        let frame = window.convertRect_fromCoordinateSpace(change.frame, &space);
        self.frame.set(frame);
        let options = UIViewAnimationOptions(
            (change.curve << 16) | UIViewAnimationOptions::BeginFromCurrentState.0,
        );
        let scroll = Retained::from(scroll);
        let tracking = Rc::clone(self);
        let block = RcBlock::new(move || {
            tracking.apply_keyboard_inset(&scroll);
            tracking.scroll_focused_clear(&scroll, false);
            scroll.layoutIfNeeded();
        });
        // `mtm` keeps the `UIKit` call on the main thread; the options
        // value is the documented `curve << 16` packing.
        UIView::animateWithDuration_delay_options_animations_completion(
            change.duration,
            0.0,
            options,
            &block,
            None,
            mtm,
        );
    }

    /// Adds the covered keyboard depth to the bottom content inset, so
    /// the content tail scrolls up clear of the keyboard region. The
    /// adjusted inset ends at the band depth: `safeAreaInsets` already
    /// carries the container band the scroll crosses.
    fn apply_keyboard_inset(&self, scroll: &UIScrollView) {
        let covered = self.keyboard_cover(scroll);
        let mut inset = scroll.contentInset();
        inset.bottom = (covered - scroll.safeAreaInsets().bottom).max(0.0);
        scroll.setContentInset(inset);
    }

    /// Scrolls the current first responder inside `scroll` the minimum
    /// distance that brings its frame clear of the keyboard band.
    fn scroll_focused_clear(&self, scroll: &UIScrollView, animated: bool) {
        if Self::nested_in_scroll(scroll) {
            return;
        }
        let keyboard = self.frame.get();
        if keyboard.size.height <= 0.0 {
            return;
        }
        let Some(responder) = first_responder(scroll) else {
            return;
        };
        let field = responder.convertRect_toView(responder.bounds(), None);
        let keyboard_top = keyboard.origin.y;
        if field.origin.y + field.size.height <= keyboard_top + 0.5 {
            return;
        }
        // The clearance boundary is the nearer of the band's top and the
        // surface's own bottom edge — a toolbar under the surface holds
        // its frame above the keyboard, and the field clears inside the
        // surface, not under it.
        let own_frame = scroll.convertRect_toView(scroll.bounds(), None);
        let visible_bottom =
            keyboard_top.min(own_frame.origin.y + own_frame.size.height) - own_frame.origin.y;
        let field_bottom = field.origin.y + field.size.height - own_frame.origin.y;
        let delta = field_bottom - visible_bottom;
        if delta <= 0.5 {
            return;
        }
        let inset = scroll.adjustedContentInset();
        let max_y = (scroll.contentSize().height + inset.bottom - scroll.bounds().size.height)
            .max(-inset.top);
        let offset = scroll.contentOffset();
        let new_y = (offset.y + delta).min(max_y);
        if new_y > offset.y + 0.5 {
            let target = CGPoint::new(offset.x, new_y);
            if animated {
                // `setContentOffset(_:animated:)` animates only inside a
                // live UI event context — a focus observer fires outside
                // one, so the same write runs inside a `UIView`
                // animation, the platform's standard ease instead of an
                // instantaneous snap.
                let mtm = scroll.mtm();
                let scroll: Retained<UIScrollView> = Retained::from(scroll);
                let block = RcBlock::new(move || {
                    scroll.setContentOffset_animated(target, false);
                });
                UIView::animateWithDuration_delay_options_animations_completion(
                    0.25,
                    0.0,
                    UIViewAnimationOptions::BeginFromCurrentState,
                    &block,
                    None,
                    mtm,
                );
            } else {
                scroll.setContentOffset_animated(target, false);
            }
        }
    }
}

/// The first responder inside `view`'s subtree, if any — a recursive
/// `isFirstResponder` walk of the view hierarchy.
fn first_responder(view: &UIView) -> Option<Retained<UIView>> {
    if view.isFirstResponder() {
        return Some(Retained::from(view));
    }
    view.subviews()
        .iter()
        .find_map(|subview| first_responder(&subview))
}
