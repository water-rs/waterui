//! The keyboard region: what a `UIKit` keyboard notification carries.
//!
//! `UIKit` announces every keyboard frame transition —
//! show, hide and resize alike — through
//! `UIKeyboardWillChangeFrameNotification` with a `userInfo` holding the
//! frame the keyboard ends at — in screen coordinates — and the duration
//! and curve of the animation `UIKit` plays to get there. A consumer that
//! mirrors the keyboard's motion applies its own change inside a `UIView`
//! animation with the same parameters.
//!
//! The window's keyboard owner — the window root, or the outermost kit
//! host view when the window root is not one of ours — tracks the last
//! reported frame and duration and reports them through
//! `cocoaUiKeyboardFrame`, `cocoaUiKeyboardDuration` and
//! `cocoaUiTracksKeyboard`. Scroll surfaces read that shared state in
//! their own layout passes instead of keeping a private copy, so a
//! surface's inset follows its current frame and never depends on
//! notification delivery order.
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
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_core_foundation::{CGPoint, CGRect};
use objc2_foundation::{NSNotification, NSNumber, NSObjectProtocol, NSString, NSValue};
use objc2_ui_kit::{
    UICoordinateSpace, UIKeyboardAnimationCurveUserInfoKey, UIKeyboardAnimationDurationUserInfoKey,
    UIKeyboardFrameEndUserInfoKey, UIKeyboardWillChangeFrameNotification, UIScrollView,
    UITextFieldTextDidBeginEditingNotification, UITextViewTextDidBeginEditingNotification, UIView,
    UIViewAnimationOptions, UIWindow,
};

use crate::notification::{NotificationName, NotificationObserver, observe_with_notification};

/// The frame and animation a keyboard notification reports, resolved
/// into the window's coordinate space.
#[derive(Debug, Clone, Copy)]
pub struct KeyboardChange {
    /// The keyboard's end frame in the window's coordinates.
    pub frame: CGRect,
    /// The animation's duration in seconds.
    pub duration: f64,
    /// The `UIViewAnimationOptions` matching the notification's curve —
    /// the documented `curve << 16` packing — plus
    /// `BeginFromCurrentState` so a second notification mid-flight
    /// picks up where the first left the region.
    pub options: UIViewAnimationOptions,
}

/// One `userInfo` lookup: the dictionary's value for `key`, `None` when the
/// key is absent.
fn user_info(note: &NSNotification, key: &'static NSString) -> Option<Retained<AnyObject>> {
    let info = note.userInfo()?;
    // SAFETY: `userInfo` is the dictionary `UIKit` filled for the
    // notification; `key` is one of the constants it exports.
    unsafe { msg_send![&*info, objectForKey: key] }
}

/// The change `notification` reports, with its screen-space end frame
/// already converted into `window`'s coordinates — the one place the
/// conversion and the `curve << 16` packing live.
///
/// # Panics
///
/// When `notification` is not a `UIKeyboardWillChangeFrameNotification` —
/// the owner registers it on that one name, so a missing frame end,
/// duration or curve is a caller bug, not an event to skip.
#[must_use]
pub fn change(notification: &NSNotification, window: &UIWindow) -> KeyboardChange {
    // SAFETY: the frame-end value is an `NSValue` wrapping a `CGRect`, the
    // duration and curve values `NSNumber`s — the types `UIKit` documents
    // for each key.
    let frame: CGRect = unsafe {
        user_info(notification, UIKeyboardFrameEndUserInfoKey)
            .expect("a keyboard notification carries a frame-end value")
            .downcast_ref::<NSValue>()
            .expect("the keyboard frame-end value is an NSValue")
            .get()
    };
    // SAFETY: `UIKit` exports the duration and curve keys as `NSString`
    // constants.
    let duration = user_info(notification, unsafe {
        UIKeyboardAnimationDurationUserInfoKey
    })
    .expect("a keyboard notification carries its animation duration")
    .downcast_ref::<NSNumber>()
    .expect("the keyboard animation-duration value is an NSNumber")
    .doubleValue();
    // SAFETY: same constants, exported by `UIKit`.
    let curve = user_info(notification, unsafe { UIKeyboardAnimationCurveUserInfoKey })
        .expect("a keyboard notification carries its animation curve")
        .downcast_ref::<NSNumber>()
        .expect("the keyboard animation-curve value is an NSNumber")
        .as_usize();
    let space = window.screen().coordinateSpace();
    KeyboardChange {
        frame: window.convertRect_fromCoordinateSpace(frame, &space),
        duration,
        options: UIViewAnimationOptions(
            (curve << 16) | UIViewAnimationOptions::BeginFromCurrentState.0,
        ),
    }
}

