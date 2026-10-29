//! The `NSMenu` popup a Mac Catalyst process is tracking, for the owner that
//! must end it.
//!
//! Under Catalyst a button's menu is not a `UIKit` context menu: `UIKit`
//! hands it to `AppKit`, which presents an `NSMenu` popup whose tracking
//! runs modally on the main thread. `setMenu:` does not replace that
//! popup's rows and `removeFromSuperview` does not close it, so a popup
//! left open offers commands its owner has retired (water-rs/waterui#3729).
//! `UIContextMenuInteraction.dismissMenu()` cannot end it from inside the
//! tracking session, but `-[NSMenu cancelTracking]` does.
//!
//! [`MenuTracking`] is the application's record of the popup being tracked,
//! kept from `NSMenuDidBeginTrackingNotification` and
//! `NSMenuDidEndTrackingNotification`. The application delegate owns it
//! (see `application::menu_tracking`); a trigger that knows its own menu is
//! the open one asks it to [`cancel`](MenuTracking::cancel) the tracking.
//!
//! # Safety
//!
//! `NSMenu` has no binding on this target (the iOS SDK headers
//! `objc2-ui-kit` is generated from do not declare it). The tracked menu is
//! the notification's object and receives `-cancelTracking`, which takes no
//! argument and answers `void`, on the main thread `AppKit` requires.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol, NSString};

/// The root `NSMenu` `AppKit` is tracking, if any.
type TrackedMenu = Rc<RefCell<Option<Retained<AnyObject>>>>;

/// The application's record of the `NSMenu` popup being tracked.
pub(super) struct MenuTracking {
    menu: TrackedMenu,
    center: Retained<NSNotificationCenter>,
    tokens: [Retained<ProtocolObject<dyn NSObjectProtocol>>; 2],
}

impl MenuTracking {
    /// Starts observing menu tracking.
    pub(super) fn observe() -> Self {
        let menu = TrackedMenu::default();
        let center = NSNotificationCenter::defaultCenter();
        let begin = {
            let menu = Rc::clone(&menu);
            RcBlock::new(move |notification: NonNull<NSNotification>| {
                // SAFETY: the center hands the block a live notification.
                let tracked = unsafe { notification.as_ref() }
                    .object()
                    .expect("a menu tracking notification names its menu");
                // Submenus begin tracking inside the root menu's session;
                // cancelling the root ends them all.
                menu.borrow_mut().get_or_insert(tracked);
            })
        };
        let end = {
            let menu = Rc::clone(&menu);
            RcBlock::new(move |notification: NonNull<NSNotification>| {
                // SAFETY: the center hands the block a live notification.
                let ended = unsafe { notification.as_ref() }
                    .object()
                    .expect("a menu tracking notification names its menu");
                let mut menu = menu.borrow_mut();
                if menu
                    .as_ref()
                    .is_some_and(|tracked| Retained::as_ptr(tracked) == Retained::as_ptr(&ended))
                {
                    *menu = None;
                }
            })
        };
        let observe = |name: &str, block: &block2::DynBlock<dyn Fn(NonNull<NSNotification>)>| {
            // SAFETY: the block runs on the posting (main) thread, borrows
            // nothing and stays alive for the observer's lifetime.
            unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(&NSString::from_str(name)),
                    None,
                    None,
                    block,
                )
            }
        };
        let tokens = [
            observe("NSMenuDidBeginTrackingNotification", &begin),
            observe("NSMenuDidEndTrackingNotification", &end),
        ];
        Self {
            menu,
            center,
            tokens,
        }
    }

    /// Ends the tracking session of the popup being tracked.
    ///
    /// `AppKit` leaves the session when control returns to its tracking
    /// loop, and the popup takes no pick from then on.
    ///
    /// # Panics
    ///
    /// When no popup is being tracked: a caller cancels only on behalf of a
    /// trigger that has seen its own menu presented.
    pub(super) fn cancel(&self) {
        let menu = self
            .menu
            .borrow_mut()
            .take()
            .expect("a trigger reports its menu open while no `NSMenu` is being tracked");
        // SAFETY: `-cancelTracking` takes no argument and answers `void`.
        let () = unsafe { msg_send![&*menu, cancelTracking] };
    }
}

impl Drop for MenuTracking {
    fn drop(&mut self) {
        for token in &self.tokens {
            // SAFETY: the tokens are the objects `addObserverForName:` returned.
            unsafe { self.center.removeObserver(token.as_ref()) };
        }
    }
}
