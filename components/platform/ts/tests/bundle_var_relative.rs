//! The path rule for `WATERUI_TS_BUNDLE`: a relative path is resolved against
//! `CARGO_MANIFEST_DIR`, which cargo and nextest set for every test binary
//! they run.
//!
//! The checked-in `fixtures/mount.js` is named relative to this crate's
//! manifest directory, and the runtime that comes back carries the module it
//! publishes. One target for one process environment, under the rule
//! `support/bundle.rs` documents.

use std::sync::Once;

use waterui::ts::{BUNDLE_VARIABLE, Components, configured_runtime};
use waterui_core::Environment;

/// The bundle, as a path relative to the crate.
const RELATIVE: &str = "tests/fixtures/mount.js";

/// Points the variable at [`RELATIVE`], once per process.
fn configure() {
    static CONFIGURED: Once = Once::new();
    CONFIGURED.call_once(|| {
        // SAFETY: inside the `Once`, so no other test in this process is past
        // `call_once` and nothing reads the environment while it is written.
        unsafe { std::env::set_var(BUNDLE_VARIABLE, RELATIVE) };
    });
}

#[test]
fn ts_a_relative_bundle_path_is_resolved_against_the_manifest_directory() {
    configure();
    assert!(
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
        "cargo and nextest set CARGO_MANIFEST_DIR for a test binary, which is what the rule \
         resolves against"
    );
    let handle = configured_runtime(&Environment::new(), Components)
        .unwrap_or_else(|error| panic!("loading the bundle by a relative path: {error}"))
        .expect("the variable is set, so there is a runtime");
    handle
        .runtime()
        .module("tests/fixtures/promo.tsx")
        .expect("the bundle the relative path resolved to is the one that carries the module");
}
