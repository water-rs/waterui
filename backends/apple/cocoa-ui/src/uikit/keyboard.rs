//! The keyboard region: what a `UIKit` keyboard notification carries,
//! and the one object that holds it per window.
//!
//! `UIKit` announces every keyboard frame transition —
//! show, hide and resize alike — through
//! `UIKeyboardWillChangeFrameNotification` with a `userInfo` holding the
//! frame the keyboard ends at — in screen coordinates — and the duration
//! and curve of the animation `UIKit` plays to get there. A consumer that
//! mirrors the keyboard's motion applies its own change inside a `UIView`
//! animation with the same parameters.
//!
//! One [`KeyboardRegion`] per `UIWindow` — a plain `NSObject` attached to
//! the window as an associated object — keeps the last reported frame in
//! window coordinates and the animation's duration. [`region_for`]
//! creates it lazily on first lookup, so the region exists from the
//! first notification after any `WaterUI` view reaches the window and is
//! shared by every host in it — including hosts in apps that own their
//! own `UIWindow`. Hosts and scroll surfaces read the region and only
//! mark: on a notification the region walks the window's whole view
//! tree once — presented panels, which mount their own `HostView` and
//! scroll surfaces outside any content owner's subtree, are reached —
//! marks every region reader, and flushes the pass with
//! `layoutIfNeeded`, all inside a `UIView` animation carrying the
//! notification's duration and curve.
//!
//! # Safety
//!
//! The `unsafe` here reads `userInfo` by messaging the dictionary
//! directly — the keys are constants `UIKit` exports — unpacks the
//! `NSValue` the frame key names into a `CGRect`, a copy `NSValue`
//! documents as typed, and stores the region as a retained associated
//! object on the window.

use std::cell::{Cell, OnceCell, RefCell};

use block2::RcBlock;
use objc2::ffi::{
    OBJC_ASSOCIATION_RETAIN_NONATOMIC, objc_getAssociatedObject, objc_setAssociatedObject,
};
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect};
use objc2_foundation::{NSNotification, NSNumber, NSObject, NSObjectProtocol, NSString, NSValue};
use objc2_ui_kit::{
    UICoordinateSpace, UIKeyboardAnimationCurveUserInfoKey, UIKeyboardAnimationDurationUserInfoKey,
    UIKeyboardFrameEndUserInfoKey, UIKeyboardWillChangeFrameNotification, UIScrollView,
    UITextFieldTextDidBeginEditingNotification, UITextViewTextDidBeginEditingNotification, UIView,
    UIViewAnimationOptions, UIWindow,
};

use crate::notification::{NotificationName, NotificationObserver, observe_with_notification};
use crate::uikit::host_view::HostView;
use crate::uikit::scroll::ScrollView;
use crate::uikit::table::TableView;

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
/// the region registers it on that one name, so a missing frame end,
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

/// The state one window's keyboard region holds.
#[derive(Default, Debug)]
pub struct KeyboardRegionIvars {
    /// The keyboard's end frame in the window's coordinates —
    /// `CGRect::ZERO` while no keyboard shows.
    frame: Cell<CGRect>,
    /// The animation the last notification reported — what a follow-up
    /// animation (a focused field scrolling clear) adopts.
    duration: Cell<f64>,
    /// The window the region belongs to — weak, since the window owns
    /// the region through its associated object.
    window: OnceCell<Weak<UIWindow>>,
    /// The keyboard notification observation — `WillChangeFrame` alone
    /// carries every transition: show, hide and resize post it alongside
    /// their semantic notifications.
    observers: RefCell<Vec<NotificationObserver>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the class
    // holds value cells and an observer token and does not implement
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiKeyboardRegion"]
    #[thread_kind = MainThreadOnly]
    #[ivars = KeyboardRegionIvars]
    /// One window's keyboard region: the frame `UIKit` last reported and
    /// the animation it played, shared by every `WaterUI` host in the
    /// window. Created lazily by [`region_for`] and retained by the
    /// window as an associated object.
    pub struct KeyboardRegion;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for KeyboardRegion {}
);