/// Whether `candidate` currently tracks the keyboard for its window —
/// `cocoaUiTracksKeyboard` is `true` only while a window owner (a kit
/// host view, or an orphaned scroll surface) holds live observers.
fn is_tracking(candidate: &UIView) -> bool {
    candidate.respondsToSelector(sel!(cocoaUiTracksKeyboard))
        // SAFETY: only kit classes declare the selector, as a `bool`
        // report of live keyboard tracking.
        && unsafe { msg_send![candidate, cocoaUiTracksKeyboard] }
}

/// The `(frame, duration)` a tracking owner reports.
///
/// # Safety
///
/// `owner` must answer `true` to [`is_tracking`] — kit classes declare
/// both selectors as a `CGRect` frame and an `NSTimeInterval` duration.
unsafe fn read_tracked(owner: &UIView) -> (CGRect, f64) {
    // SAFETY: see the caller contract above.
    unsafe {
        (
            msg_send![owner, cocoaUiKeyboardFrame],
            msg_send![owner, cocoaUiKeyboardDuration],
        )
    }
}

/// The keyboard's window-space frame and last animation duration the
/// window's keyboard owner tracks for `view`. `None` while `view` is
/// outside any window, where there is no keyboard to see.
///
/// The owner is the window root when it tracks (`cocoaUiTracksKeyboard`),
/// or the nearest ancestor host view that took tracking for an embedded
/// subtree — a kit view inside a window whose root is foreign still finds
/// its own tracker.
///
/// # Panics
///
/// When `view` is in a window but no tracker exists — the root does not
/// track and no ancestor host view does either. Every kit-mounted tree
/// owns one (the window root, or the outermost `HostView` under a foreign
/// root), so reaching this means a subtree was attached without any kit
/// host.
#[must_use]
pub fn window_keyboard(view: &UIView) -> Option<(CGRect, f64)> {
    let window = view.window()?;
    Some(tracked_keyboard(view, &window))
}

/// The tracker `view`'s window resolves to — see [`window_keyboard`].
///
/// The lookup prefers the shared owner — the window root, then `view`'s
/// strict ancestors — and consults `view` itself last, so an orphaned
/// surface's own tracking answers only when no real owner exists. A
/// surface under an owner therefore always reads the tree's single
/// source, never its own cells.
fn tracked_keyboard(view: &UIView, window: &UIWindow) -> (CGRect, f64) {
    if let Some(root) = window.rootViewController().and_then(|c| c.view())
        && is_tracking(&root)
    {
        // SAFETY: `root` reports live tracking.
        return unsafe { read_tracked(&root) };
    }
    // An embedded subtree under a foreign root: the nearest host view
    // that took tracking owns the keyboard state for it.
    let mut current = view.superview();
    while let Some(candidate) = current {
        if is_tracking(&candidate) {
            // SAFETY: `candidate` reports live tracking.
            return unsafe { read_tracked(&candidate) };
        }
        current = candidate.superview();
    }
    // An orphaned scroll surface is its own keyboard owner — the
    // fallback a bare `UIScrollView` subtree mounted without a kit host
    // needs (a foreign window, a fixture window).
    if is_tracking(view) {
        // SAFETY: `view` reports live tracking.
        return unsafe { read_tracked(view) };
    }
    panic!(
        "no keyboard tracking is live in this window: the window root does \
         not answer `cocoaUiTracksKeyboard` and neither {view:?} nor any \
         ancestor of it tracks — kit content laid out in a window must sit \
         under a kit host view that took tracking in `didMoveToWindow`"
    );
}

/// Whether a tracker already serves `view`'s window — the window root
/// tracking, or a strict ancestor of `view` that does. Evaluated when a
/// scroll surface attaches so an orphaned surface can take ownership.
fn has_tracker(view: &UIView) -> bool {
    let Some(window) = view.window() else {
        return false;
    };
    if window
        .rootViewController()
        .and_then(|controller| controller.view())
        .is_some_and(|root| is_tracking(&root))
    {
        return true;
    }
    let mut ancestor = view.superview();
    while let Some(candidate) = ancestor {
        if is_tracking(&candidate) {
            return true;
        }
        ancestor = candidate.superview();
    }
    false
}

