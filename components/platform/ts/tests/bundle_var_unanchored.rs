//! The other half of the path rule for `WATERUI_TS_BUNDLE`: a relative path
//! with no `CARGO_MANIFEST_DIR` to resolve it against is an error, never a
//! path taken relative to the working directory.
//!
//! Cargo runs a test binary with the package directory as its working
//! directory, so `bundle_var_relative.rs` alone cannot tell the rule from a
//! fallback to the working directory; this target removes the anchor and
//! asserts the refusal. One target for one process environment, under the
//! rule `support/bundle.rs` documents.

use std::sync::Once;

use waterui::ts::{BUNDLE_VARIABLE, Components, configured_runtime};
use waterui_core::Environment;

/// The bundle, as a path relative to the crate — which the working directory
/// would also resolve, if the rule fell back to it.
const RELATIVE: &str = "tests/fixtures/mount.js";

/// Points the variable at [`RELATIVE`] and removes the anchor, once per
/// process.
fn configure() {
    static CONFIGURED: Once = Once::new();
    CONFIGURED.call_once(|| {
        // SAFETY: inside the `Once`, so no other test in this process is past
        // `call_once` and nothing reads the environment while it is written.
        unsafe {
            std::env::set_var(BUNDLE_VARIABLE, RELATIVE);
            std::env::remove_var("CARGO_MANIFEST_DIR");
        }
    });
}

#[test]
fn ts_a_relative_bundle_path_with_no_manifest_directory_is_refused() {
    configure();
    let error = configured_runtime(&Environment::new(), Components)
        .expect_err("a relative path has nothing to resolve against");
    let message = error.to_string();
    assert!(
        message.contains(BUNDLE_VARIABLE) && message.contains(RELATIVE),
        "the message names the variable and the path as written: {message}"
    );
    assert!(
        message.contains("CARGO_MANIFEST_DIR"),
        "the message names the anchor that is missing: {message}"
    );
}
