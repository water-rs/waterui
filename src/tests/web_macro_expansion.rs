//! Compile-level coverage for the `include_web!` expansion.
//!
//! The expansion spells out `waterui::webview::{WebView, DirectoryServer,
//! dev_url}` and `waterui::Bundle` paths; nothing else in the workspace invokes
//! the macro, so without this test a facade re-export moving out from under an
//! expansion breaks only downstream users. The produced value is never
//! evaluated — `DirectoryServer::new` resolves the staged bundle root, which a
//! test process does not have — so the function is only type-checked.
//!
//! The expansion installs a navigation policy, which builds on Apple only at a
//! deployment target of OS 27 or newer, above the workspace's floor. The paths
//! it checks are the same on every target, so it compiles where the gate does
//! not apply.
#![cfg(all(feature = "webview", feature = "assets", not(target_vendor = "apple")))]

/// `CARGO_MANIFEST_DIR` is the `waterui` package root (`src/`), so the fixture
/// web project is addressed as `tests/fixtures/web_project`.
fn web_view(resources: &waterui::ResourceContext) -> waterui::webview::WebViewOpen {
    waterui::include_web!(resources, "tests/fixtures/web_project")
}

#[test]
fn include_web_expands_to_a_webview_open() {
    let _: fn(&waterui::ResourceContext) -> waterui::webview::WebViewOpen = web_view;
}
