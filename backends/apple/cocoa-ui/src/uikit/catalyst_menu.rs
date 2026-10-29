//! Observing the menu a Mac Catalyst button presents.
//!
//! Under Catalyst's iPad idiom a button menu is not a `UIKit` context menu
//! whose visible elements `UIContextMenuInteraction` vends: `UIKit` hands
//! the menu to `AppKit`, which presents it as an `NSMenu` popup whose
//! tracking runs a modal loop on the main thread. The popup is presented
//! later, from inside whichever main-thread pump turns `AppKit`'s event
//! loop next, and the tracking blocks that pump until the menu is closed.
//!
//! [`within_presented_menu`] observes that menu where it really exists:
//! `AppKit` posts `NSMenuDidBeginTrackingNotification` as tracking starts,
//! and the tracking loop services the main run loop in the event-tracking
//! mode, so a block scheduled then runs *inside* the tracking session. The
//! caller's continuation runs there, against the presented menu, and the
//! session is ended with `-[NSMenu cancelTracking]` so control returns to
//! the pump instead of waiting for a click.
//!
//! # Safety
//!
//! `NSMenu` has no binding on this target (the iOS SDK headers
//! `objc2-ui-kit` is generated from do not declare it). The presented menu
//! is the notification's object and is messaged with the signatures
//! `AppKit` declares — `-itemArray`, `-performActionForItemAtIndex:` and
//! `-cancelTracking` on the menu, `-isSeparatorItem` and `-title` on its
//! `NSMenuItem`s — on the main thread `AppKit` requires.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_foundation::{
    NSArray, NSNotification, NSNotificationCenter, NSObjectProtocol, NSRunLoop,
    NSRunLoopCommonModes, NSString,
};

use super::native_test::pump_main_until;

/// An `NSMenu` `AppKit` is tracking.
#[derive(Debug)]
pub struct PresentedMenu {
    menu: Retained<AnyObject>,
    state: Rc<Tracking>,
}

impl PresentedMenu {
    /// The titles of the menu's items, separators left out.
    #[must_use]
    pub fn titles(&self) -> Vec<String> {
        // SAFETY: `-itemArray` answers an `NSArray` of `NSMenuItem`s.
        let items: Retained<NSArray<AnyObject>> = unsafe { msg_send![&*self.menu, itemArray] };
        items
            .iter()
            .filter(|item| {
                // SAFETY: `-isSeparatorItem` answers `BOOL`.
                let separator: bool = unsafe { msg_send![&**item, isSeparatorItem] };
                !separator
            })
            .map(|item| {
                // SAFETY: `-title` answers a non-nil `NSString`.
                let title: Retained<NSString> = unsafe { msg_send![&*item, title] };
                title.to_string()
            })
            .collect()
    }

    /// Picks the item at `index` the way a click does: sends its action.
    pub fn perform_action_for_item(&self, index: usize) {
        let index = isize::try_from(index).expect("a menu item index fits `NSInteger`");
        // SAFETY: `-performActionForItemAtIndex:` takes an `NSInteger`
        // and answers `void`.
        let () = unsafe { msg_send![&*self.menu, performActionForItemAtIndex: index] };
    }

    /// Whether `AppKit` has ended the menu's tracking session.
    #[must_use]
    pub fn has_ended(&self) -> bool {
        self.state.ended.get()
    }

    fn cancel_tracking(&self) {
        // SAFETY: `-cancelTracking` takes no argument and answers `void`.
        let () = unsafe { msg_send![&*self.menu, cancelTracking] };
    }
}

/// What the observers share with the scheduled continuation.
struct Tracking {
    menu: RefCell<Option<Retained<AnyObject>>>,
    ended: Cell<bool>,
    finished: Cell<bool>,
    panic: RefCell<Option<Box<dyn Any + Send>>>,
}

impl std::fmt::Debug for Tracking {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Tracking")
            .field("ended", &self.ended.get())
            .field("finished", &self.finished.get())
            .finish_non_exhaustive()
    }
}

