//! Run-loop support the `native` test harnesses share on both platforms,
//! and the stepped scroll clock a case installs on a surface.

use std::cell::Cell;
use std::time::Duration;

use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};

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
