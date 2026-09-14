//! Fixture for `artifact_symbols`: a real `include_web!` invocation whose
//! `waterui_meta_bundle_web` static the CLI must find in the built rlib.

/// Never called — the expansion only has to compile; evaluating it would go
/// looking for a staged bundle this test process does not have.
pub fn web_view() -> waterui::webview::WebViewOpen {
    waterui::include_web!("web")
}