/// The registered observers; removing them and ending a still-running
/// tracking session on drop means a case that fails mid-menu leaves no
/// popup holding the shared process's main thread.
struct Observers {
    center: Retained<NSNotificationCenter>,
    tokens: [Retained<ProtocolObject<dyn NSObjectProtocol>>; 2],
    state: Rc<Tracking>,
}

impl Drop for Observers {
    fn drop(&mut self) {
        for token in &self.tokens {
            // SAFETY: the tokens are the objects `addObserverForName:` returned.
            unsafe { self.center.removeObserver(token.as_ref()) };
        }
        if !self.state.ended.get()
            && let Some(menu) = self.state.menu.borrow().as_ref()
        {
            // SAFETY: `-cancelTracking` takes no argument and answers `void`.
            let () = unsafe { msg_send![&**menu, cancelTracking] };
        }
    }
}

/// Runs `inside` within the tracking session of the menu `present` asks for.
///
/// `present` asks `AppKit` for a menu; the pump then runs until the menu's
/// tracking session begins and `inside` runs within it, against the
/// presented menu. The session is cancelled once `inside` returns, so this
/// returns instead of waiting for a click.
///
/// A panic in `inside` is carried out of the tracking session and resumed
/// here, on the caller's stack.
///
/// # Panics
///
/// When tracking does not begin within `seconds` of the pump starting, or
/// when `inside` panics.
pub fn within_presented_menu(
    seconds: f64,
    present: impl FnOnce(),
    inside: impl FnOnce(&PresentedMenu) + 'static,
) {
    let state = Rc::new(Tracking {
        menu: RefCell::new(None),
        ended: Cell::new(false),
        finished: Cell::new(false),
        panic: RefCell::new(None),
    });
    let inside = Rc::new(Cell::new(Some(inside)));
    let center = NSNotificationCenter::defaultCenter();
    let begin = {
        let state = state.clone();
        RcBlock::new(move |notification: NonNull<NSNotification>| {
            // SAFETY: the center hands the block a live notification.
            let notification = unsafe { notification.as_ref() };
            let menu = notification
                .object()
                .expect("a menu tracking notification names its menu");
            if state.menu.replace(Some(menu.clone())).is_some() {
                return;
            }
            let state = state.clone();
            let inside = inside.clone();
            let run = RcBlock::new(move || {
                let presented = PresentedMenu {
                    menu: menu.clone(),
                    state: state.clone(),
                };
                let inside = inside.take().expect("the scheduled continuation runs once");
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| inside(&presented))) {
                    state.panic.replace(Some(payload));
                }
                presented.cancel_tracking();
                state.finished.set(true);
            });
            // The tracking loop services the common run-loop modes,
            // event-tracking among them; the block runs inside the
            // session, not after it.
            // SAFETY: the modes array holds the common-modes constant, and
            // the block runs once on this (main) thread's run loop.
            unsafe {
                let modes = NSArray::from_slice(&[NSRunLoopCommonModes]);
                NSRunLoop::mainRunLoop().performInModes_block(&modes, &run);
            }
        })
    };
    let end = {
        let state = state.clone();
        RcBlock::new(move |_: NonNull<NSNotification>| state.ended.set(true))
    };
    let observe = |name: &str, block: &block2::DynBlock<dyn Fn(NonNull<NSNotification>)>| {
        // SAFETY: the block runs on the posting (main) thread, uses no
        // borrowed state and stays alive for the observer's lifetime.
        unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(&NSString::from_str(name)),
                None,
                None,
                block,
            )
        }
    };
    let observers = Observers {
        tokens: [
            observe("NSMenuDidBeginTrackingNotification", &begin),
            observe("NSMenuDidEndTrackingNotification", &end),
        ],
        center: center.clone(),
        state: state.clone(),
    };

    present();
    pump_main_until(seconds, || state.finished.get());
    drop(observers);
    if let Some(payload) = state.panic.take() {
        resume_unwind(payload);
    }
    assert!(
        state.menu.borrow().is_some(),
        "the presented menu's `NSMenuDidBeginTrackingNotification` never arrived within {seconds} s"
    );
    assert!(
        state.finished.get(),
        "the menu's tracking began but the continuation did not run within {seconds} s"
    );
}