/// A scroll surface's keyboard state (layout-spec §7.1 "scroll
/// surfaces").
///
/// The surface's own layout pass recomputes the depth the keyboard band
/// covers inside its window frame — from the frame the window's keyboard
/// owner tracks — and that depth, beyond the `safeAreaInsets` the band
/// already carries, becomes the bottom content inset so the content's
/// tail scrolls up clear of the keyboard region. Recomputing inside the
/// layout pass means the inset follows the surface's frame however it
/// changes — a window move, a resize, a sibling growing — not only a
/// notification, and it moves the inset by the delta rather than
/// overwriting any other `contentInset.bottom` contribution.
///
/// A text field gaining focus while the keyboard is up is scrolled the
/// minimum distance that brings its frame clear.
///
/// `ScrollView` and `TableView` hold one in their ivars and rebind its
/// focus observer as they move between windows — owned state, never
/// global.
pub struct KeyboardTracking {
    /// The keyboard contribution the last pass wrote into
    /// `contentInset.bottom` — kept so the next pass shifts the inset by
    /// the delta instead of clobbering other bottom-inset terms.
    applied_inset: Cell<f64>,
    /// The notification observers — the text-editing focus observers
    /// always, plus the surface's own `WillChangeFrame` observer while
    /// it self-tracks — live while the surface sits in a window.
    observers: RefCell<Vec<NotificationObserver>>,
    /// Whether the surface currently carries its own `WillChangeFrame`
    /// observer — the answer `cocoaUiTracksKeyboard` reports: true only
    /// while the surface is its window's keyboard owner because no
    /// tracker exists above it.
    self_tracking: Cell<bool>,
    /// The frame and duration the surface's own observer last stored —
    /// what `cocoaUiKeyboardFrame`/`cocoaUiKeyboardDuration` report
    /// while self-tracking; unread while an owner above answers first.
    tracked_frame: Cell<CGRect>,
    /// The duration paired with [`Self::tracked_frame`].
    tracked_duration: Cell<f64>,
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
    /// An idle tracker: no observers, nothing applied yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            applied_inset: Cell::new(0.0),
            observers: RefCell::new(Vec::new()),
            self_tracking: Cell::new(false),
            tracked_frame: Cell::new(CGRect::ZERO),
            tracked_duration: Cell::new(0.0),
        }
    }

    /// Drops the observers: the surface left its window, or its handlers
    /// were cleared. Self-tracking ends with them — the frame and
    /// duration return to zero. The applied contribution stays — the
    /// next pass the surface runs reconciles the inset against it.
    pub fn clear(&self) {
        self.observers.borrow_mut().clear();
        self.self_tracking.set(false);
        self.tracked_frame.set(CGRect::ZERO);
        self.tracked_duration.set(0.0);
    }

    /// Arms the surface for its window — called from `didMoveToWindow`
    /// after [`Self::clear`], so a surface that moves windows never
    /// holds two sets of registrations.
    ///
    /// Always installs the text-editing focus observers. When no
    /// tracker exists in the window — the root does not track and no
    /// ancestor host does either — the surface also observes
    /// `UIKeyboardWillChangeFrameNotification` itself and becomes its
    /// own window's keyboard owner: the shape a bare `UIScrollView`
    /// subtree mounted under a foreign root takes. A surface under a
    /// kit host never installs this observer — one observer per tree
    /// keeps the frame single-sourced.
    pub fn attach(self: &Rc<Self>, scroll: &UIScrollView, mtm: MainThreadMarker) {
        let mut observers = self.observers.borrow_mut();
        // SAFETY: `UIKit` exports the text-editing notification names as
        // constants for the process's lifetime.
        for name in unsafe {
            [
                UITextFieldTextDidBeginEditingNotification,
                UITextViewTextDidBeginEditingNotification,
            ]
        } {
            let weak_scroll = Weak::new(scroll);
            let tracking = Rc::downgrade(self);
            observers.push(observe_with_notification(
                mtm,
                &NotificationName::framework(name),
                move |_note| {
                    if let Some(scroll) = weak_scroll.load()
                        && tracking.upgrade().is_some()
                    {
                        Self::scroll_focused_clear(&scroll, true);
                    }
                },
            ));
        }
        if has_tracker(scroll) {
            return;
        }
        self.self_tracking.set(true);
        let weak_scroll = Weak::new(scroll);
        let tracking = Rc::downgrade(self);
        observers.push(observe_with_notification(
            mtm,
            // SAFETY: `UIKit` exports the notification name as a
            // constant for the process's lifetime. WillChangeFrame alone
            // carries every transition — show, hide and resize post it
            // alongside their semantic notifications.
            &NotificationName::framework(unsafe { UIKeyboardWillChangeFrameNotification }),
            move |note| {
                let Some(scroll) = weak_scroll.load() else {
                    return;
                };
                let Some(tracking) = tracking.upgrade() else {
                    return;
                };
                let Some(window) = scroll.window() else {
                    return;
                };
                let change = change(note, &window);
                tracking.tracked_frame.set(change.frame);
                tracking.tracked_duration.set(change.duration);
                let block = RcBlock::new(move || {
                    scroll.setNeedsLayout();
                    scroll.layoutIfNeeded();
                });
                UIView::animateWithDuration_delay_options_animations_completion(
                    change.duration,
                    0.0,
                    change.options,
                    &block,
                    None,
                    mtm,
                );
            },
        ));
    }

    /// Whether the surface currently carries its own keyboard observer
    /// — the answer `cocoaUiTracksKeyboard` reports.
    #[must_use]
    pub const fn tracks(&self) -> bool {
        self.self_tracking.get()
    }

    /// The frame the surface's own observer last stored — the answer
    /// `cocoaUiKeyboardFrame` reports.
    #[must_use]
    pub const fn tracked_frame(&self) -> CGRect {
        self.tracked_frame.get()
    }

    /// The duration the surface's own observer last stored — the answer
    /// `cocoaUiKeyboardDuration` reports.
    #[must_use]
    pub const fn tracked_duration(&self) -> f64 {
        self.tracked_duration.get()
    }

    /// Recomputes the keyboard contribution in `scroll`'s own layout
    /// pass: the covered band depth becomes the bottom content inset —
    /// shifted by the delta from the last pass so other
    /// `contentInset.bottom` terms survive — and a focused field inside
    /// is scrolled clear.
    pub fn apply_layout(&self, scroll: &UIScrollView) {
        let contribution = if Self::nested_in_scroll(scroll) {
            0.0
        } else {
            Self::keyboard_cover(scroll)
        };
        let previous = self.applied_inset.replace(contribution);
        if (contribution - previous).abs() > f64::EPSILON {
            let mut inset = scroll.contentInset();
            inset.bottom += contribution - previous;
            scroll.setContentInset(inset);
        }
        if contribution > 0.0 {
            Self::scroll_focused_clear(scroll, false);
        }
    }

    /// Whether another kit scroll surface sits above `scroll` — a nested
    /// surface's subtree sees no keyboard region (the outer surface owns
    /// its safe-area contract), so it neither insets nor scrolls for the
    /// keyboard itself: the field clears once, through the innermost
    /// surface that sees the band. A foreign `UIScrollView` — a
    /// `UITextView` — is not a kit surface and does not count.
    fn nested_in_scroll(scroll: &UIScrollView) -> bool {
        let mut ancestor = scroll.superview();
        while let Some(view) = ancestor {
            if crate::view::is_scroll_surface(&view) {
                return true;
            }
            ancestor = view.superview();
        }
        false
    }

    /// The depth of the keyboard band inside `scroll`'s window frame on
    /// the bottom edge — the inset the content needs to scroll the tail
    /// clear of the keyboard region.
    fn keyboard_cover(scroll: &UIScrollView) -> f64 {
        let Some((keyboard, _)) = window_keyboard(scroll) else {
            return 0.0;
        };
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
        // The covered band beyond the surface's own `safeAreaInsets` —
        // `adjustedContentInset` already carries those, so the content
        // inset only needs the rest.
        (bottom - keyboard.origin.y - scroll.safeAreaInsets().bottom).clamp(0.0, frame.size.height)
    }

    /// Scrolls the current first responder inside `scroll` the minimum
    /// distance that brings its frame clear of the keyboard band.
    fn scroll_focused_clear(scroll: &UIScrollView, animated: bool) {
        if Self::nested_in_scroll(scroll) {
            return;
        }
        let Some((keyboard, duration)) = window_keyboard(scroll) else {
            return;
        };
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
                // animation of the keyboard's own duration instead of an
                // instantaneous snap.
                let mtm = MainThreadMarker::new()
                    .expect("the keyboard layout pass runs on the main thread");
                let scroll: Retained<UIScrollView> = Retained::from(scroll);
                let block = RcBlock::new(move || {
                    scroll.setContentOffset_animated(target, false);
                });
                UIView::animateWithDuration_delay_options_animations_completion(
                    duration,
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
