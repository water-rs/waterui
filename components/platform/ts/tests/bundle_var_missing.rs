//! `WATERUI_TS_BUNDLE` set to a file that is not there.
//!
//! The variable being set is a request to load that bundle, so a host that
//! finds nothing at the path fails there, naming the variable and the path,
//! rather than mounting with no runtime and failing later about a runtime.
//! One target for one process environment, under the rule
//! `support/bundle.rs` documents.

use std::path::PathBuf;
use std::sync::Once;

use waterui::ts::{BUNDLE_VARIABLE, Components, configured_runtime};
use waterui::tsx;
use waterui_core::{AnyView, Environment};
use waterui_preview::with_configured_runtime;
use waterui_testing::UiBuilder;

/// Where the variable points: a path nothing writes to.
fn missing_path() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("waterui-ts-hosts/does-not-exist.js")
}

/// Points the variable at [`missing_path`], once per process.
fn configure() {
    static CONFIGURED: Once = Once::new();
    CONFIGURED.call_once(|| {
        // SAFETY: inside the `Once`, so no other test in this process is past
        // `call_once` and nothing reads the environment while it is written.
        unsafe { std::env::set_var(BUNDLE_VARIABLE, missing_path()) };
    });
}

/// The message names the variable and the path, so the reader knows which
/// setting to fix and what it currently says.
fn assert_names_the_file(message: &str) {
    assert!(
        message.contains(BUNDLE_VARIABLE),
        "the message names the variable: {message}"
    );
    assert!(
        message.contains("does-not-exist.js"),
        "the message names the path: {message}"
    );
    assert!(
        message.contains("could not be read"),
        "the message says the file could not be read: {message}"
    );
}

#[test]
fn ts_a_missing_bundle_is_an_error_naming_the_variable_and_the_path() {
    configure();
    let error = configured_runtime(&Environment::new(), Components)
        .expect_err("the file the variable names does not exist");
    assert_names_the_file(&error.to_string());
}

#[waterui::test(viewport = (320, 240))]
fn ts_a_test_session_fails_before_mounting_anything(ui: UiBuilder) {
    configure();
    // Even a view that mounts no TypeScript: the session was told to load a
    // bundle and could not, and a test that went on would be running against
    // a configuration that is not what its author set.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _app = ui.mount(|| waterui::text::text("No TypeScript here"));
    }));
    let payload = outcome.expect_err("a bundle that cannot be read fails the session");
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
        })
        .expect("the panic carries a message");
    assert_names_the_file(&message);
}

#[test]
fn ts_a_preview_render_fails_before_rendering_anything() {
    configure();
    let error = with_configured_runtime(
        &Environment::new(),
        AnyView::new(tsx!("fixtures/promo.tsx")),
    )
    .expect_err("a bundle that cannot be read fails the render");
    assert_names_the_file(&error.to_string());
}
