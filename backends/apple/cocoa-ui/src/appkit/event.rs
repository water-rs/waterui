//! Local event monitors: a handler that sees events without consuming them.
//!
//! # Safety
//!
//! The `unsafe` here registers and removes an `NSEvent` local monitor. The
//! handler block is an `RcBlock` `AppKit` copies and keeps until the monitor
//! is removed, and removal happens on the main thread inside this module's
//! `Drop`. `NSEvent` delivers local monitors on the main thread of the
//! application that owns the event.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSWindow};

use crate::callback::guarded;

/// Keeps a local event monitor installed; dropping it removes the monitor.
#[derive(Debug)]
#[must_use = "the monitor is removed as soon as this guard is dropped"]
pub struct LocalEventMonitor {
    token: Retained<AnyObject>,
}

impl Drop for LocalEventMonitor {
    fn drop(&mut self) {
        // SAFETY: `token` is the monitor object `addLocalMonitor…` returned
        // for exactly this registration; see the module safety note.
        unsafe { NSEvent::removeMonitor(&self.token) };
    }
}

/// Calls `handler` on every mouse-down — left, right or other button —
/// delivered while the returned guard is alive, and always lets the event
/// continue to its target.
///
/// `handler` receives the window the event was delivered to, or `None` for
/// an event with no window; an outside-interaction check compares it
/// against the surface it should ignore.
///
/// A panic in `handler` aborts the process (see the
/// [crate documentation](crate)).
///
/// # Panics
///
/// If `AppKit` ever declines the monitor registration, which it is not
/// known to do.
pub fn on_mouse_down(handler: impl Fn(Option<&NSWindow>) + 'static) -> LocalEventMonitor {
    let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: `AppKit` hands the block a live `NSEvent` it keeps owning;
        // the returned pointer gives the same event back unconsumed.
        let window = unsafe {
            event
                .as_ref()
                .window(objc2::MainThreadMarker::new_unchecked())
        };
        guarded("local mouse-down monitor", || {
            handler(window.as_deref());
        });
        event.as_ptr()
    });
    let mask =
        NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown;
    // SAFETY: see the module safety note.
    let token = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
    LocalEventMonitor {
        token: token.expect("AppKit always installs a local event monitor"),
    }
}
