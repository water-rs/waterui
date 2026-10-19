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

// On Mac Catalyst the binary is spawned without a bundle: its embedded
// `Info.plist` declares `LSUIElement`, so the process arrives at the
// `Prohibited` activation policy and `UIKit`'s Mac backgrounding
// controller leaves the cases' scene-less windows alone. The iOS targets
// spawn with no `Info.plist` and keep it that way, so the section is
// emitted on Catalyst only.
#[cfg(all(target_os = "ios", target_abi = "macabi"))]
#[used]
#[unsafe(link_section = "__TEXT,__info_plist")]
static NATIVE_SPAWN_INFO_PLIST: [u8; include_bytes!("native.plist").len()] =
    *include_bytes!("native.plist");

fn main() {
    let mut arguments = Arguments::from_args();
    // The main thread is the point of this harness: with a single runner
    // `libtest-mimic` executes every trial on the calling thread — `main`'s
    // — where `pthread_main` answers true.
    arguments.test_threads = Some(1);
    // A Catalyst spawn has no `UIApplication`, no scene and no
    // `NSApplication`. A scene-less `UIWindow`'s unassociated scene reads
    // the `AppKit` delegate the moment the window is created, and its
    // `UINSApplicationDelegate` raises when no `NSApplication` exists —
    // so the process's `AppKit` side comes up at the `Prohibited` policy
    // before any case mounts a window. `--list` mounts nothing and skips
    // it.
    #[cfg(all(target_os = "ios", target_abi = "macabi"))]
    let _background_application = (!arguments.list).then(|| {
        cocoa_ui::uikit::catalyst::BackgroundApplication::bring_up(harness::marker())
    });
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
