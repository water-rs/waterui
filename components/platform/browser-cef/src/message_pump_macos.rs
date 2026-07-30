use std::time::Duration;

use block2::RcBlock;
use objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFRetained, CFRunLoop, CFRunLoopTimer, kCFRunLoopCommonModes,
};

use crate::CefRuntime;

const TIMER_INTERVAL: Duration = Duration::from_millis(1000 / 30);

/// Drives CEF's external message pump from the main Core Foundation run loop.
///
/// CEF requires its browser-process loop to run on the application main
/// thread. A run-loop timer keeps that foreign event loop out of `WaterUI`'s
/// async task executor while remaining shared by native and self-drawn
/// renderers.
#[derive(Debug)]
pub struct CefMacOsMessagePump {
    timer: CFRetained<CFRunLoopTimer>,
}

impl CefMacOsMessagePump {
    /// Installs a CEF pump on the process main run loop.
    ///
    /// # Panics
    ///
    /// Panics when Core Foundation cannot provide the process main run loop or
    /// create its message-pump timer.
    #[must_use]
    pub fn install(runtime: CefRuntime) -> Self {
        let callback = RcBlock::new(move |_timer: *mut CFRunLoopTimer| {
            let _ = runtime.pump();
        });
        let interval = TIMER_INTERVAL.as_secs_f64();
        let timer = unsafe {
            CFRunLoopTimer::with_handler(
                None,
                CFAbsoluteTimeGetCurrent() + interval,
                interval,
                0,
                0,
                Some(&callback),
            )
        }
        .expect("failed to create the CEF Core Foundation message-pump timer");
        let run_loop = CFRunLoop::main().expect("the macOS process has no main run loop");
        run_loop.add_timer(Some(&timer), unsafe { kCFRunLoopCommonModes });
        Self { timer }
    }
}

impl Drop for CefMacOsMessagePump {
    fn drop(&mut self) {
        self.timer.invalidate();
    }
}
