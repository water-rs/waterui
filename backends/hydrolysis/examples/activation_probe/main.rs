//! Activation-policy probe for water-rs/waterui#1302.
//!
//! The window starts [`WindowState::Closed`]; `StayResident` keeps the
//! process alive with zero windows so another app (`TextEdit` in the test
//! harness) can be frontmost when the probe finally shows. A timer flips
//! the presentation state to `Normal` — the same show path a drop-down
//! terminal's toggle uses — and every re-show mounts a fresh window, so
//! selector resolution and the activation policy run each time. A
//! `TextField` echoes keystrokes so an observer can tell where key events
//! land.
//!
//! Environment:
//!   `PROBE_ACTIVATION`   onshow | onclick | never   (default onshow)
//!   `PROBE_SELECTOR`     primary | pointer | focused (default primary)
//!   `PROBE_SHOW_AFTER`   seconds until the window opens (default 2.5)
//!   `PROBE_START_OPEN`   open the window at launch (initial state Normal
//!                      instead of Closed) — exercises launch activation
//!                      with a window mounted at startup
//!   `PROBE_CYCLE`        reopen the window every N seconds after the first
//!                      show instead of showing once (default unset)
//!   `PROBE_EXIT_AFTER`   self-exit after N seconds (default 3600)
//!
//! Supported targets: those with `hydrolysis::run`, which excludes Android
//! (the Kotlin host owns the Activity there) and wasm32 without `web`.

#[cfg(hydrolysis_run)]
mod windowed;

#[cfg(hydrolysis_run)]
fn main() {
    windowed::main();
}

#[cfg(not(hydrolysis_run))]
fn main() {
    panic!(
        "the `activation_probe` example needs `hydrolysis::run`, which this target does not have"
    );
}
