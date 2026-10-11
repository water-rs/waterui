//! What a `tsx!` mount does when nothing set `WATERUI_TS_BUNDLE`: the case of
//! a `#[waterui::test]` run under bare `cargo nextest` rather than
//! `water test`.
//!
//! One target for one process environment: the variable is removed once per
//! process, before any test in this binary runs anything else, under the same
//! rule `support/bundle.rs` documents for setting it.

use std::cell::RefCell;
use std::sync::Once;

use waterui::ts::BUNDLE_VARIABLE;
use waterui::tsx;
use waterui_core::{AnyView, Environment, View};
use waterui_preview::with_configured_runtime;
use waterui_testing::UiBuilder;

/// Removes the variable once per process.
fn unset() {
    static UNSET: Once = Once::new();
    UNSET.call_once(|| {
        // SAFETY: inside the `Once`, so no other test in this process is past
        // `call_once` and nothing reads the environment while it is written.
        unsafe { std::env::remove_var(BUNDLE_VARIABLE) };
    });
}

/// Mounts `view` in a session, which is expected to panic, and hands back the
/// panic's message.
fn mount_failure(ui: UiBuilder, view: impl View + 'static) -> String {
    let view = RefCell::new(Some(AnyView::new(view)));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _app = ui.mount(move || {
            view.borrow_mut()
                .take()
                .expect("the test session realizes its root once")
        });
    }));
    let payload = outcome.expect_err("mounting a tsx! view with no bundle configured fails");
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
        })
        .expect("the panic carries a message")
}

/// The message a mount without a runtime fails with, as the user sees it:
/// the module, the variable, the command that sets it, and the command they
/// ran instead.
fn assert_names_the_way_out(message: &str) {
    assert!(
        message.contains("tests/fixtures/promo.tsx"),
        "the message names the module: {message}"
    );
    assert!(
        message.contains(BUNDLE_VARIABLE),
        "the message names the variable: {message}"
    );
    assert!(
        message.contains("`water test`"),
        "the message names the command that sets it: {message}"
    );
    assert!(
        message.contains("`cargo nextest`"),
        "the message names the command that does not: {message}"
    );
}

#[waterui::test(viewport = (320, 240))]
fn ts_a_test_session_without_the_variable_fails_at_the_mount(ui: UiBuilder) {
    unset();
    // A view that mounts no TypeScript needs no bundle: the session mounts it
    // with no runtime and no complaint.
    let mut plain = ui
        .clone()
        .mount(|| waterui::text::text("No TypeScript here"));
    plain.query().label("No TypeScript here").assert_exists();
    drop(plain);

    // One that does reaches `Mount::body`, which panics with the message.
    assert_names_the_way_out(&mount_failure(ui, tsx!("fixtures/promo.tsx")));
}

#[waterui::test(viewport = (320, 240))]
fn ts_a_preview_render_without_the_variable_installs_nothing(ui: UiBuilder) {
    unset();
    // With nothing to load, the preview is not an error: it hands the view
    // through untouched, and the mount inside it fails exactly as a test
    // session's does.
    let view = with_configured_runtime(
        &Environment::new(),
        AnyView::new(tsx!("fixtures/promo.tsx")),
    )
    .expect("an unset variable is not an error");
    assert_names_the_way_out(&mount_failure(ui, view));
}
