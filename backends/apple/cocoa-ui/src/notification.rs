//! Observing notifications posted to the default notification center.
//!
//! # Safety
//!
//! The `unsafe` here registers and removes a block observer. The block is an
//! `RcBlock` the center copies and keeps until the observer is removed, so it
//! outlives every call; the observer token is the object the center returned
//! for exactly that registration.

use std::ptr::NonNull;

use block2::RcBlock;
use dispatch2::MainThreadBound;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, Message};
use objc2_foundation::{
    NSCurrentLocaleDidChangeNotification, NSNotification, NSNotificationCenter, NSOperationQueue,
    NSString,
};

use crate::callback::guarded;

/// The name a notification is posted under.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NotificationName(Retained<NSString>);

impl NotificationName {
    /// A notification name spelled out, for notifications the crate does not
    /// name itself.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self(NSString::from_str(name))
    }

    /// Posted when the user's locale settings change: the preferred languages,
    /// the region, or the formats that derive from them.
    #[must_use]
    pub fn current_locale_did_change() -> Self {
        // SAFETY: reads a string constant Foundation owns for the process's
        // lifetime.
        let name: &'static NSString = unsafe { NSCurrentLocaleDidChangeNotification };
        Self(name.retain())
    }

    /// A notification name a framework defines: one of the `NSString`
    /// constants `AppKit`/`UIKit` export, retained here for registration.
    #[must_use]
    pub fn framework(name: &'static NSString) -> Self {
        Self(name.retain())
    }
}

/// Keeps a notification observer registered; dropping it removes the
/// observer.
#[derive(Debug)]
#[must_use = "the observer is removed as soon as this guard is dropped"]
pub struct NotificationObserver {
    center: Retained<NSNotificationCenter>,
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl Drop for NotificationObserver {
    fn drop(&mut self) {
        // SAFETY: `token` is the observer the center returned when this
        // registration was made; see the module safety note.
        unsafe {
            self.center
                .removeObserver(AsRef::<AnyObject>::as_ref(&*self.token));
        };
    }
}

/// Calls `handler` on the main thread every time a notification named `name`
/// is posted, from any thread, until the returned guard is dropped.
///
/// A panic in `handler` aborts the process (see the
/// [crate documentation](crate)).
///
/// # Panics
///
/// If the main operation queue delivers the notification off the main
/// thread, which it must never do.
pub fn observe(
    mtm: MainThreadMarker,
    name: &NotificationName,
    handler: impl Fn() + 'static,
) -> NotificationObserver {
    observe_impl(mtm, name, None, handler)
}

/// Like [`observe`], but only for notifications whose object is `object` —
/// the center filters delivery to notifications posted on that instance.
pub fn observe_object(
    mtm: MainThreadMarker,
    name: &NotificationName,
    object: &AnyObject,
    handler: impl Fn() + 'static,
) -> NotificationObserver {
    observe_impl(mtm, name, Some(object), handler)
}

/// The registration [`observe`] and [`observe_object`] share.
fn observe_impl(
    mtm: MainThreadMarker,
    name: &NotificationName,
    object: Option<&AnyObject>,
    handler: impl Fn() + 'static,
) -> NotificationObserver {
    // The center may release the block on the thread that posted the last
    // notification it delivered, so the handler is bound to the main thread
    // and dropped there.
    let handler = MainThreadBound::new(handler, mtm);
    let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        guarded("notification observer", || {
            let mtm = MainThreadMarker::new()
                .expect("the main operation queue must deliver notifications on the main thread");
            (handler.get(mtm))();
        });
    });
    let center = NSNotificationCenter::defaultCenter();
    let queue = NSOperationQueue::mainQueue();
    // SAFETY: see the module safety note.
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(&name.0),
            object,
            Some(&queue),
            &block,
        )
    };
    NotificationObserver { center, token }
}
