//! Run-loop support the `native` test harnesses share on both platforms,
//! and the stepped scroll clock a case installs on a surface.

use std::cell::Cell;
use std::time::Duration;

#[cfg(target_os = "macos")]
use objc2_core_foundation::CFString;
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};
#[cfg(target_os = "macos")]
use objc2_io_kit::{
    IOPMAssertionCreateWithName, IOPMAssertionDeclareUserActivity, IOPMAssertionID,
    IOPMAssertionRelease, IOPMUserActiveType, kIOPMAssertionLevelOn, kIOPMNullAssertionID,
    kIOReturnSuccess,
};

use crate::PlatformView;
use crate::scroll_flight::FlightClock;

/// Pumps the main run loop until `until` answers or `seconds` elapse.
///
/// A synchronous case awaits work enqueued on the main queue — a deferred
/// emission apply, an enqueued drop — in small turns rather than one
/// fixed wait. Answers whether `until` was reached; callers assert with
/// the condition's name so a dead queue fails the case instead of
/// hanging it.
pub fn pump_main_until(seconds: f64, until: impl Fn() -> bool) -> bool {
    let deadline = NSDate::dateWithTimeIntervalSinceNow(seconds);
    while !until() && deadline.timeIntervalSinceNow() > 0.0 {
        // SAFETY: `NSDefaultRunLoopMode` is a system-owned run-loop mode.
        NSRunLoop::currentRunLoop().runMode_beforeDate(
            unsafe { NSDefaultRunLoopMode },
            &NSDate::dateWithTimeIntervalSinceNow(0.02),
        );
    }
    until()
}

/// The display power assertion a harness holds for its whole run: the
/// suites assert on presented frames, and a sleeping display presents
/// none (#3529).
///
/// [`hold`](Self::hold) declares user activity — waking a sleeping
/// display — then takes a `PreventUserIdleDisplaySleep` assertion, which
/// the guard releases on drop at the harness's end.
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct DisplayAwake {
    /// The user-activity declaration that woke the display.
    user_activity: IOPMAssertionID,
    /// The `PreventUserIdleDisplaySleep` assertion keeping it awake.
    prevent_display_sleep: IOPMAssertionID,
}

#[cfg(target_os = "macos")]
impl DisplayAwake {
    /// Wakes the host display and takes the assertion that keeps it on.
    ///
    /// # Panics
    ///
    /// If `IOKit` refuses either call — a suite that cannot hold the
    /// display up fails at harness start rather than running its cases
    /// blind.
    #[must_use]
    pub fn hold() -> Self {
        let name = CFString::from_static_str("waterui native tests");
        let mut user_activity = kIOPMNullAssertionID;
        // SAFETY: `user_activity` is a valid out-pointer for the call's
        // assertion id, and `name` is a real `CFString`.
        let declared = unsafe {
            IOPMAssertionDeclareUserActivity(
                Some(&name),
                IOPMUserActiveType::Local,
                &raw mut user_activity,
            )
        };
        assert_eq!(
            declared, kIOReturnSuccess,
            "IOPMAssertionDeclareUserActivity failed with IOReturn {declared:#x} — \
             the harness cannot wake a sleeping display"
        );
        let prevent_display_sleep_type = CFString::from_static_str("PreventUserIdleDisplaySleep");
        let mut prevent_display_sleep = kIOPMNullAssertionID;
        // SAFETY: `prevent_display_sleep` is a valid out-pointer for the
        // call's assertion id, and the type and name are real `CFString`s.
        let created = unsafe {
            IOPMAssertionCreateWithName(
                Some(&prevent_display_sleep_type),
                kIOPMAssertionLevelOn,
                Some(&name),
                &raw mut prevent_display_sleep,
            )
        };
        assert_eq!(
            created, kIOReturnSuccess,
            "IOPMAssertionCreateWithName(PreventUserIdleDisplaySleep) failed with IOReturn \
             {created:#x} — the harness cannot keep the display awake"
        );
        Self {
            user_activity,
            prevent_display_sleep,
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for DisplayAwake {
    fn drop(&mut self) {
        IOPMAssertionRelease(self.prevent_display_sleep);
        IOPMAssertionRelease(self.user_activity);
    }
}

/// A scroll flight's clock under the test's hand.
///
/// Time stands still until [`advance`](Self::advance) moves it, and
/// each advance delivers exactly one frame while the flight's clock is
/// running. Installed per surface through its `manual_scroll_clock()`;
/// no display link drives that surface's flights afterwards.
pub struct ManualScrollClock {
    /// Seconds on this clock's timeline.
    now: Cell<f64>,
    /// Whether the flight started the clock and has not stopped it.
    running: Cell<bool>,
    /// The flight's frame callback.
    on_frame: Box<dyn Fn()>,
}

impl ManualScrollClock {
    pub(crate) fn new(on_frame: impl Fn() + 'static) -> Self {
        Self {
            now: Cell::new(0.0),
            running: Cell::new(false),
            on_frame: Box::new(on_frame),
        }
    }

    /// Moves time forward by `by`, then delivers one frame if the flight's
    /// clock is running — the frame a display link would present at the
    /// new instant.
    pub fn advance(&self, by: Duration) {
        self.now.set(self.now.get() + by.as_secs_f64());
        if self.running.get() {
            (self.on_frame)();
        }
    }

    /// Whether the flight has the clock running — `false` once a
    /// takeover cancelled it or the flight landed.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.running.get()
    }
}

impl FlightClock for ManualScrollClock {
    fn begin_time(&self) -> f64 {
        self.now.get()
    }
    fn tick_time(&self) -> f64 {
        self.now.get()
    }
    fn start(&self, view: &PlatformView) {
        self.running.set(crate::view::window(view).is_some());
    }
    fn stop(&self) {
        self.running.set(false);
    }
    fn is_running(&self) -> bool {
        self.running.get()
    }
}

impl std::fmt::Debug for ManualScrollClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManualScrollClock")
            .field("now", &self.now)
            .field("running", &self.running)
            .finish_non_exhaustive()
    }
}