impl KeyboardRegion {
    /// A fresh region for `window` with the keyboard observation live.
    fn new(mtm: MainThreadMarker, window: &UIWindow) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(KeyboardRegionIvars::default());
        // SAFETY: `init` is `NSObject`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this.ivars()
            .window
            .set(Weak::new(window))
            .expect("a fresh region has no window yet");
        // A window whose keyboard is already docked when the first
        // `WaterUI` view arrives — an app-owned window — seeds the
        // region from the keyboard's own layout guide, reported in the
        // owning view's (the window's) coordinates; a zero-height or
        // offscreen guide means no keyboard.
        let guide = <UIWindow as AsRef<UIView>>::as_ref(window).keyboardLayoutGuide();
        let seed = guide.layoutFrame();
        let bounds = window.bounds();
        if seed.size.height > 0.0
            && seed.origin.y < bounds.size.height
            && seed.origin.y + seed.size.height > 0.0
        {
            this.ivars().frame.set(seed);
        }
        let weak = Weak::new(&*this);
        // SAFETY: `UIKit` exports the notification name as a constant for
        // the process's lifetime.
        let observer = observe_with_notification(
            mtm,
            &NotificationName::framework(unsafe { UIKeyboardWillChangeFrameNotification }),
            move |note| {
                if let Some(region) = weak.load() {
                    region.apply(note);
                }
            },
        );
        this.ivars().observers.borrow_mut().push(observer);
        this
    }

    /// The keyboard's end frame in the window's coordinates —
    /// `CGRect::ZERO` until the first notification.
    #[must_use]
    pub fn frame(&self) -> CGRect {
        self.ivars().frame.get()
    }

    /// The animation duration the last keyboard notification reported.
    #[must_use]
    pub fn duration(&self) -> f64 {
        self.ivars().duration.get()
    }

    /// Applies a keyboard notification: stores the change and relayouts
    /// the window inside the notification's own animation — the same
    /// `UIView` animation parameters a consumer mirrors, so every region
    /// reader lands its new frame with the keyboard's motion.
    fn apply(&self, note: &NSNotification) {
        let window = self
            .ivars()
            .window
            .get()
            .and_then(Weak::load)
            .expect("the window retains its region — a live region means a live window");
        let change = change(note, &window);
        let block = RcBlock::new({
            let region: Retained<Self> = Retained::from(self);
            let window: Retained<UIWindow> = window;
            move || {
                region.ivars().frame.set(change.frame);
                region.ivars().duration.set(change.duration);
                mark_region_readers(&window);
                window.layoutIfNeeded();
            }
        });
        UIView::animateWithDuration_delay_options_animations_completion(
            change.duration,
            0.0,
            change.options,
            &block,
            None,
            self.mtm(),
        );
    }
}

/// The keyboard region attached to `window`, created and associated on
/// first lookup.
///
/// Laziness is the contract: the first `WaterUI` view to reach a window
/// brings the region into existence, so it answers from the first
/// notification after that — and a second call answers the same object.
///
/// # Panics
///
/// Never in practice: the associated object was checked non-null before
/// it is retained.
#[must_use]
pub fn region_for(window: &UIWindow) -> Retained<KeyboardRegion> {
    // SAFETY: `window` is a live object on the main thread; the returned
    // pointer is only borrowed for this lookup.
    let existing = unsafe {
        objc_getAssociatedObject(
            core::ptr::from_ref::<UIWindow>(window).cast::<AnyObject>(),
            crate::view::association_key(c"dev.cocoaui.keyboardRegion"),
        )
    };
    if !existing.is_null() {
        // SAFETY: `region_for` writes only `KeyboardRegion` instances
        // under this key, and the association retains them.
        return unsafe { Retained::retain(existing.cast_mut().cast::<KeyboardRegion>()) }
            .expect("the window's associated region is a live object");
    }
    let region = KeyboardRegion::new(window.mtm(), window);
    // SAFETY: `window` and `region` are live objects on the main thread;
    // the runtime retains `region` for the association's lifetime.
    unsafe {
        objc_setAssociatedObject(
            core::ptr::from_ref::<UIWindow>(window)
                .cast_mut()
                .cast::<AnyObject>(),
            crate::view::association_key(c"dev.cocoaui.keyboardRegion"),
            Retained::as_ptr(&region).cast_mut().cast::<AnyObject>(),
            OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        );
    }
    region
}

