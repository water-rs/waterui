//! The native suite: real `AppKit`/`UIKit` objects exercised through the
//! crate's public API on the process's real main thread.
//!
//! `Cargo.toml` declares this target `harness = false`, so `main` here is
//! the process entry point and therefore the true main thread, and the suite
//! pins `--test-threads 1` so `libtest-mimic` runs every trial on this
//! thread rather than on a worker pool. `MainThreadMarker::new()` inside a
//! case is then a real assertion — no forged marker, no global lock, no
//! thread hopping. Under nextest each case additionally runs as its own
//! process, which is the isolation `AppKit`/`UIKit` global state wants;
//! under `cargo test` the same cases run one after another on this thread.

#[cfg(target_os = "macos")]
mod appkit;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod harness;
#[cfg(target_os = "ios")]
mod uikit;

use libtest_mimic::{Arguments, Trial};

fn main() {
    let mut arguments = Arguments::from_args();
    // The main thread is the point of this harness: with a single runner
    // `libtest-mimic` executes every trial on the calling thread — `main`'s
    // — where `pthread_main` answers true.
    arguments.test_threads = Some(1);
    libtest_mimic::run(&arguments, trials()).exit();
}

fn trials() -> Vec<Trial> {
    #[cfg(target_os = "macos")]
    {
        appkit::trials()
    }
    #[cfg(target_os = "ios")]
    {
        uikit::trials()
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        Vec::new()
    }
}
