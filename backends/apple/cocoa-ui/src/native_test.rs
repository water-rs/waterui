//! Run-loop support the `native` test harnesses share on both platforms.

use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};

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