/// The keyboard state `view`'s window reports: the frame in window
/// coordinates and the last animation's duration. `None` while `view`
/// sits outside any window, where there is no keyboard to see.
#[must_use]
pub fn window_keyboard(view: &UIView) -> Option<(CGRect, f64)> {
    let window = view.window()?;
    let region = region_for(&window);
    Some((region.frame(), region.duration()))
}

/// Marks every region reader in `view`'s subtree for the layout pass a
/// keyboard region change drives: every kit `HostView` that runs a
/// layout handler — its children's placement reads the boundaries
/// through the sibling backend's region context — and every kit scroll
/// surface, whose content insets do. The region object starts the walk
/// at the window, once per notification, so readers presented outside a
/// content owner's subtree — a menu panel's `HostView` and scroll
/// surface — are reached too. The walk descends through the foreign
/// containers `UIKit` interposes (transition, container and wrapper
/// views inside navigation and tab controllers), which never run kit
/// `layoutSubviews`; a scroll surface's own subtree is not walked — it
/// owns the safe-area contract inside itself and nothing under it reads
/// the regions.
fn mark_region_readers(view: &UIView) {
    if let Some(scroll) = view.downcast_ref::<ScrollView>() {
        scroll.mark_keyboard();
        return;
    }
    if let Some(table) = view.downcast_ref::<TableView>() {
        table.mark_keyboard();
        return;
    }
    if let Some(host) = view.downcast_ref::<HostView>()
        && host.has_layout_handler()
    {
        view.setNeedsLayout();
    }
    for subview in &view.subviews() {
        mark_region_readers(&subview);
    }
}

