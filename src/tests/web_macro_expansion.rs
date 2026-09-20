//! Compile-level coverage for the `include_web!` expansion.
//!
//! The expansion spells out `waterui::webview::{WebView, DirectoryServer,
//! dev_url}` and `waterui::Bundle` paths; nothing else in the workspace invokes
//! the macro, so without this test a facade re-export moving out from under an
//! expansion breaks only downstream users. The produced value is never
//! evaluated — `DirectoryServer::new` resolves the staged bundle root, which a
//! test process does not have — so the function is only type-checked.
#![cfg(all(feature = "webview", feature = "assets"))]

/// `CARGO_MANIFEST_DIR` is the `waterui` package root (`src/`), so the fixture
/// web project is addressed as `tests/fixtures/web_project`.
fn web_view() -> waterui::webview::WebViewOpen {
    waterui::include_web!("tests/fixtures/web_project")
}

#[test]
fn include_web_expands_to_a_webview_open() {
    let _: fn() -> waterui::webview::WebViewOpen = web_view;
}
