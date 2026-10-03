//! Shared scaffolding for the native cases: the real main-thread marker.

use cocoa_ui::MainThreadMarker;
use libtest_mimic::Trial;

/// The suite's trial list, prefixed with the check the whole harness
/// depends on: the marker is real.
pub fn trials(mut cases: Vec<Trial>) -> Vec<Trial> {
    cases.insert(
        0,
        Trial::test("the_main_thread_marker_is_real", || {
            assert!(
                cocoa_ui::MainThreadMarker::new().is_some(),
                "a native case ran off the process's main thread"
            );
            Ok(())
        }),
    );
    cases
}

/// The marker a case runs under — `MainThreadMarker::new()` on the thread
/// the case executes on, so a case that drifted off the main thread fails
/// here instead of being handed a forged token.
///
/// # Panics
///
/// If the calling thread is not the process's main thread.
pub fn marker() -> MainThreadMarker {
    MainThreadMarker::new().expect("the native suite runs every case on the process's main thread")
}