/// A scroll surface's keyboard state (layout-spec §7.1 "scroll
/// surfaces").
///
/// The surface's own layout pass recomputes the depth the keyboard band
/// covers inside its window frame — from the window's keyboard region —
/// when the pass was marked by a notification or the surface's bounds
/// size changed; a pass re-run by anything else — a content-offset
/// change, a same-size move — recomputes nothing. The depth, beyond the
/// `safeAreaInsets` the band already carries, becomes the bottom content
/// inset, shifted by the delta from the last applied value rather than
/// overwriting any other `contentInset.bottom` contribution.
///
/// A text field gaining focus while the keyboard is up is scrolled the
/// minimum distance that brings its frame clear.
///
/// `ScrollView` and `TableView` hold one in their ivars and rebind its
/// focus observers as they move between windows — owned state, never
/// global.
pub struct KeyboardTracking {
    /// The keyboard contribution the last pass wrote into
    /// `contentInset.bottom` — kept so the next pass shifts the inset by
    /// the delta instead of clobbering other bottom-inset terms.
    applied_inset: Cell<f64>,
    /// The window-space frame and bottom safe-area inset the last
    /// contribution computed against — a move or a safe-area change
    /// re-derives the covered band with no notification at all, while a
    /// content-offset pass recomputes nothing.
    applied_frame: Cell<CGRect>,
    /// See `applied_frame`.
    applied_safe: Cell<f64>,
    /// The mark the window's notification walk left — the next pass
    /// recomputes whatever the size did.
    marked: Cell<bool>,
    /// The text-editing focus observers, live while the surface sits in
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
    /// An idle tracking state: no observers, nothing applied yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            applied_inset: Cell::new(0.0),
            applied_frame: Cell::new(CGRect::ZERO),
            applied_safe: Cell::new(0.0),
            marked: Cell::new(false),
            observers: RefCell::new(Vec::new()),
        }
    }

    /// The window's notification walk marked the surface — the next
    /// layout pass recomputes the keyboard contribution.
    pub fn mark(&self) {
        self.marked.set(true);
    }

    /// Drops the observers: the surface left its window, or its handlers
    /// were cleared. The applied contribution stays — the next pass the
    /// surface runs reconciles the inset against it.
    pub fn clear(&self) {
        self.observers.borrow_mut().clear();
    }

    /// Arms the surface for its window — called from `didMoveToWindow`
    /// after [`Self::clear`], so a surface that moves windows never
    /// holds two sets of registrations. Installs only the text-editing
    /// focus observers: the keyboard frame itself comes from the
    /// window's region object at layout time (`window_keyboard`), and
    /// the region's notification already drives the layout passes that
    /// re-read it, so a surface needs no observer of its own.
    pub fn attach(&self, scroll: &UIScrollView, mtm: MainThreadMarker) {
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
            observers.push(observe_with_notification(
                mtm,
                &NotificationName::framework(name),
                move |_note| {
                    if let Some(scroll) = weak_scroll.load() {
                        Self::scroll_focused_clear(&scroll, true);
                    }
                },
            ));
        }
    }

    /// Recomputes the keyboard contribution in `scroll`'s own layout
    /// pass — only when the window's notification marked the surface or
    /// what the cover reads changed: the surface's frame in window
    /// coordinates — a banner pushing the surface down into the band
    /// counts, a content-offset scroll does not — and the bottom
    /// `safeAreaInsets` the covered band already carries. The covered
    /// band depth becomes the bottom content inset — shifted by the
    /// delta from the last applied value so other `contentInset.bottom`
    /// terms survive. The focused field is scrolled clear only when the
    /// contribution itself grows; a pass that re-runs because the user
    /// scrolled must leave the offset alone.
    pub fn apply_layout(&self, scroll: &UIScrollView) {
        let frame = scroll.convertRect_toView(scroll.bounds(), None);
        let safe_bottom = scroll.safeAreaInsets().bottom;
        if !self.marked.replace(false)
            && self.applied_frame.get() == frame
            && (self.applied_safe.get() - safe_bottom).abs() <= f64::EPSILON
        {
            return;
        }
        let contribution = if Self::nested_in_scroll(scroll) {
            0.0
        } else {
            Self::keyboard_cover(scroll, frame, safe_bottom)
        };
        let previous = self.applied_inset.replace(contribution);
        self.applied_frame.set(frame);
        self.applied_safe.set(safe_bottom);
        if (contribution - previous).abs() > f64::EPSILON {
            let mut inset = scroll.contentInset();
            inset.bottom += contribution - previous;
            scroll.setContentInset(inset);
            if contribution > previous {
                Self::scroll_focused_clear(scroll, false);
            }
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
    /// clear of the keyboard region. `frame` is the window-space frame
    /// the pass already read; `safe_bottom` the `safeAreaInsets` term
    /// `adjustedContentInset` already carries, so the inset only needs
    /// the covered depth beyond it.
    fn keyboard_cover(scroll: &UIScrollView, frame: CGRect, safe_bottom: f64) -> f64 {
        let Some((keyboard, _)) = window_keyboard(scroll) else {
            return 0.0;
        };
        if keyboard.size.width <= 0.0 || keyboard.size.height <= 0.0 {
            return 0.0;
        }
        let bottom = frame.origin.y + frame.size.height;
        let horizontal = keyboard.origin.x < frame.origin.x + frame.size.width
            && keyboard.origin.x + keyboard.size.width > frame.origin.x;
        if !horizontal || keyboard.origin.y + keyboard.size.height < bottom - 0.5 {
            return 0.0;
        }
        (bottom - keyboard.origin.y - safe_bottom).clamp(0.0, frame.size.height)
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
                let mtm = scroll.mtm();
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
